//! The reader's window: translates input into [`code_core`] commands and paints the session's state through
//! the shared single-pane presentation. It owns pixel geometry only (font, horizontal scroll, modal rows);
//! every semantic change goes through `Session::dispatch`.
#![deny(unsafe_code)]

use std::{path::PathBuf, sync::Arc};

use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, FocusHandle, Font, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Pixels, Render, ScrollDelta, ScrollEvent, Styled, TitlebarOptions, Window, WindowOptions,
    canvas, div, px, rgb,
};
use gpui::InteractiveElement;
use lenz_content::find::FindOption;
use lenz_content::scrollbar::{self, Hit, Mark, MarkKind, Scrollbar};
use code_core::markers::{match_colors, scrollbar_mark_color, scrollbar_thumb_color};
use lenz_content::{display, theme::{ThemeCommand, ThemeSelection}};
use lenz_gpui::{Metrics, TextRow};
use code_core::{
    actions::{CatalogAction, CommandGroup, InterfaceAction},
    markdown::Preview,
    search::{Fold, Group, Row},
    semantics::hierarchy::Direction,
    chrome::MenuTarget,
    session::{Command, CommandId, Completion, Config, Layout, MenuEntry, Motion, PlaceList, Rendering, SearchIn, Selection, Session, Split, StartItem},
    tabs::{Tab, TabId},
};

pub mod icons;
mod preview;
mod completion;
mod csv;
mod navigation;
mod search;
mod dialog;
mod activity;
mod terminal;
mod source;
mod inline_diagnostics;
mod close;
mod input_method;
mod external;
mod save_as;
mod recovery;
mod reconcile;

static FONTS: std::sync::OnceLock<lenz_content::appearance::FontPreferences> = std::sync::OnceLock::new();
fn fonts() -> &'static lenz_content::appearance::FontPreferences {
    static DEFAULT: std::sync::OnceLock<lenz_content::appearance::FontPreferences> = std::sync::OnceLock::new();
    FONTS.get().unwrap_or_else(|| DEFAULT.get_or_init(Default::default))
}
fn font_size() -> f32 { fonts().font_size }
fn line_h() -> f32 { 20.0 * font_size() / 13.0 }
/// The gutter's fold-mark column, right of the line numbers.
fn fold_w() -> f32 { 12.0 * font_size() / 13.0 }
fn num_w() -> f32 { 56.0 * font_size() / 13.0 + fold_w() }
/// A scrollbar's shortest thumb, and its track's mark cells: one per 3 px.
const THUMB_MIN: f32 = 16.0;
fn track_cells(len: f32) -> usize { (len / 3.0).floor().max(0.0) as usize }
/// Drawn after a collapsed region's first line (then its closing line's text, when that line only closes).
const PLACEHOLDER: &str = " … ";
/// The status bar along the window's bottom: one row, never taller.
fn status_h() -> f32 { 24.0 * font_size() / 13.0 }
fn toolbar_h() -> f32 { 28.0 * font_size() / 13.0 }
/// The status bar's text inset from each side.
const STATUS_PAD: f32 = 8.0;
/// The query and status rows above a list modal's results in [`ReaderView::modal_rows`].
const FINDER_HEADER_ROWS: usize = 2;
const THEME_HEADER_ROWS: usize = 2;
/// The AZERTY digit row's keys for 0–9 without Shift, as GPUI on Linux names them (the accented ones by keysym name).
const AZERTY_DIGITS: [&str; 10] = ["agrave", "&", "eacute", "\"", "'", "(", "-", "egrave", "_", "ccedilla"];

/// About's logo, decoded when About opens; its texture leaves the atlas once About is closed.
#[derive(Clone)]
pub struct AboutLogo(pub Arc<gpui::RenderImage>);

impl std::fmt::Debug for AboutLogo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AboutLogo")
    }
}

impl PartialEq for AboutLogo {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

/// A list over the text that takes all input until dismissed.
#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    /// The command menu: catalog rows and the highlighted one.
    Commands { at: usize },
    /// The app menu and its shared command groups.
    AppMenu { at: usize },
    CommandGroup { group: CommandGroup, at: usize },
    /// Shared server management actions, with diagnostics in Server Details.
    LanguageServer { at: usize },
    /// A frozen, read-only diagnostic snapshot (or About, with its logo); Copy preserves the complete text.
    /// `kind`: plain text (Copy / Close), or one of the core's dialogs whose text and buttons follow the core.
    StatusDetails { title: &'static str, text: String, logo: Option<AboutLogo>, kind: Details },
    /// The theme chooser: "system" then each theme ID, and the highlighted one.
    Themes { ids: Vec<(String, String)>, at: usize },
    /// Recent folders, most recent first.
    Recent { at: usize },
    ScratchFiles { files: Vec<PathBuf>, at: usize },
    /// An in-window destination prompt; the originating tab stays captured until acceptance.
    SaveAs { tab: code_core::tabs::TabId, dir: PathBuf, input: String, selected: bool, error: Option<String> },
    NewScratch { input: String, selected: bool, error: Option<String> },
    RenameTerminal { id: code_core::terminal::Id, input: String, selected: bool, error: Option<String> },
    /// Unified file, symbol and command search; result targets stay in the core.
    Navigation { at: usize },
    /// The search box: the text typed so far, and whether it searches the project (else the active file); for the
    /// project also the file pattern, and whether keys go to it (Tab switches).
    /// `selected`: the focused field's text is selected, so typing replaces it (a reopened search's old query).
    Search { query: String, files: String, in_files: bool, project: bool, selected: bool },
    /// Declaration search: the name typed so far; results update in the panel as it is typed. `semantic` asks
    /// the language servers for workspace symbols instead of the syntactic index (Tab switches).
    Declarations { query: String, semantic: bool },
    /// The workspace root for the active file's language server: the path typed so far (empty restores the
    /// server's own discovery).
    Root { path: String },
    SqlMetadata { path: String },
    ServerRuntime { path: String, at: usize },
    /// The text's context menu, opened at a point: its entries and the highlighted one.
    Context { entries: Vec<MenuEntry>, at: usize, x: f32, y: f32, scroll: f32 },
    /// Go to line: what is typed so far (`line` or `line:column`), and why the last Enter was refused.
    GoTo { input: String, error: Option<String> },
    /// What is inside breadcrumb `crumb` of pane `pane`: the filter typed so far and the highlighted entry.
    Siblings { pane: u8, crumb: usize, query: String, at: usize },
    /// A list of places (bookmarks, recent files, recent locations): the filter typed so far and the highlighted row.
    Places { list: PlaceList, query: String, at: usize },
    /// The bookmark key chooser: the next 0–9 / A–Z key sets it on the caret's line.
    Mnemonic,
    /// The Git Branches popup (the core's lists): the filter typed so far and the highlighted row.
    Branches { query: String, at: usize },
    /// A popup row's actions: the row, its label, the highlighted action, and the popup's filter and row to go back to.
    BranchMenu { target: code_core::branches::BranchTarget, label: String, at: usize, query: String, row: usize },
    /// A new branch: where it starts (`None`: HEAD), its name so far, whether to check it out, and why it was refused.
    NewBranch { from: Option<String>, name: String, checkout: bool, error: Option<String> },
    /// Checkout Tag or Revision: what is typed so far, and why it was refused.
    /// Checkout Tag or Revision….
    Revision { input: String, error: Option<String> },
    /// New File / New Folder: the name typed so far in the explorer's folder (`dir`, project-relative), and why it
    /// was refused.
    Create { kind: code_core::create::Kind, dir: String, name: String, error: Option<String> },
    PasteEntry { request: code_core::session::PasteEntry, name: String, selected: bool, error: Option<String> },
    /// Rename or Move: the new name or folder for explorer entry `from` (project-relative).
    Relocate { kind: code_core::relocate::Kind, from: String, input: String, error: Option<String> },
    /// Delete: confirm removing explorer entry `from` (project-relative), into the trash or permanently.
    ConfirmDelete { removal: code_core::trash::Removal, from: String, error: Option<String> },
    /// The keymap: its rows' highlight, and whether the next key becomes a shortcut for the highlighted command.
    Keymap { at: usize, capture: bool },
}

/// What a details dialog is: plain text, the core's commit popup, or its checkout-problem dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Details {
    Install,
    Terminal { ticket: u64 },
    Text,
    Sql,
    Change,
    Commit,
    Checkout,
    Close,
    External,
    /// A dirty draft and a file that changed on disk: compare them, take disk, save over it or keep the draft.
    Reconcile,
    Replace,
    Recovered,
}

/// A drawn breadcrumb: its bounds, pane, crumb (`None`: a shrunk "…"), whether it is the `›` after the crumb, and its
/// text.
#[derive(Clone, Debug)]
pub struct CrumbHit {
    pub bounds: Bounds<Pixels>,
    pub pane: u8,
    pub crumb: Option<usize>,
    pub chevron: bool,
    pub label: String,
}

type StatusCell = (Bounds<Pixels>, String);

/// A list modal as drawn and clicked (context menus and navigation have their own layout): its box, its lines
/// (the [`ReaderView::modal_rows`] row each belongs to, text, disabled), the first line shown, how many show, and the
/// highlighted row.
struct ModalLayout {
    bounds: Bounds<Pixels>,
    lines: Vec<(usize, String, bool)>,
    first: usize,
    visible: usize,
    at: usize,
    footer: Vec<String>,
    heading: Option<String>,
    query: Option<String>,
    body_top: f32,
    shortcuts: Vec<(usize, String)>,
}

impl ModalLayout {
    /// Fit buttons in a grid while reserving a readable body row, even for long reconciliation labels.
    fn button_grid(width: f32, char_w: f32, labels: &[&str], available_h: f32) -> (Vec<Bounds<Pixels>>, f32) {
        if labels.is_empty() { return (Vec::new(), 0.0); }
        let pad = 12.0_f32.min(width / 4.0);
        let gap = 8.0;
        let bh = line_h() + 12.0;
        let room = (width - 2.0 * pad).max(0.0);
        let desired = labels.iter().map(|l| lenz_content::wrap::width(l) as f32 * char_w + 24.0).fold(0.0_f32, f32::max);
        let natural_cols = ((room + gap) / (desired + gap)).floor().max(1.0) as usize;
        let max_rows = ((available_h + gap) / (bh + gap)).floor().max(1.0) as usize;
        let cols = natural_cols.max(labels.len().div_ceil(max_rows)).min(labels.len());
        let rows = labels.len().div_ceil(cols);
        let bw = ((room - gap * cols.saturating_sub(1) as f32) / cols as f32).max(0.0).min(desired);
        let height = rows as f32 * bh + rows.saturating_sub(1) as f32 * gap;
        let buttons = labels.iter().enumerate().map(|(i, _)| {
            let row_len = (labels.len() - i / cols * cols).min(cols);
            let x = (width - pad - row_len as f32 * bw - row_len.saturating_sub(1) as f32 * gap + (i % cols) as f32 * (bw + gap)).clamp(0.0, width);
            Bounds { origin: gpui::point(px(x), px((i / cols) as f32 * (bh + gap))), size: gpui::size(px(bw.min(width - x)), px(bh)) }
        }).collect();
        (buttons, height)
    }

    fn details_buttons(&self, char_w: f32, labels: &[&str]) -> Vec<Bounds<Pixels>> {
        let (w, h) = (f32::from(self.bounds.size.width), f32::from(self.bounds.size.height));
        let (buttons, height) = Self::button_grid(w, char_w, labels, (h - self.body_top - line_h() - 24.0).max(0.0));
        let top = (h - 12.0 - height).max(0.0);
        buttons.into_iter().map(|b| {
            let y = (top + f32::from(b.origin.y)).min(h);
            Bounds { origin: self.bounds.origin + gpui::point(b.origin.x, px(y)), size: gpui::size(b.size.width, px(f32::from(b.size.height).min(h - y))) }
        }).collect()
    }
}

pub struct ReaderView {
    session: Session,
    focus: FocusHandle,
    font: Font,
    /// Rows that fit in the focused pane, and in the explorer, from the last paint.
    rows: usize,
    tree_rows: usize,
    /// The text area's bounds, from the last paint.
    text_bounds: Option<Bounds<Pixels>>,
    /// The sticky lines drawn last frame: bounds, pane and source line (a click goes to the line).
    pub(crate) sticky_hits: Vec<(Bounds<Pixels>, u8, usize)>,
    /// The start screen's rows as last painted, by their index in the core's start items.
    pub(crate) start_hits: Vec<(Bounds<Pixels>, usize)>,
    /// The focused pane's underlines and strikethroughs as last drawn (row, display range, colour, modifier bits).
    pub(crate) marks_drawn: Vec<(f32, std::ops::Range<usize>, u32, u8)>,
    /// The focused pane's fold marks as last drawn (row, collapsed): text rows below the sticky band, then the sticky
    /// lines'.
    pub(crate) fold_marks_drawn: Vec<(f32, bool)>,
    /// The breadcrumbs drawn last frame.
    pub(crate) crumb_hits: Vec<CrumbHit>,
    /// The breadcrumb bar has the keys: its pane and highlighted crumb.
    pub(crate) crumb_focus: Option<(u8, usize)>,
    /// The pointer over a blame column; what it is over is found when drawn (it follows scrolling).
    ann_hover: Option<gpui::Point<Pixels>>,
    /// For tests: worktrees Open in New Window was asked for (a real window is not opened).
    #[cfg(feature = "test-support")]
    pub opened_windows: Vec<PathBuf>,
    /// For tests: where Lenz Code is, so Lenz Diff is found beside it.
    #[cfg(feature = "test-support")]
    pub exe: Option<PathBuf>,
    input_state: input_method::InputState,
    csv_ui: csv::Ui,
    terminal_ui: terminal::Ui,
    #[cfg(test)]
    painted_annotations: Vec<String>,
    /// The dirty marks and Git status letters painted on tabs, then on explorer rows, in order.
    #[cfg(test)]
    painted_badges: Vec<&'static str>,
    /// The change-marker letters painted in the gutters, in order, with the colour each was painted in.
    #[cfg(feature = "test-support")]
    painted_markers: Vec<(char, u32)>,
    #[cfg(feature = "test-support")]
    painted_inline: Vec<(u8, inline_diagnostics::Annotation)>,
    /// Each pane's scrollbar as last painted: its thumb (start, size) and the kinds of its marks, top to bottom.
    #[cfg(feature = "test-support")]
    pub painted_scrollbars: Vec<PaintedScrollbar>,
    #[cfg(feature = "test-support")]
    painted_problem_bounds: inline_diagnostics::PaintedUnderlines,
    /// The browsed suggestion's documentation lines as last painted beside the completion list.
    #[cfg(feature = "test-support")]
    painted_completion_doc: Option<Vec<String>>,
    /// The font's advance, measured at each paint.
    char_w: f32,
    /// Window width, from the last paint, for wrapping.
    width: f32,
    dragging: bool,
    /// The pane whose gutter the pointer is over: its fold marks show at full strength.
    gutter_hover: Option<u8>,
    /// The pointer is over a pane's text (past its gutter) or a Markdown preview: an I-beam.
    text_hover: bool,
    /// A drag is selecting the focused Markdown preview's text.
    preview_drag: bool,
    themes_started: bool,
    pub modal: Option<Modal>,
    double_shift: code_core::navigation::DoubleShift,
    navigation_drawn: Option<(u64, usize, usize)>,
    /// Popup viewport and fractional wheel steps, independent of the highlighted option.
    modal_scroll: std::cell::Cell<usize>,
    modal_wheel: std::cell::Cell<f32>,
    /// Fractional trackpad movement belongs to its pane and tab.
    wheel: [gpui::Point<f32>; 2],
    wheel_tab: [Option<(TabId, u64)>; 2],
    tab_scroll: [f32; 2],
    tab_scroll_max: [f32; 2],
    tab_scroll_seen: [Option<TabId>; 2],
    tab_scroll_manual: [bool; 2],
    modal_seen: std::cell::Cell<Option<(std::mem::Discriminant<Modal>, usize)>>,
    /// The project tree's highlighted row (by path, so arriving files above it do not move it), its first
    /// visible row, whether the next paint brings the highlight into view, whether keys go to it and whether it
    /// is shown.
    pub tree_sel: Option<std::sync::Arc<str>>,
    tree_scroll: usize,
    /// Fractional wheel rows of the tree and of the results, as the text keeps its own.
    tree_wheel: f32,
    results_wheel: f32,
    tree_reveal: bool,
    /// The last explorer-reveal ticket applied, so a created entry is selected once.
    reveal_seen: u64,
    pub tree_focus: bool,
    tree_shown: bool,
    /// What Open in Editor and Compare With started; tests record here instead of starting anything.
    pub launched: Vec<lenz_content::open::Launch>,
    pending_explorer: Option<code_core::workspace::Explorer>,
    /// The sidebar's width from the last paint (0 when hidden).
    sidebar_w: f32,
    /// Keys go to the results panel; its first visible result; its list's top and row count from the last paint.
    pub results_focus: bool,
    log_focus: bool,
    log_scroll: usize,
    results_scroll: usize,
    panel_list_y: f32,
    /// The results list's width: the preview is beside it in a wide window (`f32::MAX` when it is below).
    panel_list_w: f32,
    panel_list_rows: usize,
    /// Each tab's x range (in both panes' bars), from the last paint.
    pub tab_hits: Vec<(f32, f32, TabId)>,
    /// Each pane's place, from the last paint.
    pub panes: Vec<PaneGeom>,
    /// Panel sizes: the core's remembered ones, changed here while a divider is dragged.
    layout: Layout,
    /// A divider or tab being dragged, and where a dragged tab would land (pane, position).
    pub drag: Option<Drag>,
    /// Where a dragged tab would land: pane, position among its other tabs, and the mark's x.
    pub tab_drop: Option<(u8, usize, f32)>,
    /// The divider under the pointer (its cursor shows it can be dragged).
    hover_divider: Option<Drag>,
    /// While Ctrl (Cmd on macOS) is held over an identifier in a pane's text: that pane and the identifier's
    /// source bytes, drawn as a link that a click follows.
    pub link: Option<(u8, std::ops::Range<usize>)>,
    /// The documentation popup: its layout (kept while the same documentation shows), scroll, and box when drawn.
    pub doc_popup: Option<DocPopup>,
    completion_doc: Option<DocPopup>,
    completion_doc_building: Option<completion::Building>,
    completion_doc_drawn: Option<Bounds<Pixels>>,
    /// The completion popup as last drawn: its box, first shown row and row count, for clicks and tests.
    completion_drawn: Option<(Bounds<Pixels>, usize, usize)>,
    /// The parameter information box, while one is drawn.
    signature_drawn: Option<Bounds<Pixels>>,
    /// The quick-fix list's box, first shown action and row count, as last drawn.
    code_actions_drawn: Option<(Bounds<Pixels>, usize, usize)>,
    /// The name the pointer rests on (tab and range) and the timer that asks for its documentation; the timer
    /// that closes a pointer's popup once the pointer has left it and its name.
    doc_rest: (Option<NameAt>, Option<gpui::Task<()>>),
    doc_grace: Option<gpui::Task<()>>,
    /// The caret (tab and byte) last seen, and the timer that asks for its name's occurrences once it rests.
    caret_rest: (Option<(TabId, usize)>, Option<gpui::Task<()>>),
    caret_seen: Option<(TabId, code_core::text::SnapshotKey, Selection)>,
    caret_shown: bool,
    caret_blink: Option<gpui::Task<()>>,
    /// The window's height, the results panel's top and the split divider (x for columns, y for rows), from the
    /// last paint.
    height: f32,
    pub panel_top: f32,
    split_at: f32,
    /// The toolbar button under the pointer.
    pub nav_hover: Option<CommandId>,
    modal_hover: Option<Bounds<Pixels>>,
    preview_hover: Option<u8>,
    app_menu_hover: bool,
    commands_hover: bool,
    /// The installed Nerd Font for file icons (looked up once; `Some(None)`: none, shapes are drawn).
    icon_font: Option<Option<gpui::Font>>,
    /// The tree's and tabs' logos drawn so far, at `logo_px` device pixels, and their uploads.
    logos: std::collections::HashMap<&'static str, Arc<gpui::RenderImage>>,
    logo_px: u32,
    logo_uploads: lenz_gpui::image::Uploads,
    /// About's logo texture while About is open.
    pub about_upload: lenz_gpui::image::Uploads,
    /// How a new window's session is configured (tests use disposable directories).
    make_config: std::rc::Rc<dyn Fn(bool) -> Config>,
    /// The image shown: its pixels (weakly: the document owns them), its texture once made off the UI thread, and
    /// its size. Only its texture stays in GPUI's atlas; switching or closing removes the rest.
    image: [Option<Shown>; 2],
    pub uploads: lenz_gpui::image::Uploads,
    /// The Markdown preview each pane shows, if any.
    markdown: [Option<PreviewView>; 2],
    /// The reading style and the palette it was made for.
    reading: Option<(Arc<lenz_content::theme::Palette>, Arc<lenz_gpui::markdown::ReadingStyle>)>,
    /// The pane whose find box has the keys (typing goes to its query).
    pub find_focus: Option<u8>,
    /// The keys go to the find box's replacement row rather than its query.
    pub find_replace: bool,
    /// Each pane's text size (columns, rows) last reported to the core.
    sent_size: [(usize, usize); 2],
    source: source::Cache,
    source_geometry: [Option<SourceGeometry>; 2],
}

/// Identity of the last painted source viewport; pointers cannot reuse old geometry.
struct SourceGeometry {
    font: Font,
    cursor: code_core::projection::Cursor,
    tab: TabId,
    visible: Arc<code_core::folds::Visible>,
    declarations: Option<Arc<lenz_content::highlight::Declarations>>,
    top: (usize, usize),
    column: usize,
    cols: Option<usize>,
    size: Option<(usize, usize)>,
    breadcrumbs: bool,
    split: Option<Split>,
    explorer: bool,
    results: bool,
    sticky: bool,
    annotation_cols: usize,
    layout: Layout,
    viewport: (u32, u32, u32, u32),
}

impl SourceGeometry {
    fn new(view: &code_core::session::View<'_>, reader: &ReaderView) -> Option<Self> {
        if matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending))
            || wanted_image(view).is_some() { return None; }
        let state = reader.session.state();
        Some(Self {
            font: reader.font.clone(), cursor: view.display_start(0)?, tab: view.tab().id(), visible: view.visible(),
            declarations: view.declarations_handle().cloned(),
            top: (view.tab().top_line(), view.tab().top_row()), column: view.tab().column(),
            cols: state.wrap_cols(view.tab()), size: state.text_size(view.tab().pane()),
            breadcrumbs: state.breadcrumbs(), split: state.split(),
            explorer: reader.tree_shown && state.has_tree(), results: state.results().is_some(),
            sticky: state.sticky_scroll(), annotation_cols: state.annotation_cols(view.tab()),
            layout: reader.layout, viewport: (reader.width.to_bits(), reader.height.to_bits(), reader.char_w.to_bits(), reader.body_bottom().to_bits()),
        })
    }

    fn matches(&self, view: &code_core::session::View<'_>, reader: &ReaderView) -> bool {
        let state = reader.session.state();
        let declarations_match = match (self.declarations.as_ref(), view.declarations_handle()) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        self.font == reader.font && self.tab == view.tab().id() && self.cursor.matches(view.snapshot(), view.inlay_hints())
            && Arc::ptr_eq(&self.visible, &view.visible()) && declarations_match
            && self.top == (view.tab().top_line(), view.tab().top_row()) && self.column == view.tab().column()
            && self.cols == state.wrap_cols(view.tab()) && self.size == state.text_size(view.tab().pane())
            && self.breadcrumbs == state.breadcrumbs() && self.split == state.split()
            && self.explorer == (reader.tree_shown && state.has_tree()) && self.results == state.results().is_some()
            && self.sticky == state.sticky_scroll() && self.annotation_cols == state.annotation_cols(view.tab())
            && self.layout == reader.layout && self.viewport == (reader.width.to_bits(), reader.height.to_bits(), reader.char_w.to_bits(), reader.body_bottom().to_bits())
            && !matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending))
            && wanted_image(view).is_none()
    }
}

/// Open a reader window with its own session; `path` opens a file in it.
pub fn open_window(cx: &mut App, make_config: std::rc::Rc<dyn Fn(bool) -> Config>, path: Option<PathBuf>) -> Result<(), String> {
    open_reader_window(cx,make_config,path).map(|_|())
}
/// Native acceptance tools use the installed production constructor; opening does not opt into editing.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn open_acceptance_window_for_test(
    cx: &mut App,
    make_config: std::rc::Rc<dyn Fn(bool) -> Config>,
    path: Option<PathBuf>,
) -> Result<gpui::WindowHandle<ReaderView>, String> {
    open_reader_window(cx, make_config, path)
}
/// Native text input (typing and input methods through `EntityInputHandler`) on the verified backends: Linux X11 and
/// Wayland, and macOS.
fn text_input_supported(compositor: &str) -> bool { cfg!(target_os = "macos") || cfg!(target_os = "linux") && matches!(compositor, "X11" | "Wayland") }

fn open_reader_window(cx: &mut App, make_config: std::rc::Rc<dyn Fn(bool)->Config>,path:Option<PathBuf>)->Result<gpui::WindowHandle<ReaderView>,String> {
    let options = WindowOptions {
        app_id: Some(code_core::chrome::APP_ID.into()),
        focus: true,
        titlebar: Some(TitlebarOptions { title: Some(code_core::chrome::APP_NAME.into()), ..Default::default() }),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        let config = make_config(dark(window));
        let view = cx.new(|cx| ReaderView::new(Session::with_text_input(config, text_input_supported(cx.compositor_name())), path, cx).with_config(make_config));
        view.update(cx, |_, cx| {
            cx.observe_window_appearance(window, |v, window, cx| {
                v.dispatch(Command::Theme(ThemeCommand::NativeAppearance(dark(window))), cx);
            })
            .detach();
            // Coming back to the window: recheck open sources against the disk.
            cx.observe_window_activation(window, |v, window, cx| {
                if window.is_window_active() {
                    v.dispatch(Command::Recheck, cx);
                }
            })
            .detach();
        });
        close::register(&view,window,cx);
        let closing=view.downgrade();
        window.on_window_should_close(cx,move|_,cx|closing.update(cx,|v,cx|v.window_should_close(cx)).unwrap_or(true));
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        view
    })
    .map_err(|e| e.to_string())
}


/// Benchmark markers (the `perf-probe` feature only), milliseconds from launch on stderr: when the first frame
/// built with document content (rows, a preview or a picture) is painted (built, not yet presented), and when every
/// file `READER_PROBE_OPEN` names (`:`-separated, opened as kept tabs under `READER_PROBE_ROOT`) has its tab and
/// the active one is shown (inactive tabs release their content by design).
#[cfg(feature = "perf-probe")]
mod probe {
    use std::sync::{OnceLock, atomic::{AtomicBool, Ordering}};
    pub static START: OnceLock<std::time::Instant> = OnceLock::new();
    static READABLE: AtomicBool = AtomicBool::new(false);
    static TABS: AtomicBool = AtomicBool::new(false);
    fn ms() -> f64 {
        START.get().map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0)
    }
    pub fn painted(readable: bool, tabs_ready: bool) {
        if readable && !READABLE.swap(true, Ordering::Relaxed) {
            eprintln!("lenz-code: readable {:.1} ms", ms());
        }
        if tabs_ready && !TABS.swap(true, Ordering::Relaxed) {
            eprintln!("lenz-code: tabs ready {:.1} ms", ms());
        }
    }
    /// Optional bounded native-font diagnostic; TestPlatform does not reproduce native advances.
    pub fn native_widths(window: &gpui::Window, metrics: lenz_gpui::Metrics<'_>) {
        static CHECKED: AtomicBool = AtomicBool::new(false);
        if CHECKED.swap(true, Ordering::Relaxed) || std::env::var_os("READER_PROBE_NATIVE").is_none() { return; }
        for (name, text) in [("hangul-cho-filler", "\u{115f}\u{200b}"), ("hangul-filler", "\u{3164}\u{200b}"), ("halfwidth-filler", "\u{ffa0}\u{200b}"), ("soft-hyphen", "\u{ad}\u{200b}"), ("family", "👨‍👩‍👧‍👦"), ("marks", "a\u{301}\u{301}")] {
            let run = gpui::TextRun { len: text.len(), font: metrics.font.clone(), color: gpui::Hsla::default(), background_color: None, underline: None, strikethrough: None };
            let pixels = f32::from(window.text_system().shape_line(text.into(), gpui::px(metrics.font_size), &[run], None).width);
            eprintln!("READER_NATIVE_WIDTH_V1 {name} columns={} pixels={pixels}", lenz_content::wrap::width(text));
        }
    }
    pub fn files() -> Vec<std::path::PathBuf> {
        std::env::var_os("READER_PROBE_OPEN").map_or_else(Vec::new, |v| std::env::split_paths(&v).collect())
    }
    /// `READER_PROBE_SCROLL=<seconds>`: scroll one line per frame once readable, recording frame intervals.
    pub fn scroll_seconds() -> Option<f64> {
        std::env::var("READER_PROBE_SCROLL").ok()?.parse().ok()
    }
    /// `READER_PROBE_FOLD=1`: collapse every fold region once they are ready, before scrolling over them.
    pub fn fold() -> bool {
        std::env::var_os("READER_PROBE_FOLD").is_some()
    }
    pub static FOLDED: AtomicBool = AtomicBool::new(false);
    /// `READER_PROBE_WRAP=1`: turn soft wrap on before scrolling.
    pub fn wrap() -> bool {
        std::env::var_os("READER_PROBE_WRAP").is_some()
    }
    pub static WRAPPED: AtomicBool = AtomicBool::new(false);
    /// `READER_PROBE_DECOR=1`: start scrolling only once the active tab has its inlay hints and tokens.
    pub fn decor() -> bool {
        std::env::var_os("READER_PROBE_DECOR").is_some()
    }
    pub static DECORATED: AtomicBool = AtomicBool::new(false);
    /// `READER_PROBE_FIND=<text>`: once readable, find `text` and report when its matches are drawn, then quit.
    pub fn find() -> Option<String> {
        std::env::var("READER_PROBE_FIND").ok()
    }
    pub static FIND_ASKED: OnceLock<std::time::Instant> = OnceLock::new();
    /// Report the find's time once its matches are drawn; whether to quit.
    pub fn found(count: usize) -> bool {
        let Some(asked) = FIND_ASKED.get() else { return false };
        eprintln!("lenz-code: find drawn {:.1} ms after asking, {count} matches", asked.elapsed().as_secs_f64() * 1000.0);
        true
    }
    pub static FRAMES: std::sync::Mutex<Vec<std::time::Instant>> = std::sync::Mutex::new(Vec::new());
    /// Record a frame; once `seconds` of frames are in, report their intervals and say to quit.
    pub fn frame(seconds: f64) -> bool {
        let mut frames = FRAMES.lock().unwrap_or_else(|e| e.into_inner());
        frames.push(std::time::Instant::now());
        if frames.len() < 2 || frames[frames.len() - 1].duration_since(frames[0]).as_secs_f64() < seconds {
            return false;
        }
        let mut gaps: Vec<f64> = frames.windows(2).map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.0).collect();
        eprintln!("READER_PERF {{\"name\":\"reader/scroll-frame-interval\",\"unit\":\"ms\",\"samples\":{gaps:?}}}");
        gaps.sort_by(f64::total_cmp);
        let p = |q: f64| gaps[((gaps.len() - 1) as f64 * q).round() as usize];
        let over = gaps.iter().filter(|g| **g > 1000.0 / 60.0 * 1.5).count();
        eprintln!("lenz-code: scrolled {} frames in {seconds} s: interval median {:.2} ms, p95 {:.2} ms, p99 {:.2} ms, max {:.2} ms; {over} over 25 ms (a missed 60 Hz frame)", gaps.len() + 1, p(0.5), p(0.95), p(0.99), p(1.0));
        true
    }
}

/// Run the reader, opening `path` when given.
pub fn run(path: Option<PathBuf>) {
    let (preferences, notices) = lenz_content::appearance::load();
    let _ = FONTS.set(preferences);
    for notice in notices { eprintln!("lenz-code: {notice}"); }
    #[cfg(feature = "perf-probe")]
    let _ = probe::START.set(std::time::Instant::now());
    // The argument, else the folder the reader was started in (not home or `/`).
    let path = code_core::session::launch_target(path, std::env::current_dir().ok(), std::env::var_os("HOME").map(PathBuf::from));
    gpui_platform::application().run(move |cx: &mut App| {
        if let Err(e) = open_window(cx, std::rc::Rc::new(Config::from_env), path) {
            eprintln!("lenz-code: cannot open a window: {e}");
            cx.quit();
            return;
        }
        // Each window is its own session; the reader ends with the last one.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

/// The Markdown preview a view shows: its layout (shaped off the UI thread; the last one stands in meanwhile),
/// its scroll in pixels and the textures of its visible images.
struct PreviewView {
    tab: TabId,
    doc: std::sync::Weak<lenz_content::markdown::MarkdownDocument>,
    layout: Option<Arc<preview::Layout>>,
    /// What a layout is being shaped for: the preview, the width and the style.
    building: Option<(std::sync::Weak<Preview>, f32, gpui::Task<()>)>,
    top: f32,
    /// The reading line the view was last placed at or reported; when the core's line differs (history, a
    /// result, a refresh), the view moves there.
    line: usize,
    textures: std::collections::HashMap<usize, (std::sync::Weak<lenz_content::image::Pixels>, Option<Arc<gpui::RenderImage>>)>,
    /// A new width and since when: reshaped once it holds for [`SETTLE`] (a resize drag reshapes once).
    resized: Option<(f32, std::time::Instant)>,
    settle: Option<gpui::Task<()>>,
    /// Selected rendered text: anchor and head, bytes of the preview's text (adapter state: layout coordinates).
    selection: Option<(usize, usize)>,
}

/// How long a new width must hold before the preview is reshaped for it, as in Lenz's Preview.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(100);

/// How long the pointer rests on a name before its documentation is asked for, and how long a popup it opened
/// stays after the pointer leaves it and the name.
pub const DOC_REST: std::time::Duration = std::time::Duration::from_millis(600);
pub const DOC_GRACE: std::time::Duration = std::time::Duration::from_millis(300);

/// A name in a tab's document: the tab and the name's range.
type NameAt = (TabId, std::ops::Range<usize>);

/// The documentation popup's layout for one preview, its scroll, and where it was drawn.
/// The window width from which the results preview sits beside the list rather than below it.
const SIDE_PREVIEW_MIN_W: f32 = 900.0;

/// The most suggestions the completion popup shows at once (Page Up/Down moves by this many).
pub const COMPLETION_ROWS: usize = 8;
/// A drawn suggestion: label, detail, selected, unavailable.
type CompletionRow = (String, String, bool, bool);
/// The completion popup's box and rows, and the browsed suggestion's documentation box and lines.
type HighlightedCompletionRow = (CompletionRow, Vec<std::ops::Range<usize>>, Option<&'static str>, bool);
type CompletionFrame = (Bounds<Pixels>, Vec<HighlightedCompletionRow>, Option<(Bounds<Pixels>, Option<Arc<preview::Layout>>, f32)>);
/// The quick-fix list's box, its rows (as completion rows, without detail) and the selected action's reason it is
/// not applied.
type CodeActionFrame = (Bounds<Pixels>, Vec<CompletionRow>, Option<String>);
/// One line of the parameter information box: its text, the byte range drawn as the active parameter, and whether it
/// is secondary (dimmed).
type SignatureLine = (String, Option<std::ops::Range<usize>>, bool);

/// `text` wrapped to `cols` columns, at spaces where it can; at most `max` lines, the last ending in `…` when cut.
fn wrap_text(text: &str, cols: usize, max: usize) -> Vec<String> {
    let cols = cols.max(8);
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split(' ') {
            let mut word = word.to_string();
            while word.chars().count() > cols {
                let head: String = word.chars().take(cols).collect();
                word = word.chars().skip(cols).collect();
                if !line.is_empty() { lines.push(std::mem::take(&mut line)); }
                lines.push(head);
            }
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > cols { lines.push(std::mem::take(&mut line)); }
            if !line.is_empty() { line.push(' '); }
            line.push_str(&word);
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) { lines.pop(); }
    if lines.len() > max {
        lines.truncate(max);
        if let Some(last) = lines.last_mut() { last.push('…'); }
    }
    lines
}

pub struct DocPopup {
    layout: Arc<preview::Layout>,
    top: f32,
    bounds: Bounds<Pixels>,
}

pub use lenz_gpui::find::FindBox;

/// One pane's place on screen: its tab bar (x range and top), its text area and the rows that fit there.
#[derive(Clone, Copy, Debug)]
pub struct PaneGeom {
    pub pane: u8,
    pub bar: (f32, f32, f32),
    /// The pane's text and its gutter (right of the annotation column).
    pub text: Bounds<Pixels>,
    pub rows: usize,
    /// The blame annotation column's width, left of `text` (0: none).
    pub ann: f32,
    /// The scrollbar at the pane's right edge, right of `text` (zero-sized: none).
    pub bar_strip: Bounds<Pixels>,
}

/// What the pointer is dragging: a divider, or a tab (from where it was pressed; `moved` once it travelled).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Explorer,
    Results,
    Split,
    Annotations { pane: u8, tab: TabId, key: code_core::text::SnapshotKey },
    Tab { id: TabId, x: f32, y: f32, moved: bool },
    /// A pane's scrollbar thumb, held `grab` px below its top; pressed at window `y`, `moved` once it travelled. It
    /// ends once the pane shows another tab or file than `shown`.
    Scrollbar { pane: u8, shown: (TabId, code_core::text::BufferId), grab: f32, y: f32, moved: bool },
}

/// The image a view shows.
struct Shown {
    pixels: std::sync::Weak<lenz_content::image::Pixels>,
    texture: Option<Arc<gpui::RenderImage>>,
    size: (u32, u32),
}

impl ReaderView {
    #[cfg(any(not(test), feature = "test-support"))]
    fn start_updates(&self, cx: &mut Context<Self>) {
        let completions = self.session.completions();
        cx.spawn(async move |this, cx| {
            while let Ok(c) = completions.recv().await {
                if this.update(cx, |v, cx| { v.dispatch(c, cx); v.poll(cx) }).is_err() {
                    break;
                }
            }
        })
        .detach();
        let themes = self.session.theme_updates();
        cx.spawn(async move |this, cx| {
            while let Ok(u) = themes.recv().await {
                if this.update(cx, |v, cx| { v.dispatch(Completion::Theme(Box::new(u)), cx); v.poll(cx) }).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Native acceptance uses the production listeners; TestPlatform continues to poll explicitly.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn start_native_updates_for_test(&self, cx: &mut Context<Self>) { self.start_updates(cx); }

    pub fn new(session: Session, path: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let (font, notice) = lenz_gpui::pick_mono(cx, fonts().font_family.as_deref());
        let mut view = Self { session, focus: cx.focus_handle(), font, rows: 0, tree_rows: 0, results_focus: false, log_focus: false, log_scroll: 0, results_scroll: 0, panel_list_y: f32::MAX, panel_list_w: f32::MAX, panel_list_rows: 0, tab_hits: Vec::new(), nav_hover: None, modal_hover: None, preview_hover: None, app_menu_hover: false, commands_hover: false, icon_font: None, logos: Default::default(), logo_px: 0, logo_uploads: Default::default(), about_upload: Default::default(), make_config: std::rc::Rc::new(Config::from_env), text_bounds: None, panes: Vec::new(), layout: Layout::default(), drag: None, tab_drop: None, hover_divider: None, link: None, caret_rest: (None, None), caret_seen: None, caret_shown: true, caret_blink: None, doc_popup: None, completion_doc: None, completion_doc_building: None, completion_doc_drawn: None, completion_drawn: None, signature_drawn: None, code_actions_drawn: None, doc_rest: (None, None), doc_grace: None, height: 600.0, panel_top: f32::MAX, split_at: 0.0, char_w: font_size() * 0.6, width: 800.0, dragging: false, gutter_hover: None, text_hover: false, preview_drag: false, themes_started: false, modal: None, double_shift: Default::default(), navigation_drawn: None, modal_scroll: Default::default(), modal_wheel: Default::default(), wheel: Default::default(), wheel_tab: [None; 2], tab_scroll: Default::default(), tab_scroll_max: Default::default(), tab_scroll_seen: [None; 2], tab_scroll_manual: [false; 2], modal_seen: Default::default(), tree_sel: None, tree_scroll: 0, tree_wheel: 0.0, results_wheel: 0.0, tree_reveal: false, reveal_seen: 0, tree_focus: false, tree_shown: true, launched: Vec::new(), pending_explorer: None, sidebar_w: 0.0, image: [None, None], uploads: Default::default(), markdown: [None, None], reading: None, find_focus: None, find_replace: false, sent_size: [(0, 0); 2], source: source::Cache::default(), source_geometry: [None, None], sticky_hits: Vec::new(), start_hits: Vec::new(), marks_drawn: Vec::new(), fold_marks_drawn: Vec::new(), crumb_hits: Vec::new(), crumb_focus: None, ann_hover: None, #[cfg(feature = "test-support")] opened_windows: Vec::new(), #[cfg(feature = "test-support")] exe: None, input_state: Default::default(), csv_ui: Default::default(), terminal_ui: Default::default(), #[cfg(test)] painted_annotations: Vec::new(), #[cfg(test)] painted_badges: Vec::new(), #[cfg(feature = "test-support")] painted_markers: Vec::new(), #[cfg(feature = "test-support")] painted_inline: Vec::new(), #[cfg(feature = "test-support")] painted_scrollbars: Vec::new(), #[cfg(feature = "test-support")] painted_problem_bounds: Default::default(), #[cfg(feature = "test-support")] painted_completion_doc: None };
        view.layout = view.session.state().layout();
        cx.on_app_quit(|view, cx| {
            view.remember_explorer();
            view.session.dispatch(Command::SaveWorkspace);
            let saved = view.session.workspace_saved();
            cx.background_executor().spawn(async move { let _ = saved.recv(); })
        }).detach();
        if let Some(notice) = notice {
            eprintln!("lenz-code: {notice}");
        }
        // Background results and theme changes enter the session through the same dispatch as commands. The
        // test platform rejects wakeups from other threads, so tests call `poll` themselves (as Lenz's do).
        #[cfg(not(any(test, feature = "test-support")))]
        view.start_updates(cx);
        // A folder opens as the project; a file opens in a tab.
        match path {
            Some(path) if path.is_dir() => view.dispatch(Command::OpenRoot(path), cx),
            Some(path) => view.open(path, cx),
            None => {}
        }
        #[cfg(feature = "perf-probe")]
        {
            if let Some(root) = std::env::var_os("READER_PROBE_ROOT") {
                view.dispatch(Command::OpenRoot(PathBuf::from(root)), cx);
            }
            for file in probe::files() {
                view.dispatch(Command::OpenRetained(file), cx);
            }
        }
        // Recovered work asks before anything else can replace it.
        view.sync_recovery_prompt();
        view
    }

    pub fn with_config(mut self, make_config: std::rc::Rc<dyn Fn(bool) -> Config>) -> Self {
        self.make_config = make_config;
        self
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.dispatch(Command::Open(path), cx);
    }

    /// Open in Editor, or with the system's default app.
    fn open_externally(&mut self, default_app: bool, cx: &mut Context<Self>) {
        match self.session.external_command(&lenz_content::open::Host::current(), lenz_content::appearance::editor().as_deref(), default_app) {
            Ok(command) => self.dispatch(command, cx),
            Err(why) => self.dispatch(Command::Notice(why.into()), cx),
        }
    }

    /// This program, for the apps installed beside it.
    fn exe(&self) -> PathBuf {
        #[cfg(feature = "test-support")]
        if let Some(exe) = &self.exe {
            return exe.clone();
        }
        std::env::current_exe().and_then(std::fs::canonicalize).unwrap_or_default()
    }

    /// Lenz Diff's window comparing `left` with `right`.
    pub fn compare(&mut self, left: &std::path::Path, right: &std::path::Path, cx: &mut Context<Self>) {
        let exe = self.exe();
        let launch = self.session.compare_launch(left, right, &exe, false);
        self.start(launch, Some(code_core::external::COMPARISON_NOTICE.into()), cx);
    }

    /// Starts `launch` detached; a failure, or else `toast`, shows with the core's messages.
    fn start(&mut self, launch: Result<lenz_content::open::Launch, String>, toast: Option<String>, cx: &mut Context<Self>) {
        let message = match launch {
            Err(e) => Some(e),
            Ok(launch) => {
                #[cfg(feature = "test-support")]
                let started = { self.launched.push(launch.clone()); Ok::<(), std::io::Error>(()) };
                #[cfg(not(feature = "test-support"))]
                let started = lenz_content::open::spawn(&launch);
                match started {
                    Err(e) => Some(format!("Cannot start {}: {e}", launch.program)),
                    Ok(()) => toast,
                }
            }
        };
        if let Some(message) = message {
            self.dispatch(Command::Notice(message), cx);
        }
    }

    /// The active tab's horizontal scroll in pixels.
    fn hscroll(&self) -> f32 {
        self.session.state().column() as f32 * self.char_w
    }

    fn scroll_to_x(&mut self, x: f32, cx: &mut Context<Self>) {
        let column = (x / self.char_w).round().clamp(0.0, self.max_column() as f32) as usize;
        self.dispatch(Command::ScrollColumn(column), cx);
    }

    pub fn open_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        // A new root's rows replace the old ones: reset what pointed into them.
        self.close_modal(cx);
        self.dispatch(Command::OpenRoot(root), cx);
    }

    /// Activate an explorer row: a directory toggles, a file opens.
    fn activate_row(&mut self, i: usize, retain: bool, cx: &mut Context<Self>) {
        let Some(row) = self.session.state().tree_rows().nth(i).cloned() else { return };
        self.tree_sel = Some(row.path.clone());
        self.remember_explorer();
        self.tree_reveal = true;
        self.dispatch(Command::ActivateTree { path: row.path, retain }, cx);
        cx.notify();
    }

    fn tree_len(&self) -> usize {
        self.session.state().tree_rows().count()
    }

    /// The highlighted tree row's index in the current rows (the first row when none is highlighted yet).
    pub fn tree_at(&self) -> usize {
        self.tree_sel.as_ref().and_then(|sel| self.session.state().tree_rows().position(|r| r.path == *sel)).unwrap_or(0)
    }

    fn tree_move(&mut self, to: usize) {
        self.tree_sel = self.session.state().tree_rows().nth(to.min(self.tree_len().saturating_sub(1))).map(|r| r.path.clone());
        self.tree_reveal = true;
        self.remember_explorer();
    }

    fn theme_ids(&self) -> Vec<(String, String)> {
        std::iter::once(("system".to_string(), "Follow system".to_string()))
            .chain(self.session.theme_entries().iter().map(|e| (e.id.clone(), e.label.clone())))
            .collect()
    }

    /// Registry updates retain the highlighted ID, including when user themes arrive asynchronously.
    fn refresh_theme_entries(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.modal, Some(Modal::Themes { .. })) { return; }
        let entries = self.theme_ids();
        let Some(Modal::Themes { ids, at }) = &mut self.modal else { return };
        let selected = ids.get(*at).map(|(id, _)| id.as_str());
        let next = selected.and_then(|id| entries.iter().position(|(entry, _)| entry == id));
        *at = next.unwrap_or(0);
        *ids = entries;
        if next.is_none() { self.dispatch(Command::PreviewTheme(None), cx); }
    }

    /// Close the open modal and its core search or runtime dialog.
    fn close_modal(&mut self, cx: &mut Context<Self>) {
        self.modal_seen.set(None);
        self.terminal_ui.path_context = None;
        match self.modal.take() {
            Some(Modal::StatusDetails { kind: Details::Terminal { ticket }, .. }) => self.dispatch(Command::Terminal(code_core::terminal::Command::Confirm { ticket, accept: false }), cx),
            Some(Modal::Navigation { .. }) => { self.navigation_drawn = None; self.dispatch(Command::CloseSearchEverywhere, cx); },
            Some(Modal::ServerRuntime { .. }) => self.dispatch(Command::CloseServerRuntime, cx),
            Some(Modal::StatusDetails { kind: Details::Install, .. }) => self.dispatch(Command::CloseServerInstall, cx),
            Some(Modal::Themes { .. }) => self.dispatch(Command::PreviewTheme(None), cx),
            Some(Modal::StatusDetails { kind: Details::External, .. }) => self.choose_external(code_core::external::Choice::Cancel,cx),
            Some(Modal::StatusDetails { kind: Details::Reconcile, .. }) => self.choose_reconcile(code_core::reconcile::Choice::KeepDraft,cx),
            Some(Modal::StatusDetails { kind: Details::Replace, .. }) => self.choose_replace(code_core::save::ReplaceChoice::Cancel,cx),
            Some(Modal::StatusDetails { kind: Details::Close, .. }) => self.choose_close(code_core::close::Choice::Cancel,cx),
            Some(Modal::StatusDetails { kind: Details::Recovered, .. }) => self.choose_recovery(code_core::recovery::Choice::Later,cx),
            Some(Modal::StatusDetails { kind: Details::Change, .. }) => self.dispatch(Command::CloseChange, cx),
            Some(Modal::StatusDetails { kind: Details::Commit, .. }) => self.dispatch(Command::CloseCommit, cx),
            Some(Modal::Branches { .. } | Modal::BranchMenu { .. }) => self.dispatch(Command::CloseBranches, cx),
            Some(Modal::StatusDetails { kind: Details::Checkout, .. }) => self.dispatch(Command::CheckoutChoice(code_core::branches::CheckoutChoice::Cancel), cx),
            _ => {}
        }
        self.sync_install_prompt();
        cx.notify();
    }

    /// Apply every background result and theme update that has arrived.
    pub fn poll(&mut self, cx: &mut Context<Self>) {
        let (completions, themes) = (self.session.completions(), self.session.theme_updates());
        loop {
            if let Ok(c) = completions.try_recv() {
                self.dispatch(c, cx);
            } else if let Ok(u) = themes.try_recv() {
                self.dispatch(Completion::Theme(Box::new(u)), cx);
            } else {
                break;
            }
        }
    }

    /// One key or menu step of a panel's size, within the limits its drag keeps: 20 px of explorer, or a twentieth
    /// of the height (results) or the panel (split). A hidden explorer is shown first.
    fn resize_panel(&mut self, panel: code_core::actions::Panel, grow: bool, cx: &mut Context<Self>) {
        use code_core::actions::Panel;
        let step = |now: u32, lo: u32, hi: u32| Some(if grow { now + 50 } else { now.saturating_sub(50) }.clamp(lo, hi));
        match panel {
            Panel::Explorer if !self.tree_shown => { self.tree_shown = true; self.remember_explorer(); cx.notify(); return; }
            Panel::Explorer if self.sidebar_w <= 0.0 => return,
            Panel::Explorer => self.layout.explorer_width = Some((self.sidebar_w + if grow { 20.0 } else { -20.0 }).clamp(120.0, (self.width - 160.0).max(120.0)).round() as u32),
            Panel::Results => {
                let (top, bottom) = (toolbar_h() + line_h(), self.body_bottom());
                let shown = ((bottom - self.panel_top.min(bottom)) / (bottom - top).max(1.0) * 1000.0).round() as u32;
                self.layout.results_permille = step(self.layout.results_permille.unwrap_or(shown), 50, 950);
            }
            Panel::FirstPane => self.layout.split_permille = step(self.layout.split_permille.unwrap_or(500), 100, 900),
        }
        self.dispatch(Command::SetLayout(self.layout), cx);
    }

    fn remember_explorer(&mut self) {
        if self.pending_explorer.is_some() || self.session.state().project().is_none() { return; }
        let explorer = code_core::workspace::Explorer { visible: self.tree_shown, selected: self.tree_sel.as_ref().map(ToString::to_string), top: self.session.state().tree_rows().nth(self.tree_scroll).map(|r| r.path.to_string()) };
        if self.session.state().explorer() != &explorer { self.session.dispatch(Command::SetExplorer(explorer)); }
    }

    fn sync_workspace(&mut self, previous: u64) {
        if self.session.state().workspace_revision() != previous {
            let explorer = self.session.state().explorer().clone();
            self.tree_shown = explorer.visible; self.tree_sel = None; self.tree_scroll = 0;
            self.tree_focus &= self.tree_shown;
            self.layout = self.session.state().layout();
            self.pending_explorer = Some(explorer);
            self.results_scroll = 0; self.results_focus = false;
        }
        let (Some(explorer), Some(project)) = (&self.pending_explorer, self.session.state().project()) else { return };
        let at = |path: &Option<String>| path.as_ref().and_then(|path| project.rows().iter().position(|r| r.path.as_ref() == path));
        let selected = at(&explorer.selected); let top = at(&explorer.top);
        let done = matches!(project.coverage(), code_core::project::Coverage::Complete(_) | code_core::project::Coverage::Unavailable(_));
        if (explorer.selected.is_none() || selected.is_some() || done) && (explorer.top.is_none() || top.is_some() || done) {
            self.tree_sel = selected.map(|i| project.rows()[i].path.clone());
            self.tree_scroll = top.unwrap_or(0); self.tree_reveal = false;
            self.pending_explorer = None;
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Follow the core's decision prompts. Any core dispatch can open or settle one (each finishes by accounting
    /// completed saves), a render-time layout command included, so rendering calls this after its own dispatches.
    fn sync_install_prompt(&mut self) {
        match (self.session.state().server_install().map(|m|m.text()), &mut self.modal) {
            (Some(text),Some(Modal::StatusDetails {kind:Details::Install,text:body,..})) => *body=text,
            (Some(text),None) => {
                self.modal_scroll.set(0);
                self.modal=Some(Modal::StatusDetails {title:"Install language server",text,logo:None,kind:Details::Install});
            }
            (None,Some(Modal::StatusDetails {kind:Details::Install,..})) => self.modal=None,
            _=>{}
        }
    }
    fn sync_prompts(&mut self) {
        self.sync_install_prompt();
        self.sync_close_prompt();
        self.sync_external_prompt();
        self.sync_replace_prompt();
        self.sync_recovery_prompt();
        self.sync_reconcile_prompt();
        self.sync_terminal_prompt();
    }

    fn metrics(&self) -> Metrics<'_> {
        metrics(&self.font)
    }

    fn dispatch(&mut self, input: impl Into<code_core::session::Input>, cx: &mut Context<Self>) {
        let terminal_before = (self.session.state().terminal().focused(), self.session.state().terminal().active().map(|t| t.id()));
        let before = self.session.state().results().map(|r| r.id());
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let selected = (self.session.state().active().map(|t| t.id()), self.session.state().selection());
        let input: code_core::session::Input = input.into();
        #[cfg(feature = "perf-probe")]
        let (started, kind) = (std::time::Instant::now(), input.kind());
        let theme_update = matches!(&input, code_core::session::Input::Command(Command::Theme(_)) | code_core::session::Input::Completion(Completion::Theme(_)));
        if matches!(&input, code_core::session::Input::Command(Command::OpenRoot(_))) { self.remember_explorer(); }
        let workspace = self.session.state().workspace_revision();
        let tree_revision = self.session.state().tree_revision();
        let library_read = matches!(&input, code_core::session::Input::Completion(Completion::LibraryRead { .. }));
        let mut effect = self.session.dispatch(input);
        if terminal_before != (self.session.state().terminal().focused(), self.session.state().terminal().active().map(|t| t.id())) {
            self.cancel_input(true, cx);
            self.terminal_ui.marked = None;
            self.terminal_ui.suppress_commit = None;
            self.terminal_ui.wheel = 0.0;
            self.terminal_ui.mouse_capture = None;
            if self.session.state().terminal().focused() {
                self.tree_focus = false; self.results_focus = false; self.log_focus = false;
                self.find_focus = None; self.crumb_focus = None;
            }
        }
        self.layout.annotation_width = self.session.state().layout().annotation_width;
        if let Some(ticket) = self.session.external_ready() {
            let delivery = self.session.dispatch(Command::TakeExternal { ticket });
            effect.repaint |= delivery.repaint;
            effect.external = delivery.external.or(effect.external);
        }
        if matches!(self.modal, Some(Modal::Navigation { .. })) && self.session.state().navigation().is_none() {
            self.modal = None;
            self.navigation_drawn = None;
            self.modal_seen.set(None);
            self.tree_focus = false;
            self.results_focus = false;
        }
        if theme_update { self.refresh_theme_entries(cx); }
        self.sync_workspace(workspace);
        if self.session.state().tree_revision() != tree_revision {
            self.pending_explorer = None;
            self.tree_shown = true;
            self.tree_sel = self.session.state().active_in_tree();
            self.tree_reveal = true;
        } else if library_read && self.tree_sel == self.session.state().active_in_tree() {
            self.tree_reveal = true;
        }
        // The session says which explorer row to show and select.
        if let Some((path, ticket)) = self.session.state().tree_reveal()
            && ticket > self.reveal_seen {
            self.reveal_seen = ticket;
            if let Some(i) = self.session.state().tree_rows().position(|r| r.path == path) {
                self.tree_sel = Some(path);
                self.tree_shown = true;
                self.tree_reveal = true;
                self.tree_scroll = self.tree_scroll.min(i);
            }
        }
        #[cfg(feature = "perf-probe")]
        if started.elapsed().as_millis() >= 30 {
            eprintln!("lenz-code: slow dispatch {kind} {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
        }
        // Definition candidates or a peek arrived: keys go to them, as for any result list the user asked for.
        if let Some(r) = self.session.state().results().filter(|r| Some(r.id()) != before)
            && matches!(r.results_kind(), code_core::search::ResultsKind::Definitions | code_core::search::ResultsKind::Implementations | code_core::search::ResultsKind::TypeDefinitions | code_core::search::ResultsKind::Peek | code_core::search::ResultsKind::Hierarchy)
        {
            self.results_focus = true;
            self.tree_focus = false;
            self.results_scroll = 0;
        }
        if let Some(text) = effect.copy {
            if effect.copy_ticket.is_none_or(|ticket| self.session.clipboard_text_current(ticket)) {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            if let Some(ticket) = effect.copy_ticket { self.dispatch(Command::ClipboardTextPublished(ticket), cx); }
        }
        if let Some(plan) = effect.external { self.start(Ok(plan.launch),plan.toast,cx); }
        // Linux: a new text selection becomes the primary selection (pasted elsewhere with the middle button); showing
        // another tab or pane with its own selection is not a new one.
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if self.session.state().active().map(|t| t.id()) == selected.0
            && self.session.state().selection() != selected.1
            && let Some(text) = self.session.state().selected_text()
        {
            cx.write_to_primary(ClipboardItem::new_string(text.to_owned()));
        }
        // The core's commit popup opens, follows and closes its dialog.
        // The core's Git Branches popup opens and closes its list.
        match (self.session.state().branches().is_some(), &self.modal) {
            (true, None) => self.modal = Some(Modal::Branches { query: String::new(), at: 0 }),
            (false, Some(Modal::Branches { .. } | Modal::BranchMenu { .. })) => self.modal = None,
            _ => {}
        }
        // So do the checkout-problem dialog's.
        for (kind, title, now) in [(Details::Change, "Marked change", self.session.inspected_change_text()), (Details::Commit, "Commit", self.session.commit_text()), (Details::Checkout, "Checkout", self.session.checkout_text())] {
            match (now, &mut self.modal) {
                (Some(t), Some(Modal::StatusDetails { kind: k, text, .. })) if *k == kind => *text = t,
                (None, Some(Modal::StatusDetails { kind: k, .. })) if *k == kind => self.modal = None,
                (Some(text), None) => {
                    self.modal_scroll.set(0);
                    self.modal = Some(Modal::StatusDetails { title, text, logo: None, kind });
                }
                _ => {}
            }
        }
        self.sync_prompts();
        self.invalidate_input_source(cx);
        close::schedule(cx);
        if effect.repaint {
            cx.notify();
        }
        self.rest_caret(cx);
    }

    fn reset_caret_blink(&mut self) {
        self.caret_blink = None;
        self.caret_seen = None;
        self.caret_shown = true;
    }

    fn sync_caret_blink(&mut self, window: &Window, cx: &mut Context<Self>) {
        let target = (window.is_window_active() && self.focus.is_focused(window) && self.modal.is_none() && !self.session.state().terminal().focused() && !self.csv_active()
            && !self.log_focus && !self.tree_focus && !self.results_focus && self.find_focus.is_none() && self.crumb_focus.is_none()
            && self.input_state.capture.is_none()).then(|| {
                let state = self.session.state();
                let tab = state.active()?;
                let view = state.presentation_view(tab.id())?;
                if matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return None; }
                Some((tab.id(), view.snapshot().key(), tab.selection()?))
            }).flatten();
        if target.is_none() { self.reset_caret_blink(); return; }
        if target == self.caret_seen && self.caret_blink.is_some() { return; }
        self.reset_caret_blink();
        self.caret_seen = target;
        self.caret_blink = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                if this.update(cx, |v, cx| { v.caret_shown = !v.caret_shown; cx.notify(); }).is_err() { break; }
            }
        }));
    }

    /// When the caret moved, ask for its name's occurrences once it has rested there.
    fn rest_caret(&mut self, cx: &mut Context<Self>) {
        if self.input_state.capture.is_some() || self.session.state().terminal().focused() { self.caret_rest = (None, None); return; }
        let state = self.session.state();
        let caret = state.active().map(|t| t.id()).zip(state.selection().map(|s| s.head));
        if caret == self.caret_rest.0 {
            return;
        }
        self.caret_rest = (caret, None);
        if caret.is_some() {
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(code_core::session::HIGHLIGHT_REST).await;
                let _ = this.update(cx, |v, cx| v.dispatch(Command::HighlightAt, cx));
            });
            self.caret_rest.1 = Some(task);
        }
    }

    /// Run a catalog command, unless it is disabled now. Every [`CommandId`] is handled here.
    pub fn run(&mut self, id: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        if id == CommandId::PasteTerminalPath && let Some(command) = self.terminal_ui.path_context.take() {
            if self.session.disabled(id).is_none() {
                match command { Ok(command) => self.dispatch(command, cx), Err(why) => self.dispatch(Command::Notice(why), cx) }
            }
            return;
        }
        self.terminal_ui.path_context = None;
        if id == CommandId::ToggleActivityLog {
            self.log_focus = !self.session.state().activity_log_shown();
            self.log_scroll = 0;
        }
        if id == CommandId::CsvTable && self.session.disabled(id).is_none() {
            self.find_focus = None; self.results_focus = false; self.tree_focus = false; self.crumb_focus = None;
            self.csv_ui.selected = None; self.csv_ui.marked = None;
            self.cancel_input(true, cx);
        }
        // A create acts on the explorer's highlighted row, which only this adapter knows (the first row is drawn
        // highlighted before anything is picked, as in the terminal).
        if id.targets_explorer() {
            if self.tree_sel.is_none() {
                self.tree_sel = self.session.state().tree_rows().nth(self.tree_at()).map(|r| r.path.clone());
            }
            self.remember_explorer();
        }
        match self.session.action(id) {
            CatalogAction::Disabled(_) => {},
            CatalogAction::Command(command) => {
                // Rendered Markdown selection belongs to the pixel layout.
                if id == CommandId::SelectAll && self.focused_markdown().is_some_and(|m| m.layout.is_some()) {
                    let len = self.focused_markdown().and_then(|m| m.layout.as_ref()).map_or(0, |l| l.preview.doc.text.len());
                    self.select_preview(Some((0, len)), cx);
                    return;
                }
                if id == CommandId::Copy && self.focused_markdown().is_some() {
                    self.dispatch(Command::CopyText(self.preview_text().unwrap_or_default()), cx);
                    return;
                }
                if matches!(id, CommandId::Outline | CommandId::FileSymbols | CommandId::RerunSearch | CommandId::ShowProblems | CommandId::CheckProjectSyntax) {
                    self.results_scroll = 0;
                }
                if id == CommandId::CloseResults {
                    self.results_focus = false;
                }
                self.dispatch(command, cx);
                match id {
                    // Shown results take the keys; hidden ones give them back to the text.
                    CommandId::Outline | CommandId::ShowProblems | CommandId::CheckProjectSyntax | CommandId::ToggleResults => {
                        self.results_focus = self.session.state().results().is_some();
                        self.tree_focus &= !self.results_focus;
                    }
                    // Another list, or another result of this one: its selected row comes into view.
                    CommandId::PreviousResults | CommandId::NextResults | CommandId::NextResult | CommandId::PreviousResult => {
                        if matches!(id, CommandId::PreviousResults | CommandId::NextResults) { self.results_scroll = 0; }
                        if let Some(i) = self.session.state().results().and_then(|r| r.selected()) { self.reveal_result(i); }
                    }
                    CommandId::LocateFile => {
                        self.tree_sel = self.session.state().active_in_tree();
                        self.tree_reveal = true;
                        self.tree_shown = true;
                        self.tree_focus = true;
                        self.results_focus = false;
                        cx.notify();
                    }
                    _ => {},
                }
            }
            CatalogAction::Interface(action) => match action {
                InterfaceAction::Resize { panel, grow } => self.resize_panel(panel, grow, cx),
                InterfaceAction::RenameTerminal(id) => {
                    if let Some(tab) = self.session.state().terminal().tabs().iter().find(|tab| tab.id() == id) {
                        self.modal_scroll.set(0);
                        self.modal = Some(Modal::RenameTerminal { id, input: tab.label().into(), selected: true, error: None });
                    }
                }
                InterfaceAction::PasteTerminalPath => {
                    let answer = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: true, multiple: true, prompt: Some("Insert Path into Terminal".into()) });
                    let terminal = self.session.state().terminal().active().map(|t| t.id());
                    cx.spawn_in(window, async move |this,cx| {
                        if let Ok(Ok(Some(paths))) = answer.await {
                            let _ = this.update(cx, |v,cx| {
                                if terminal == v.session.state().terminal().active().map(|t| t.id()) && v.modal.is_none() {
                                    v.dispatch(Command::Terminal(code_core::terminal::Command::Paths(paths)),cx);
                                }
                            });
                        }
                    }).detach();
                }
                InterfaceAction::SaveAs => self.prompt_save_as(window, cx),
                InterfaceAction::NewScratch => self.prompt_new_scratch(window, cx),
                InterfaceAction::ScratchFiles => self.prompt_scratch_files(window, cx),
                InterfaceAction::Open => {
                    let answer = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Open".into()) });
                    cx.spawn_in(window, async move |this, cx| {
                        if let Ok(Ok(Some(paths))) = answer.await
                            && let Some(path) = paths.into_iter().next()
                        {
                            let _ = this.update(cx, |v, cx| v.open(path, cx));
                        }
                    })
                    .detach();
                }
                InterfaceAction::OpenFolder => {
                    let answer = cx.prompt_for_paths(gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Open Folder".into()) });
                    cx.spawn_in(window, async move |this, cx| {
                        if let Ok(Ok(Some(paths))) = answer.await
                            && let Some(path) = paths.into_iter().next()
                        {
                            let _ = this.update(cx, |v, cx| v.open_root(path, cx));
                        }
                    })
                    .detach();
                }
                InterfaceAction::LanguageServer => {
                    self.dispatch(Command::CloseDocumentation, cx);
                    self.modal = Some(Modal::LanguageServer { at: 0 });
                    cx.notify();
                }
                InterfaceAction::Keymap => {
                    self.dispatch(Command::CloseDocumentation, cx);
                    self.modal = Some(Modal::Keymap { at: 0, capture: false });
                    cx.notify();
                }
                InterfaceAction::ServerDetails | InterfaceAction::StatusDetails | InterfaceAction::EditingPreferences | InterfaceAction::SqlDialect | InterfaceAction::Problems => {
                    self.dispatch(Command::CloseDocumentation, cx);
                    self.modal_scroll.set(0);
                    let (title, text) = match action {
                        InterfaceAction::ServerDetails => ("Server details", self.session.language_server_details()),
                        InterfaceAction::EditingPreferences => ("Editing preferences", self.session.editing_preferences_details()),
                        InterfaceAction::SqlDialect => ("SQL dialect", self.session.sql_dialect_details()),
                        InterfaceAction::Problems => ("Problems", self.session.problems_details()),
                        _ => ("Status details", self.session.status_details()),
                    };
                    self.modal = Some(Modal::StatusDetails { title, text, logo: None, kind: if action == InterfaceAction::SqlDialect { Details::Sql } else { Details::Text } });
                    cx.notify();
                }
                InterfaceAction::About => {
                    self.dispatch(Command::CloseDocumentation, cx);
                    self.modal_scroll.set(0);
                    // The small checked-in PNG is decoded only now; no filesystem reads.
                    let logo = lenz_content::image::decode(lenz_content::about::LOGO_PNG, lenz_content::image::Format::Png)
                        .ok()
                        .map(|d| AboutLogo(lenz_gpui::image::texture(d.pixels)));
                    self.modal = Some(Modal::StatusDetails { title: code_core::chrome::ABOUT_TITLE, text: code_core::chrome::about(), logo, kind: Details::Text });
                    cx.notify();
                }
                InterfaceAction::OpenInEditor => self.open_externally(false, cx),
                InterfaceAction::OpenWithDefaultApp => self.open_externally(true, cx),
                InterfaceAction::NewBranch => self.modal = Some(Modal::NewBranch { from: None, name: String::new(), checkout: true, error: None }),
                InterfaceAction::Create(kind) => match self.session.create_target() {
                    Some(dir) => self.modal = Some(Modal::Create { kind, dir, name: String::new(), error: None }),
                    None => self.dispatch(Command::Notice("No folder is open".into()), cx),
                },
                InterfaceAction::PasteExplorerEntry => match self.session.paste_entry_request() {
                    Ok(request) => self.modal = Some(Modal::PasteEntry { name: self.session.paste_entry_start(&request), request, selected: true, error: None }),
                    Err(why) => self.dispatch(Command::Notice(why.into()), cx),
                },
                InterfaceAction::ExplorerHistory => {
                    let launch = self.session.explorer_history_launch(&self.exe(), false);
                    self.start(launch, None, cx);
                },
                InterfaceAction::Relocate(kind) => match self.session.relocate_target() {
                    Some(from) => self.modal = Some(Modal::Relocate { kind, input: self.session.relocate_start(kind, &from), from, error: None }),
                    None => self.dispatch(Command::Notice("Highlight a file or folder in the explorer".into()), cx),
                },
                InterfaceAction::ConfirmDelete(removal) => match self.session.relocate_target() {
                    Some(from) => self.modal = Some(Modal::ConfirmDelete { error: self.session.delete_refusal(removal, &from), removal, from }),
                    None => self.dispatch(Command::Notice("Highlight a file or folder in the explorer".into()), cx),
                },
                InterfaceAction::CheckoutRevision => self.modal = Some(Modal::Revision { input: String::new(), error: None }),
                InterfaceAction::CompareWithBase => self.git_diff(&code_core::branches::GitDiff::BaseBranch, cx),
                InterfaceAction::CompareSelected => {
                    let exe = self.exe();
                    let launch = self.session.compare_selected_launch(&exe, false);
                    self.start(launch, Some(code_core::external::COMPARISON_NOTICE.into()), cx);
                }
                InterfaceAction::ShowLocalChanges => self.git_diff(&code_core::branches::GitDiff::LocalChanges, cx),
                InterfaceAction::ShowBranchChanges => self.git_diff(&code_core::branches::GitDiff::BranchChanges(None), cx),
                InterfaceAction::ResolveConflicts => {
                    let exe = self.exe();
                    let launch = self.session.merge_launch(&exe, false);
                    self.start(launch, None, cx);
                }
                InterfaceAction::ShowCommitDiff => {
                    let exe = self.exe();
                    let launch = self.session.commit_diff_launch(&exe, false);
                    self.dispatch(Command::CloseCommit, cx);
                    self.start(launch, None, cx);
                }
                InterfaceAction::Paste => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) { self.dispatch(Command::Paste(text), cx); }
                }
                InterfaceAction::CompareWith => {
                    let row = self.tree_focus.then(|| self.tree_sel.clone()).flatten();
                    match self.session.compare_source(row.as_deref()) {
                        Err(why) => self.dispatch(Command::Notice(why.into()), cx),
                        Ok(left) => {
                            // A folder compares with a folder, a file with a file.
                            let dir = left.is_dir();
                            let answer = cx.prompt_for_paths(gpui::PathPromptOptions { files: !dir, directories: dir, multiple: false, prompt: Some(CommandId::CompareWith.label().into()) });
                            cx.spawn_in(window, async move |this, cx| {
                                if let Ok(Ok(Some(paths))) = answer.await
                                    && let Some(right) = paths.into_iter().next()
                                {
                                    let _ = this.update(cx, |v, cx| v.compare(&left, &right, cx));
                                }
                            })
                            .detach();
                        }
                    }
                }
                InterfaceAction::OpenRecent => {
                    self.modal = Some(Modal::Recent { at: 0 });
                    cx.notify();
                }
                InterfaceAction::FindFile => self.open_navigation(code_core::navigation::Scope::Files, cx),
                InterfaceAction::SearchEverywhere => self.open_navigation(code_core::navigation::Scope::All, cx),
                InterfaceAction::FindSymbol => self.open_navigation(code_core::navigation::Scope::Symbols, cx),
                InterfaceAction::FindAction => self.open_navigation(code_core::navigation::Scope::Actions, cx),
                InterfaceAction::FindDeclarations | InterfaceAction::Symbols => {
                    self.modal = Some(Modal::Declarations { query: String::new(), semantic: id == CommandId::Symbols });
                    cx.notify();
                }
                InterfaceAction::ToggleExplorer => {
                    self.tree_shown = !self.tree_shown;
                    self.remember_explorer();
                    self.tree_focus &= self.tree_shown;
                    cx.notify();
                }
                InterfaceAction::FocusBreadcrumbs => {
                    let pane = self.session.state().focused_pane();
                    self.crumb_focus = self.crumb_hits.iter().filter(|h| h.pane == pane).filter_map(|h| h.crumb).max().map(|i| (pane, i));
                    cx.notify();
                }
                InterfaceAction::GoToLine => {
                    self.modal = Some(Modal::GoTo { input: String::new(), error: None });
                    cx.notify();
                }
                InterfaceAction::SearchProject => {
                    let results = self.session.state().results();
                    let (query, files) = (results.map(|r| r.query().to_owned()).unwrap_or_default(), results.map(|r| r.pattern().to_owned()).unwrap_or_default());
                    self.modal = Some(Modal::Search { selected: !query.is_empty(), query, files, in_files: false, project: true });
                    cx.notify();
                }
                InterfaceAction::FindInFile => {
                    let seed = self.session.find_seed();
                    self.find_focus = Some(self.session.state().focused_pane());
                    self.find_replace = false;
                    self.results_focus = false;
                    self.tree_focus = false;
                    self.dispatch(Command::Find(seed), cx);
                }
                InterfaceAction::SetBookmarkMnemonic => {
                    self.modal = Some(Modal::Mnemonic);
                    cx.notify();
                }
                InterfaceAction::RecentFiles => self.open_navigation(code_core::navigation::Scope::Recent, cx),
                InterfaceAction::ShowBookmarks | InterfaceAction::RecentLocations => {
                    let list = match id {
                        CommandId::ShowBookmarks => PlaceList::Bookmarks,
                        _ => PlaceList::RecentLocations,
                    };
                    self.modal = Some(Modal::Places { list, query: String::new(), at: 0 });
                    self.dispatch(Command::CheckPlaces, cx);
                    cx.notify();
                }
                InterfaceAction::CloseWindow | InterfaceAction::QuitAll => close::request(self.session.id(),action == InterfaceAction::QuitAll,cx),
                InterfaceAction::NewWindow => {
                    let make_config = self.make_config.clone();
                    cx.defer(move |cx| {
                        if let Err(e) = open_window(cx, make_config, None) {
                            eprintln!("lenz-code: cannot open a window: {e}");
                        }
                    });
                }
                InterfaceAction::ServerRuntime => {
                    self.dispatch(Command::OpenServerRuntime, cx);
                    let path = self.session.state().server_runtime().and_then(|m| m.configured.as_ref()).map(|p| p.display().to_string()).unwrap_or_default();
                    self.modal = Some(Modal::ServerRuntime { path, at: 0 });
                    cx.notify();
                }
                InterfaceAction::ConfigureSqlMetadata => {
                    if self.session.sql_identity().is_some_and(|i| i.dialect == code_core::sql_dialect::Dialect::DuckDb) {
                        self.run(CommandId::SqlDialect, window, cx);
                        let identity = self.session.sql_identity();
                        let answer = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Choose a DuckDB or SQLite database (configure only)".into()) });
                        cx.spawn_in(window, async move |this, cx| {
                            if let Ok(Ok(Some(paths))) = answer.await && let Some(path) = paths.into_iter().next() {
                                let _ = this.update(cx, |v, cx| {
                                    if v.session.sql_identity() != identity {
                                        v.dispatch(Command::Notice("SQL metadata: the SQL document changed while choosing a database; choose the database again".into()), cx); return;
                                    }
                                    let Some(source) = path.to_str() else {
                                        v.dispatch(Command::Notice("Choose a DuckDB or SQLite database with a UTF-8 path".into()), cx); return;
                                    };
                                    v.dispatch(Command::ConfigureSqlMetadata(source.into()), cx);
                                    let group = CommandGroup::Metadata;
                                    let at = v.session.menu(Some(group)).iter().position(|row| row.target == MenuTarget::Run(CommandId::ConnectSqlMetadata)).unwrap_or(0);
                                    v.modal = Some(Modal::CommandGroup { group, at });
                                    cx.notify();
                                });
                            }
                        }).detach();
                    } else {
                        self.modal = Some(Modal::SqlMetadata { path: self.session.sql_metadata_source().into() });
                        cx.notify();
                    }
                }
                InterfaceAction::WorkspaceRoot => {
                    let path = self.session.state().provider().and_then(|v| v.root).map(|r| r.display().to_string()).unwrap_or_default();
                    self.modal = Some(Modal::Root { path });
                    cx.notify();
                }
                InterfaceAction::ChooseTheme => {
                    self.dispatch(Command::Theme(ThemeCommand::OpenChooser), cx);
                    let ids = self.theme_ids();
                    let at = ids.iter().position(|(id, _)| *id == self.session.theme_id()).unwrap_or(0);
                    self.modal = Some(Modal::Themes { ids, at });
                    cx.notify();
                }
            },
        }
    }

    /// Keys while a find box has them: typing edits the query or the replacement row, Enter steps (or replaces the
    /// current match from the replacement row), Alt+A replaces every match, Alt+C/W/R toggle the options, Alt+Enter
    /// lists every match, Tab switches rows, Escape closes. Caret and page keys go on to the text. Returns whether
    /// it took the key.
    fn find_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let k = &ev.keystroke;
        let query = self.session.state().active().and_then(|t| t.find()).map(|f| f.query().to_owned());
        let Some(query) = query.filter(|_| self.find_focus == Some(self.session.state().focused_pane())) else {
            self.find_focus = None;
            self.find_replace = false;
            return false;
        };
        let (key, ctrl, alt) = (k.key.as_str(), k.modifiers.control || k.modifiers.platform, k.modifiers.alt);
        match key {
            "c" | "w" | "r" if alt && !ctrl => {
                let option = match key { "c" => FindOption::Case, "w" => FindOption::Word, _ => FindOption::Regex };
                self.dispatch(Command::ToggleFindOption(option), cx);
            }
            "a" if alt && !ctrl => self.dispatch(Command::ReplaceFind { all: true }, cx),
            "enter" if alt => self.dispatch(Command::ListMatches, cx),
            "enter" if self.find_replace => self.dispatch(Command::ReplaceFind { all: false }, cx),
            "enter" => self.dispatch(Command::FindStep { forward: !k.modifiers.shift }, cx),
            "tab" => self.find_replace = !k.modifiers.shift,
            "escape" => {
                self.find_focus = None;
                self.find_replace = false;
                self.dispatch(Command::CloseFind, cx);
            }
            "backspace" if self.find_replace => {
                let mut replacement = self.session.replace_text().to_owned();
                replacement.pop();
                self.dispatch(Command::SetReplaceText(replacement), cx);
            }
            "backspace" => {
                let mut query = query;
                query.pop();
                self.dispatch(Command::Find(query), cx);
            }
            _ if ctrl || alt => return false,
            _ => match k.key_char.as_deref().filter(|c| !c.chars().any(char::is_control)) {
                Some(typed) if self.find_replace => self.dispatch(Command::SetReplaceText(format!("{}{typed}", self.session.replace_text())), cx),
                Some(typed) => self.dispatch(Command::Find(query + typed), cx),
                None => return false,
            },
        }
        true
    }

    /// The shared find control over this pane's document, with its replacement row.
    pub fn find_box(&self, g: &PaneGeom) -> Option<FindBox> {
        let state = self.session.state();
        let tab = state.shown(g.pane)?;
        if state.csv(tab.id()).is_some() { return None; }
        let f = tab.find()?;
        let o = self.session.find_options();
        Some(FindBox {
            bounds: Bounds::default(),
            toggles: [(FindOption::Case, Bounds::default()); 3],
            on: [o.case, o.word, o.regex],
            query: f.query().to_owned(),
            counter: f.counter(),
            error: f.error().is_some(),
            focused: self.find_focus == Some(g.pane),
            show_replace: true,
            replace_label: CommandId::ReplaceMatch.label().to_owned(),
            replace: self.session.replace_text().to_owned(),
            replace_focused: self.find_replace,
            replace_row: Bounds::default(),
        }.layout(g.text, line_h(), self.char_w))
    }

    #[cfg(feature = "test-support")]
    pub fn enable_draft_input_for_test(&mut self) { self.session.enable_text_input_for_test(); }

    /// A core command through this view's own dispatch, as its menus and keys send them (other crates' parity tests).
    #[cfg(feature = "test-support")]
    pub fn dispatch_for_test(&mut self, command: Command, cx: &mut Context<Self>) { self.dispatch(command, cx); }

    /// Deletions go to this private trash, never the user's own (other crates' tests).
    #[cfg(feature = "test-support")]
    pub fn set_trash_home_for_test(&mut self, home: PathBuf) { self.session.set_trash_home_for_test(home); }

    /// A currently visible result row's click point for real input replays.
    #[cfg(feature = "test-support")]
    pub fn result_point_of(&self, index: usize) -> Option<(f32, f32)> {
        let row = self.session.state().results()?.row_of_hit(index)?.checked_sub(self.results_scroll)?;
        (row < self.panel_list_rows).then_some((self.sidebar_w + 16.0, self.panel_list_y + (row as f32 + 0.5) * line_h()))
    }

    fn draft_context(&self) -> bool {
        self.session.text_input_enabled() && self.session.vim_insert_allowed() && !self.csv_active() && self.modal.is_none() && !self.session.state().terminal().focused() && !self.log_focus && !self.tree_focus && !self.results_focus && self.find_focus.is_none() && self.crumb_focus.is_none()
            && self.session.state().active().and_then(|t| self.session.state().view(t.id())).is_some_and(|v| !matches!(v.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)))
    }

    fn draft_target(&self) -> Option<(TabId, code_core::text::SnapshotKey, std::ops::Range<usize>)> {
        if !self.draft_context() { return None; }
        let state = self.session.state();
        let view = state.view(state.active()?.id())?;
        if view.edit_disabled().is_some() || matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return None; }
        let range = if let Some(selection) = view.tab().selection() { selection.range() }
            else { let start = view.snapshot().line_range(view.tab().top_line())?.start; start..start };
        Some((view.tab().id(), view.snapshot().key(), range))
    }

    /// Typed text replaces every selection of the focused view; the core makes it one transaction.
    fn replace_draft_input(&mut self, text: &str, kind: code_core::text::EditKind, cx: &mut Context<Self>) -> bool {
        if self.draft_target().is_none() {
            if self.draft_context() && self.session.disabled(CommandId::Paste).is_some() { return true; }
            return false;
        }
        self.dispatch(Command::InsertText { text: text.to_owned(), kind }, cx);
        true
    }

    /// Ctrl+Space asks for completion; while its popup shows, it takes Up/Down/Page keys, Tab/Enter (accept) and
    /// Escape. Every other key, punctuation included, types as usual.
    fn completion_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let k = &ev.keystroke;
        let (control, other) = (k.modifiers.control || k.modifiers.platform, k.modifiers.alt || k.modifiers.shift);
        if k.key == "space" && k.modifiers.control && !other { self.run(CommandId::Completion, window, cx); return true; }
        if !control && !other && k.key == "escape" && self.session.state().completion().is_none() && self.session.state().signature_help().is_some() {
            self.dispatch(Command::CloseSignature, cx);
            return true;
        }
        // The quick-fix list takes the same keys while it shows.
        if !control && !other && self.session.state().code_actions().is_some() {
            let command = match k.key.as_str() {
                "up" => Command::MoveCodeAction(-1),
                "down" => Command::MoveCodeAction(1),
                "pageup" => Command::MoveCodeAction(-(COMPLETION_ROWS as isize)),
                "pagedown" => Command::MoveCodeAction(COMPLETION_ROWS as isize),
                "enter" | "tab" => Command::ApplyCodeAction,
                "escape" => Command::CloseCodeActions,
                _ => return false,
            };
            self.dispatch(command, cx);
            return true;
        }
        if !control && k.modifiers.alt && !k.modifiers.shift && self.session.state().completion().is_some()
            && matches!(k.key.as_str(), "pageup"|"pagedown"|"home"|"end") {
            if let Some(d) = &mut self.completion_doc {
                let maximum=(d.layout.height-f32::from(d.bounds.size.height)).max(0.0);
                d.top=match k.key.as_str(){"home"=>0.0,"end"=>maximum,"pageup"=>(d.top-f32::from(d.bounds.size.height)).max(0.0),_ =>(d.top+f32::from(d.bounds.size.height)).min(maximum)};
                cx.notify();
            }
            return true;
        }
        if control || other || self.session.state().completion().is_none() { return false; }
        let command = match k.key.as_str() {
            "up" => Command::MoveCompletion(-1),
            "down" => Command::MoveCompletion(1),
            "pageup" => Command::MoveCompletion(-(COMPLETION_ROWS as isize)),
            "pagedown" => Command::MoveCompletion(COMPLETION_ROWS as isize),
            "enter" | "tab" => Command::AcceptCompletion,
            "escape" => Command::CloseCompletion,
            _ => return false,
        };
        self.dispatch(command, cx);
        true
    }

    fn draft_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.draft_context() { return false; }
        let key = &ev.keystroke;
        let control = key.modifiers.control || key.modifiers.platform;
        if !key.modifiers.alt && matches!(key.key.as_str(),"home"|"end") {
            let motion=match (control,key.key.as_str()) { (true,"home")=>Motion::DocumentStart,(true,_)=>Motion::DocumentEnd,(false,"home")=>Motion::LineStart,_=>Motion::LineEnd };
            self.dispatch(if key.modifiers.shift {Command::Extend(motion)}else{Command::MoveCaret(motion)},cx);
            true
        } else if control && !key.modifiers.alt && key.key == "z" {
            if self.draft_target().is_none() { return false; }
            self.run(if key.modifiers.shift { CommandId::Redo } else { CommandId::Undo }, window, cx);
            true
        } else if control && !key.modifiers.alt && !key.modifiers.shift && matches!(key.key.as_str(), "x" | "v") {
            if !self.session.text_input_enabled() { return false; }
            self.run(if key.key == "x" { CommandId::Cut } else { CommandId::Paste }, window, cx); true
        } else if !key.modifiers.platform && matches!(key.key.as_str(), "backspace" | "delete") {
            // Ctrl (or Alt, macOS's Option) deletes to the word stop, as IntelliJ's Ctrl+Backspace / Ctrl+Delete.
            if self.draft_target().is_none() { return false; }
            self.dispatch(Command::Delete { backward: key.key == "backspace", word: key.modifiers.control || key.modifiers.alt }, cx); true
        } else if !control && !key.modifiers.alt && !key.modifiers.shift && key.key == "enter" {
            let newline = self.session.state().active().and_then(|t| self.session.state().view(t.id())).map_or("\n", |v| v.input_newline());
            self.replace_draft_input(newline, code_core::text::EditKind::Typing, cx)
        } else if !control && !key.modifiers.alt && key.key == "tab" {
            // Tab indents what is selected (or inserts the file's step) and Shift+Tab takes a step back; a completion
            // popup took the key before this.
            if self.draft_target().is_none() { return false; }
            self.run(if key.modifiers.shift { CommandId::OutdentSelection } else { CommandId::IndentSelection }, window, cx);
            true
        } else if !control && !key.modifiers.alt {
            key.key_char.as_deref().filter(|text| !text.chars().any(char::is_control))
                .is_some_and(|text| self.replace_draft_input(text, code_core::text::EditKind::Typing, cx))
        } else { false }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_caret_blink(); cx.notify();
        let k = &ev.keystroke;
        if !matches!(k.key.as_str(), "shift" | "control" | "alt" | "super" | "meta" | "fn") { self.double_shift.interrupt(); }
        let (key, ctrl, shift) = (k.key.as_str(), k.modifiers.control || k.modifiers.platform, k.modifiers.shift);
        // The modal takes every key first.
        if self.modal.is_some() {
            if !matches!(key, "enter" | "escape") { self.modal_seen.set(None); }
            self.modal_key(ev, window, cx);
            if self.modal.is_none() { self.modal_seen.set(None); }
            cx.notify();
            return;
        }
        if self.terminal_key(ev, window, cx) { return; }
        if self.activity_key(ev, window, cx) { return; }
        // A focused find box takes its own keys before the keymap (Alt+Enter lists every match there, not Quick Fix);
        // keys it does not use go on.
        if self.find_focus.is_some() && self.crumb_focus.is_none() && !(key == "escape" && self.session.state().documentation().is_some()) && self.find_key(ev, cx) {
            return;
        }
        if self.tree_focus && ctrl && !k.modifiers.alt && !shift {
            let action = match key { "c" => Some(CommandId::CopyExplorerEntry), "v" => Some(CommandId::PasteExplorerEntry), _ => None };
            if let Some(action) = action { return self.run(action, window, cx); }
        }
        // IntelliJ's Stretch keys in a focused panel, before the keymap's (Ctrl+Shift+Up/Down move lines in the text).
        if k.modifiers.control && shift && !k.modifiers.alt {
            let stretch = match (key, self.tree_focus, self.results_focus) {
                ("left", true, _) => Some(CommandId::NarrowExplorer),
                ("right", true, _) => Some(CommandId::WidenExplorer),
                ("up", _, true) => Some(CommandId::TallerResults),
                ("down", _, true) => Some(CommandId::ShorterResults),
                _ => None,
            };
            if let Some(id) = stretch { return self.run(id, window, cx); }
        }
        if self.tree_focus && key == "delete" && !ctrl && !shift && !k.modifiers.alt {
            return self.run(CommandId::DeleteExplorerEntry, window, cx);
        }
        if self.explorer_search_key(ev, cx) { return; }
        // Alt+Left / Alt+Right in the results panel go through its earlier and later lists, before the keymap's Back.
        if self.results_focus && k.modifiers.alt && !ctrl && !shift && matches!(key, "left" | "right") {
            return self.run(if key == "left" { CommandId::PreviousResults } else { CommandId::NextResults }, window, cx);
        }
        if !self.log_focus && !self.tree_focus && !self.results_focus && self.find_focus.is_none() && self.crumb_focus.is_none() && !self.csv_active() && self.input_state.capture.is_none() {
            let text = k.key_char.as_deref().filter(|text| text.chars().count() == 1 && !text.chars().any(char::is_control));
            let press = text.map(|text| code_core::keymap::KeyPress { key: text.into(), ctrl, alt: k.modifiers.alt, shift: false })
                .or_else(|| code_core::keymap::KeyPress::new(key, ctrl, shift, k.modifiers.alt));
            if let Some(press) = press.filter(|press| self.session.vim_handles(press)) {
                self.dispatch(Command::VimKey(press), cx);
                cx.stop_propagation();
                return;
            }
        }
        if self.csv_key(ev, window, cx) { return; }
        // Semantic commands take their keys from the session's keymap: a preset or an override changes what runs, and
        // an unavailable command says why instead of vanishing. Typing keys are never bound there.
        if let Some(id) = self.key_command(ev) {
            match self.session.disabled(id) {
                Some(why) => self.dispatch(Command::Notice(why.into()), cx),
                None => self.run(id, window, cx),
            }
            return;
        }
        if let Some((pane, i)) = self.crumb_focus {
            let last = self.crumb_hits.iter().filter(|h| h.pane == pane).filter_map(|h| h.crumb).max().unwrap_or(0);
            let shown = |j: usize| self.crumb_hits.iter().any(|h| h.pane == pane && h.crumb == Some(j));
            match key {
                "left" => self.crumb_focus = (0..i).rev().find(|&j| shown(j)).map_or(Some((pane, i)), |j| Some((pane, j))),
                "right" => self.crumb_focus = Some((pane, (i + 1..=last).find(|&j| shown(j)).unwrap_or(i))),
                "enter" | "down" => self.modal = Some(Modal::Siblings { pane, crumb: i, query: String::new(), at: 0 }),
                "escape" => self.crumb_focus = None,
                _ => {
                    self.crumb_focus = None;
                    return self.on_key(ev, window, cx);
                }
            }
            return cx.notify();
        }
        if key == "escape" && self.session.state().documentation().is_some() {
            return self.dispatch(Command::CloseDocumentation, cx);
        }
        if self.find_focus.is_some() && self.find_key(ev, cx) {
            return;
        }
        if key == "f3" && !ctrl {
            return self.run(if shift { CommandId::PreviousMatch } else { CommandId::NextMatch }, window, cx);
        }
        if self.results_focus && self.results_key(key, ctrl, window, cx) {
            return;
        }
        if self.tree_focus && self.tree_key(key, ctrl, cx) {
            return;
        }
        // Back/Forward: Alt+Left/Right, and Cmd+[ / Cmd+] as on macOS.
        match (key, k.modifiers.alt && !ctrl, k.modifiers.platform) {
            ("left", true, _) | ("[", _, true) => return self.run(CommandId::Back, window, cx),
            ("right", true, _) | ("]", _, true) => return self.run(CommandId::Forward, window, cx),
            _ => {}
        }
        {
            if self.input_key(ev, window, cx) { return; }
            if self.input_state.capture.is_none() && self.completion_key(ev, window, cx) { cx.stop_propagation(); return; }
            if self.draft_key(ev, window, cx) { cx.stop_propagation(); return; }
        }
        // The preview scrolls in pixels; it has no caret to move.
        if !ctrl && self.focused_markdown().is_some_and(|m| m.layout.is_some()) {
            let view = self.rows as f32 * line_h();
            let to = self.focused_markdown().map_or(0.0, |m| m.top);
            let to = match key {
                "down" => Some(to + line_h() * 2.0),
                "up" => Some(to - line_h() * 2.0),
                "pagedown" => Some(to + (view - line_h() * 2.0).max(line_h())),
                "pageup" => Some(to - (view - line_h() * 2.0).max(line_h())),
                "home" => Some(0.0),
                "end" => Some(f32::MAX),
                _ => None,
            };
            if let Some(to) = to {
                return self.scroll_preview(self.session.state().focused_pane(), to, cx);
            }
        }
        let page = self.rows.max(1);
        match (key, ctrl, shift) {
            ("v", true, true) => self.run(CommandId::MarkdownSource, window, cx),
            ("tab", true, false) => self.run(CommandId::NextTab, window, cx),
            ("tab", true, true) => self.run(CommandId::PreviousTab, window, cx),
            ("k", true, true) => self.run(CommandId::PinTab, window, cx),
            // Ctrl+1 / Ctrl+2 focus a pane; Ctrl+` opens Git Branches.
            ("`", true, true) | ("~", true, _) => self.run(CommandId::GitBranches, window, cx),
            // Linux reports Ctrl+Shift+digit as the shifted symbol (US layout) with Shift cleared, macOS as the digit.
            (d, true, true) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => self.dispatch(Command::SetMnemonic(d.chars().next().unwrap_or('0')), cx),
            (s, true, _) if s.len() == 1 && ")!@#$%^&*(".contains(s) => self.dispatch(Command::SetMnemonic(char::from(b'0' + ")!@#$%^&*(".find(s).unwrap_or(0) as u8)), cx),
            (d, false, false) if k.modifiers.alt && d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => self.dispatch(Command::JumpMnemonic(d.chars().next().unwrap_or('0')), cx),
            // AZERTY: Alt and the digit row's key without Shift. GPUI reports no physical key, so Ctrl+Shift+digit there
            // arrives as Ctrl+digit and stays unavailable (Ctrl+F11 sets a numbered bookmark instead).
            (s, false, false) if k.modifiers.alt && AZERTY_DIGITS.contains(&s) => {
                let d = AZERTY_DIGITS.iter().position(|a| *a == s).unwrap_or(0);
                self.dispatch(Command::JumpMnemonic(char::from(b'0' + d as u8)), cx);
            }
            ("1", true, false) => self.dispatch(Command::FocusPane(0), cx),
            ("2", true, false) => self.dispatch(Command::FocusPane(1), cx),
            ("f10", _, _) => {
                self.modal = Some(Modal::Commands { at: 0 });
                cx.notify();
            }
            // Ctrl+Shift+E shows and focuses the explorer; Ctrl+T finds declarations (workspace symbols).
            ("e", true, true) => {
                self.tree_focus = self.session.state().has_tree();
                self.results_focus &= !self.tree_focus;
                self.tree_shown |= self.tree_focus;
                cx.notify();
            }
            ("b", true, false) => self.run(CommandId::ToggleExplorer, window, cx),
            ("a", true, false) => self.run(CommandId::SelectAll, window, cx),
            ("c", true, false) => self.run(CommandId::Copy, window, cx),
            ("t", true, false) => self.run(CommandId::FindDeclarations, window, cx),
            ("escape", false, false) if self.session.state().asking().is_some() => self.dispatch(Command::CancelSemantic, cx),
            // Several cursors: Escape goes back to one before it closes anything else. The add-cursor and
            // next-occurrence chords come from the keymap, above, so a preset or an override moves them.
            ("escape", false, false) if self.session.disabled(CommandId::CollapseSelections).is_none() => self.run(CommandId::CollapseSelections, window, cx),
            // macOS: Cmd+Left/Right go to the line's ends and Cmd+Up/Down to the document's, as there natively and in
            // IntelliJ's macOS keymap; Shift selects. ponytail: no Option+arrow word motion there, Option+Left/Right is Back.
            (arrow @ ("left" | "right" | "up" | "down"), true, shift) if k.modifiers.platform && !k.modifiers.control => {
                let motion = match arrow { "left" => Motion::LineStart, "right" => Motion::LineEnd, "up" => Motion::DocumentStart, _ => Motion::DocumentEnd };
                self.dispatch(if shift { Command::Extend(motion) } else { Command::MoveCaret(motion) }, cx);
            }
            // IntelliJ's word and document motions: Ctrl+arrows by word, Shift to select; Ctrl+Up/Down scroll a row.
            ("left", true, true) => self.extend(Motion::WordLeft, cx),
            ("right", true, true) => self.extend(Motion::WordRight, cx),
            ("left", true, false) => self.dispatch(Command::MoveCaret(Motion::WordLeft), cx),
            ("right", true, false) => self.dispatch(Command::MoveCaret(Motion::WordRight), cx),
            ("up", true, false) => self.dispatch(Command::ScrollRows(-1), cx),
            ("down", true, false) => self.dispatch(Command::ScrollRows(1), cx),
            ("home", true, true) => self.extend(Motion::DocumentStart, cx),
            ("end", true, true) => self.extend(Motion::DocumentEnd, cx),
            ("left", false, true) => self.extend(Motion::Left, cx),
            ("right", false, true) => self.extend(Motion::Right, cx),
            ("up", false, true) => self.extend(Motion::Up, cx),
            ("down", false, true) => self.extend(Motion::Down, cx),
            ("home", false, true) => self.extend(Motion::LineStart, cx),
            ("end", false, true) => self.extend(Motion::LineEnd, cx),
            ("left", false, false) => self.dispatch(Command::MoveCaret(Motion::Left), cx),
            ("right", false, false) => self.dispatch(Command::MoveCaret(Motion::Right), cx),
            ("up", false, false) => self.dispatch(Command::MoveCaret(Motion::Up), cx),
            ("down", false, false) => self.dispatch(Command::MoveCaret(Motion::Down), cx),
            ("pagedown", _, _) => self.dispatch(Command::ScrollRows(page as isize), cx),
            ("pageup", _, _) => self.dispatch(Command::ScrollRows(-(page as isize)), cx),
            ("end", _, _) => self.dispatch(Command::Scroll(usize::MAX), cx),
            ("home", _, _) => {
                self.dispatch(Command::ScrollColumn(0), cx);
                self.dispatch(Command::Scroll(0), cx);
            }
            _ => {}
        }
    }

    /// The command the session's keymap gives this key press, if any.
    fn key_command(&self, ev: &KeyDownEvent) -> Option<CommandId> {
        let k = &ev.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let press = code_core::keymap::KeyPress::new(&k.key, ctrl, k.modifiers.shift, k.modifiers.alt)?;
        self.session.key_command(&press)
    }

    /// The key a menu shows: the session's keymap, or the fixed editing keys this adapter maps itself.
    pub fn key_hint(&self, id: CommandId) -> Option<String> {
        self.session.key_hint(id)
    }

    /// Accept the open list modal's highlighted row, as Enter does: run the command, pick the theme, open the recent
    /// folder or the found file, follow the crumb's entry, or go to the place (a missing place keeps the list open).
    fn modal_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.action_menu_help().is_some() { cx.notify(); return; }
        if matches!(self.modal, Some(Modal::Navigation { .. })) {
            let revision = self.session.state().navigation().map(|n| n.revision);
            if let Some(revision) = revision { self.accept_navigation(revision, window, cx); }
            return;
        }
        match self.modal.take() {
            Some(Modal::StatusDetails { .. }) => {},
            Some(Modal::Navigation { .. }) => {}, // Handled above without taking the modal.
            Some(Modal::ServerRuntime { path, at }) => {
                let choice = self.session.state().server_runtime().and_then(|m| m.choices(&path).get(at).cloned());
                self.modal = Some(Modal::ServerRuntime { path, at });
                if let Some((_, path)) = choice {
                    if let Some(Modal::ServerRuntime { path: input, .. }) = &mut self.modal { *input = path.as_ref().map_or_else(String::new, |p| p.display().to_string()); }
                    self.dispatch(Command::SetServerRuntime(path), cx);
                }
            }
            Some(Modal::LanguageServer { at }) => {
                if let Some(id) = self.server_rows().get(at).and_then(|(_, id)| *id) {
                    self.run(id, window, cx);
                } else {
                    self.modal = Some(Modal::LanguageServer { at });
                }
            }
            Some(Modal::Commands { at }) => {
                if let Some(&id) = CommandId::ALL.get(at) { self.run(id, window, cx); }
            }
            Some(m @ (Modal::AppMenu { at } | Modal::CommandGroup { at, .. })) => {
                let group = match m { Modal::CommandGroup { group, .. } => Some(group), _ => None };
                match self.session.menu(group).get(at).map(|r| r.target) {
                    Some(MenuTarget::Group(group)) => self.modal = Some(Modal::CommandGroup { group, at: 1 }),
                    Some(MenuTarget::Back) => self.modal = Some(Modal::AppMenu { at: self.session.menu(None).iter().position(|row| group.is_some_and(|g| row.target == MenuTarget::Group(g))).unwrap_or(0) }),
                    Some(MenuTarget::Run(id)) => self.run(id, window, cx),
                    Some(MenuTarget::AllCommands) => self.open_navigation(code_core::navigation::Scope::Actions, cx),
                    Some(MenuTarget::Separator) | None => {}
                }
            }
            Some(Modal::Themes { ids, at }) => {
                if let Some(selection) = ids.get(at).and_then(|(id, _)| ThemeSelection::from_id(id)) {
                    self.dispatch(Command::Theme(ThemeCommand::Select(selection)), cx);
                }
            }
            Some(Modal::ScratchFiles { files, at }) => {
                if let Some(path) = files.get(at) { self.dispatch(Command::OpenScratch(path.clone()), cx); }
            }
            Some(Modal::Recent { at }) => {
                if let Some(root) = self.session.state().recent().get(at).cloned() {
                    self.open_root(root, cx);
                }
            }
            Some(Modal::Siblings { pane, crumb, query, at }) => {
                self.crumb_focus = None;
                if let Some(item) = self.sibling_items(pane, crumb, &query).into_iter().nth(at) {
                    self.follow_sibling(item.target, cx);
                }
            }
            Some(Modal::Branches { query, at }) => {
                use code_core::branches::BranchTarget;
                let Some(row) = self.session.state().branch_rows(&query).into_iter().nth(at) else { return self.modal = Some(Modal::Branches { query, at }) };
                match &row.target {
                    BranchTarget::Action(id) => {
                        self.dispatch(Command::CloseBranches, cx);
                        self.run(*id, window, cx);
                    }
                    t => match self.session.branch_enter(t) {
                        Some(action) => self.branch_action(t, action, window, cx),
                        None => self.modal = Some(Modal::Branches { query, at }),
                    },
                }
            }
            Some(Modal::BranchMenu { target, label, at, query, row }) => match self.session.branch_actions(&target).get(at) {
                Some(&(_, action)) => self.branch_action(&target, action, window, cx),
                None => self.modal = Some(Modal::BranchMenu { target, label, at, query, row }),
            },
            m @ Some(Modal::NewBranch { .. } | Modal::Revision { .. } | Modal::Create { .. } | Modal::PasteEntry { .. } | Modal::SaveAs { .. } | Modal::NewScratch { .. } | Modal::RenameTerminal { .. } | Modal::Relocate { .. } | Modal::ConfirmDelete { .. }) => self.modal = m,
            Some(Modal::Places { list, query, at }) => {
                let row = self.session.state().place_rows(list, &query).into_iter().nth(at).filter(|r| !r.missing);
                match row {
                    Some(row) => self.dispatch(Command::GoToPlace(row.place), cx),
                    None => self.modal = Some(Modal::Places { list, query, at }),
                }
            }
            m @ Some(Modal::Search { .. } | Modal::Declarations { .. } | Modal::Root { .. } | Modal::SqlMetadata { .. } | Modal::Context { .. } | Modal::GoTo { .. } | Modal::Mnemonic | Modal::Keymap { .. }) => self.modal = m,
            None => {}
        }
    }

    /// A click on modal row `row` (of [`Self::modal_rows`]): a list's row is highlighted and accepted as Enter would
    /// accept it; in the key chooser a bookmark's row sets its key on the caret's line; header and input rows do
    /// nothing.
    fn modal_click(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if row == usize::MAX || matches!(self.modal, Some(Modal::StatusDetails { .. })) { return; }
        if matches!(self.modal, Some(Modal::Mnemonic)) {
            // Row 0 is the prompt; each other row starts with its key.
            let key = row.checked_sub(1).and_then(|i| self.keyed_bookmarks().into_iter().nth(i)).and_then(|r| r.label.chars().next());
            if let Some(key) = key {
                self.modal = None;
                self.dispatch(Command::SetMnemonic(key), cx);
            }
            return;
        }
        match &mut self.modal {
            Some(Modal::Navigation { .. }) => return,
            Some(Modal::Commands { at } | Modal::AppMenu { at } | Modal::CommandGroup { at, .. } | Modal::LanguageServer { at } | Modal::Recent { at }) => *at = row,
            Some(Modal::ScratchFiles { at, .. }) => match row.checked_sub(1) { Some(i) => *at = i, None => return },
            Some(Modal::Themes { at, .. }) => match row.checked_sub(THEME_HEADER_ROWS) {
                Some(i) => *at = i,
                None => return,
            },
            Some(Modal::Keymap { at, .. }) => match row.checked_sub(1) {
                Some(i) if i < self.session.keymap().rows().len() => *at = i,
                _ => return,
            },
            Some(Modal::ServerRuntime { at, .. } | Modal::Siblings { at, .. } | Modal::Places { at, .. } | Modal::Branches { at, .. }) => match row.checked_sub(FINDER_HEADER_ROWS) {
                Some(i) => *at = i,
                None => return,
            },
            Some(Modal::BranchMenu { at, .. }) => match row.checked_sub(1) {
                Some(i) => *at = i,
                None => return,
            },
            Some(Modal::StatusDetails { .. } | Modal::Search { .. } | Modal::Declarations { .. } | Modal::Root { .. } | Modal::SqlMetadata { .. } | Modal::Context { .. } | Modal::GoTo { .. } | Modal::Mnemonic | Modal::NewBranch { .. } | Modal::Revision { .. } | Modal::Create { .. } | Modal::PasteEntry { .. } | Modal::SaveAs { .. } | Modal::NewScratch { .. } | Modal::RenameTerminal { .. } | Modal::Relocate { .. } | Modal::ConfirmDelete { .. }) | None => return,
        }
        self.modal_accept(window, cx);
    }

    fn modal_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Modal::StatusDetails { kind: Details::Terminal { ticket }, .. }) = self.modal {
            match ev.keystroke.key.as_str() {
                "enter" if !ev.is_held => self.dispatch(Command::Terminal(code_core::terminal::Command::Confirm { ticket, accept:true }),cx),
                "escape" => self.dispatch(Command::Terminal(code_core::terminal::Command::Confirm { ticket, accept:false }),cx),
                _ => self.terminal_prompt_scroll(ev),
            }
            cx.stop_propagation(); return;
        }
        if self.file_prompt_key(ev, cx) { return; }
        if matches!(self.modal, Some(Modal::Navigation { .. })) {
            return self.navigation_key(ev, window, cx);
        }
        let key = ev.keystroke.key.as_str();
        // The keymap: arrows move, Enter captures the next key as the highlighted command's shortcut, Delete restores
        // its preset chord, Escape leaves a capture or closes. The header row says which state it is in.
        if let Some(Modal::Keymap { at, capture }) = self.modal.clone() {
            let rows = self.session.keymap().rows();
            let at = at.min(rows.len().saturating_sub(1));
            if capture {
                if key == "escape" {
                    self.modal = Some(Modal::Keymap { at, capture: false });
                    return cx.notify();
                }
                let k = &ev.keystroke;
                let ctrl = k.modifiers.control || k.modifiers.platform;
                let Some(press) = code_core::keymap::KeyPress::new(&k.key, ctrl, k.modifiers.shift, k.modifiers.alt) else {
                    return cx.notify();
                };
                let Some(chord) = code_core::keymap::Chord::from_press(&press) else {
                    self.dispatch(Command::Notice("A shortcut needs Ctrl, Alt or a function key".into()), cx);
                    return cx.notify();
                };
                self.modal = Some(Modal::Keymap { at, capture: false });
                if let Some(row) = rows.get(at) {
                    self.dispatch(Command::SetKeybinding { command: row.command, chord: chord.label() }, cx);
                }
                return cx.notify();
            }
            self.modal = Some(Modal::Keymap { at, capture: false });
            match key {
                "escape" => self.close_modal(cx),
                "enter" => self.modal = Some(Modal::Keymap { at, capture: true }),
                "delete" => {
                    if let Some(row) = rows.get(at) {
                        self.dispatch(Command::ResetKeybinding(row.command), cx);
                    }
                }
                "up" | "down" | "pageup" | "pagedown" | "home" | "end" => {
                    let last = rows.len().saturating_sub(1);
                    let next = match key {
                        "up" => at.saturating_sub(1),
                        "down" => (at + 1).min(last),
                        "home" => 0,
                        "end" => last,
                        "pageup" => at.saturating_sub(10),
                        _ => (at + 10).min(last),
                    };
                    self.modal = Some(Modal::Keymap { at: next, capture: false });
                }
                _ => {}
            }
            return cx.notify();
        }
        if let Some(Modal::StatusDetails { kind: Details::Checkout, .. }) = &self.modal {
            // s Smart Checkout, f Force, y confirms the discard (another key than f, and never a held one); Escape
            // cancels; Enter takes the first safe choice.
            use code_core::branches::CheckoutChoice as C;
            let choices: Vec<C> = self.session.checkout_choices().into_iter().map(|c| c.1).collect();
            let m = &ev.keystroke.modifiers;
            let plain = !(m.control || m.platform || m.alt) && !ev.is_held;
            let pick = match key {
                "escape" => Some(C::Cancel),
                "s" if plain => Some(C::Smart),
                "f" if plain => Some(C::Force),
                "y" if plain => Some(C::ConfirmForce),
                "enter" => choices.first().copied().filter(|c| *c != C::ConfirmForce),
                _ => None,
            };
            if let Some(c) = pick.filter(|c| choices.contains(c)) {
                self.dispatch(Command::CheckoutChoice(c), cx);
            }
            return cx.notify();
        }
        if matches!(self.modal,Some(Modal::StatusDetails {kind:Details::External,..})) {
            let ctrl=ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
            let choice=match key {
                "s" if !ctrl && !ev.keystroke.modifiers.alt=>Some(code_core::external::Choice::SaveAndOpen),
                "o" if !ctrl && !ev.keystroke.modifiers.alt=>Some(code_core::external::Choice::OpenSavedVersion),
                "escape"=>Some(code_core::external::Choice::Cancel),
                _=>None,
            };
            if let Some(choice)=choice {self.choose_external(choice,cx);return;}
        }
        if matches!(self.modal,Some(Modal::StatusDetails {kind:Details::Reconcile,..})) {
            let plain=!(ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform || ev.keystroke.modifiers.alt);
            let choice=match key {
                "c" if plain=>Some(code_core::reconcile::Choice::Compare),
                "d" if plain=>Some(code_core::reconcile::Choice::TakeDisk),
                "s" if plain=>Some(code_core::reconcile::Choice::SaveDraft),
                "escape"=>Some(code_core::reconcile::Choice::KeepDraft),
                _=>None,
            };
            if let Some(choice)=choice {self.choose_reconcile(choice,cx);return;}
        }
        if matches!(self.modal,Some(Modal::StatusDetails {kind:Details::Replace,..})) {
            let plain=!(ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform || ev.keystroke.modifiers.alt);
            let choice=match key {"r" if plain=>Some(code_core::save::ReplaceChoice::Replace),"escape"=>Some(code_core::save::ReplaceChoice::Cancel),_=>None};
            if let Some(choice)=choice {self.choose_replace(choice,cx);return;}
        }
        if matches!(self.modal,Some(Modal::StatusDetails {kind:Details::Recovered,..})) {
            let plain=!(ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform || ev.keystroke.modifiers.alt);
            let choice=match key {"r" if plain=>Some(code_core::recovery::Choice::Restore),"d" if plain=>Some(code_core::recovery::Choice::Discard),"escape"=>Some(code_core::recovery::Choice::Later),_=>None};
            if let Some(choice)=choice {self.choose_recovery(choice,cx);return;}
        }
        if matches!(self.modal,Some(Modal::StatusDetails {kind:Details::Close,..})) {
            let ctrl=ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
            let choice=match key {"s" if !ctrl && !ev.keystroke.modifiers.alt=>Some(code_core::close::Choice::Save),"d" if !ctrl && !ev.keystroke.modifiers.alt=>Some(code_core::close::Choice::Discard),"escape"=>Some(code_core::close::Choice::Cancel),_=>None};
            if let Some(choice)=choice {self.choose_close(choice,cx);return;}
        }
        if matches!(self.modal,Some(Modal::StatusDetails {kind:Details::Change,..})) && !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !ev.keystroke.modifiers.alt {
            match key { "d" => return self.run(CommandId::ShowChangeDiff,window,cx), "r" => return self.run(CommandId::RevertChange,window,cx), _=>{} }
        }
        if matches!(self.modal, Some(Modal::StatusDetails { kind: Details::Install, .. })) && !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !ev.keystroke.modifiers.alt {
            match key {
                "s" | "enter" => { self.run(CommandId::StartServerInstall,window,cx); return; }
                "c" => { self.run(CommandId::CancelServerInstall,window,cx); return; }
                "d" => { self.run(CommandId::ServerDetails,window,cx); return; }
                _ => {}
            }
        }
        if let Some(Modal::StatusDetails { text, kind, .. }) = &self.modal {
            if *kind == Details::Sql && key == "m" && !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !ev.keystroke.modifiers.alt {
                self.modal = Some(Modal::CommandGroup { group: CommandGroup::Sql, at: 1 }); cx.notify(); return;
            }
            let commit = *kind == Details::Commit;
            let copy = key == "c" && (ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform);
            if copy && commit {
                self.run(CommandId::CopyRevision, window, cx);
            } else if copy {
                self.dispatch(Command::CopyText(text.clone()), cx);
            } else if matches!(key, "escape" | "enter" | "f2") {
                self.close_modal(cx);
            } else if let Some(l) = self.modal_layout() {
                let max = l.lines.len().saturating_sub(l.visible);
                let step = l.visible.saturating_sub(1).max(1);
                self.modal_scroll.set(match key {
                    "up" => l.first.saturating_sub(1),
                    "down" => (l.first + 1).min(max),
                    "pageup" => l.first.saturating_sub(step),
                    "pagedown" => (l.first + step).min(max),
                    "home" => 0,
                    "end" => max,
                    _ => l.first,
                });
            }
            return;
        }
        if let Some(Modal::Themes { ids, at }) = &mut self.modal {
            let last = ids.len().saturating_sub(1);
            *at = (*at).min(last);
            match key {
                "escape" => self.close_modal(cx),
                "enter" => self.modal_accept(window, cx),
                "up" | "down" | "home" | "end" | "pageup" | "pagedown" => {
                    *at = match key {
                        "up" => at.saturating_sub(1),
                        "down" => (*at + 1).min(last),
                        "home" => 0,
                        "end" => last,
                        "pageup" => at.saturating_sub(8),
                        "pagedown" => (*at + 8).min(last),
                        _ => *at,
                    };
                    let id = ids.get(*at).map(|(id, _)| id.clone());
                    self.dispatch(Command::PreviewTheme(id), cx);
                }
                _ => {}
            }
            return;
        }
        if let Some(Modal::CommandGroup { group, .. }) = &self.modal
            && matches!(key, "left" | "escape")
        {
            let parent = group.menu_parent();
            let at = self.session.menu(parent).iter().position(|row| row.target == MenuTarget::Group(*group)).unwrap_or(0);
            self.modal = Some(match parent { Some(group) => Modal::CommandGroup { group, at }, None => Modal::AppMenu { at } });
            return;
        }
        if key == "right" {
            let menu = match self.modal.as_ref() {
                Some(Modal::AppMenu { at }) => Some((None, *at)),
                Some(Modal::CommandGroup { group, at }) => Some((Some(*group), *at)),
                _ => None,
            };
            if let Some((group, at)) = menu {
                if self.session.menu(group).get(at).is_some_and(|row| matches!(row.target, MenuTarget::Group(_))) { self.modal_accept(window, cx); }
                return;
            }
        }
        if matches!(self.modal, Some(Modal::LanguageServer { .. })) && key == "c"
            && (ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform) && !ev.keystroke.modifiers.alt
        {
            self.run(CommandId::CopyServerDetails, window, cx);
            return;
        }
        if let Some(Modal::ServerRuntime { path, at }) = &mut self.modal {
            if ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform {
                if key == "v" && let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    path.push_str(text.trim()); *at = 0;
                } else if key == "a" { path.clear(); *at = 0; }
                return;
            }
            match key {
                "escape" => { self.close_modal(cx); return; }
                "backspace" => { path.pop(); *at = 0; return; }
                "enter" => { self.modal_accept(window, cx); return; }
                "up" | "down" | "pageup" | "pagedown" | "home" | "end" => {},
                _ => {
                    if let Some(text) = ev.keystroke.key_char.as_ref().filter(|t| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !t.chars().any(char::is_control)) { path.push_str(text); *at = 0; }
                    return;
                }
            }
        }
        // The Git Branches popup: typing filters, Up/Down move, Enter checks out (or opens a worktree), Right shows the
        // row's actions, Escape closes.
        // The Git prompts take a paste (a commit id, a branch name): its first line.
        let paste = (key == "v" && (ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform)).then(|| cx.read_from_clipboard().and_then(|c| c.text())).flatten()
            .map(|t| t.lines().next().unwrap_or_default().trim().to_string());
        // So does the Git Branches popup's filter (a commit id to compare with).
        if let (Some(text), Some(Modal::Branches { query, at })) = (&paste, &mut self.modal) {
            query.push_str(text);
            *at = 0;
            return cx.notify();
        }
        if let Some(Modal::Branches { query, at }) = self.modal.clone() {
            let rows = self.session.state().branch_rows(&query);
            let last = rows.len().saturating_sub(1);
            let at = at.min(last);
            let typed = ev.keystroke.key_char.clone().filter(|_| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform);
            let set = |query: String, at: usize| Some(Modal::Branches { query, at });
            match key {
                "escape" => self.close_modal(cx),
                "up" => self.modal = set(query, at.saturating_sub(1)),
                "down" => self.modal = set(query, (at + 1).min(last)),
                "home" => self.modal = set(query, 0),
                "end" => self.modal = set(query, last),
                "enter" => {
                    self.modal = set(query, at);
                    self.modal_accept(window, cx);
                }
                "right" => self.modal = match rows.get(at) {
                    Some(r) if !self.session.branch_actions(&r.target).is_empty() => Some(Modal::BranchMenu { target: r.target.clone(), label: r.label.clone(), at: 0, query, row: at }),
                    _ => set(query, at),
                },
                "backspace" => self.modal = set(query.chars().take(query.chars().count().saturating_sub(1)).collect(), 0),
                _ => {
                    if let Some(text) = typed.filter(|t| !t.chars().any(char::is_control)) {
                        self.modal = set(query + &text, 0);
                    }
                }
            }
            return cx.notify();
        }
        if let Some(Modal::BranchMenu { target, label, at, query, row }) = self.modal.clone() {
            let last = self.session.branch_actions(&target).len().saturating_sub(1);
            match key {
                "escape" | "left" => self.modal = Some(Modal::Branches { query, at: row }),
                "up" => self.modal = Some(Modal::BranchMenu { target, label, at: at.saturating_sub(1), query, row }),
                "down" => self.modal = Some(Modal::BranchMenu { target, label, at: (at + 1).min(last), query, row }),
                "enter" | "right" => self.modal_accept(window, cx),
                _ => {}
            }
            return cx.notify();
        }
        if let (Some(text), Some(Modal::NewBranch { name: input, error, .. } | Modal::Revision { input, error, .. })) = (&paste, &mut self.modal) {
            input.push_str(text);
            *error = None;
            return cx.notify();
        }
        // A new branch's name: typing edits it, Tab turns Check out on or off, Enter makes it, Escape cancels.
        if let Some(Modal::NewBranch { from, name, checkout, error }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "backspace" => {
                    name.pop();
                    *error = None;
                }
                "tab" => *checkout = !*checkout,
                "enter" => match self.session.validate_branch_name(name) {
                    Ok(()) => {
                        let (name, from, checkout) = (name.trim().to_string(), from.clone(), *checkout);
                        self.modal = None;
                        self.dispatch(Command::NewBranch { name, from, checkout }, cx);
                    }
                    Err(e) => *error = Some(e),
                },
                _ => {
                    if let Some(text) = ev.keystroke.key_char.clone().filter(|t| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !t.chars().any(char::is_control)) {
                        name.push_str(&text);
                        *error = None;
                    }
                }
            }
            return cx.notify();
        }
        if let (Some(text), Some(Modal::Create { name: input, error, .. } | Modal::Relocate { input, error, .. })) = (&paste, &mut self.modal) {
            input.push_str(text);
            *error = None;
            return cx.notify();
        }
        // A delete waits for its confirmation: Enter deletes (unless the core refuses), Escape cancels, nothing else acts.
        if let Some(Modal::ConfirmDelete { removal, from, error }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "enter" => match self.session.delete_refusal(*removal, from) {
                    Some(why) => *error = Some(why),
                    None => {
                        let (removal, from) = (*removal, from.clone());
                        self.modal = None;
                        self.dispatch(Command::DeleteEntry { removal, from: from.into() }, cx);
                    }
                },
                _ => {}
            }
            return cx.notify();
        }
        if let Some(Modal::PasteEntry { request, name, selected, error }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "a" if ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform => *selected = true,
                "backspace" => { if *selected { name.clear(); } else { name.pop(); } *selected = false; *error = None; },
                "enter" => match self.session.paste_entry_refusal(request, name.trim()) {
                    Some(why) => *error = Some(why),
                    None => {
                        let (request, name) = (request.clone(), name.trim().to_string());
                        self.modal = None;
                        self.dispatch(Command::PasteExplorerEntry { request, name }, cx);
                    }
                },
                _ => {
                    let text = paste.clone().or_else(|| ev.keystroke.key_char.clone().filter(|t| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !t.chars().any(char::is_control)));
                    if let Some(text) = text { if *selected { name.clear(); } *selected = false; name.push_str(&text); *error = None; }
                }
            }
            return cx.notify();
        }
        // An entry's new name or folder: typing edits it, Enter moves it, Escape cancels.
        if let Some(Modal::Relocate { kind, from, input, error }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "backspace" => {
                    input.pop();
                    *error = None;
                }
                "enter" => match self.session.relocate_refusal(*kind, from, input) {
                    Some(why) => *error = Some(why),
                    None => {
                        let (kind, from, input) = (*kind, from.clone(), input.clone());
                        self.modal = None;
                        self.dispatch(Command::RelocateEntry { kind, from: from.into(), input }, cx);
                    }
                },
                _ => {
                    if let Some(text) = ev.keystroke.key_char.clone().filter(|t| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !t.chars().any(char::is_control)) {
                        input.push_str(&text);
                        *error = None;
                    }
                }
            }
            return cx.notify();
        }
        // A new file or folder's name: typing edits it, Enter creates it, Escape cancels.
        if let Some(Modal::Create { kind, dir, name, error }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "backspace" => {
                    name.pop();
                    *error = None;
                }
                "enter" => match self.session.create_refusal(dir, name.trim()) {
                    Some(why) => *error = Some(why),
                    None => {
                        let (kind, dir, name) = (*kind, dir.clone(), name.trim().to_string());
                        self.modal = None;
                        // A created file opens; the keys go to it, not the explorer.
                        if kind == code_core::create::Kind::File { self.tree_focus = false; self.results_focus = false; }
                        self.dispatch(Command::CreateEntry { kind, dir: dir.into(), name }, cx);
                    }
                },
                _ => {
                    if let Some(text) = ev.keystroke.key_char.clone().filter(|t| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !t.chars().any(char::is_control)) {
                        name.push_str(&text);
                        *error = None;
                    }
                }
            }
            return cx.notify();
        }
        if let Some(Modal::Revision { input, error }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "backspace" => {
                    input.pop();
                    *error = None;
                }
                "enter" => match self.session.validate_revision(input) {
                    Ok(_) => {
                        let rev = input.trim().to_string();
                        self.modal = None;
                        self.dispatch(Command::Checkout(lenz_content::branches::Target::Detached(rev)), cx);
                    }
                    Err(e) => *error = Some(e),
                },
                _ => {
                    if let Some(text) = ev.keystroke.key_char.clone().filter(|t| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !t.chars().any(char::is_control)) {
                        input.push_str(&text);
                        *error = None;
                    }
                }
            }
            return cx.notify();
        }
        // A list of places: typing filters, Up/Down move, Enter goes there, Delete forgets, Escape back to the text.
        if let Some(Modal::Places { list, query, at }) = self.modal.clone() {
            let rows = self.session.state().place_rows(list, &query);
            let last = rows.len().saturating_sub(1);
            let at = at.min(last);
            let typed = ev.keystroke.key_char.clone().filter(|_| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform);
            let set = |query: String, at: usize| Some(Modal::Places { list, query, at });
            match key {
                "escape" => self.modal = None,
                "up" => self.modal = set(query, at.saturating_sub(1)),
                "down" => self.modal = set(query, (at + 1).min(last)),
                "home" => self.modal = set(query, 0),
                "end" => self.modal = set(query, last),
                "enter" => {
                    self.modal = set(query, at);
                    self.modal_accept(window, cx);
                }
                "delete" => {
                    if let Some(row) = rows.into_iter().nth(at) {
                        self.dispatch(Command::ForgetPlace(row.place), cx);
                        let left = self.session.state().place_rows(list, &query).len();
                        self.modal = set(query, at.min(left.saturating_sub(1)));
                    }
                }
                "backspace" => self.modal = set(query.chars().take(query.chars().count().saturating_sub(1)).collect(), 0),
                _ => {
                    if let Some(text) = typed {
                        self.modal = set(query + &text, 0);
                    }
                }
            }
            cx.notify();
            return;
        }
        if matches!(self.modal, Some(Modal::Mnemonic)) {
            if key == "escape" {
                self.modal = None;
            } else if let Some(c) = ev.keystroke.key_char.as_deref().and_then(|s| s.chars().next()).filter(|c| c.is_ascii_alphanumeric()) {
                self.modal = None;
                self.dispatch(Command::SetMnemonic(c.to_ascii_uppercase()), cx);
            }
            cx.notify();
            return;
        }
        // A crumb's list: typing filters, Up/Down move, Enter goes there, Escape back to the bar (or the text).
        if let Some(Modal::Siblings { pane, crumb, query, at }) = self.modal.clone() {
            let items = self.sibling_items(pane, crumb, &query);
            let at = at.min(items.len().saturating_sub(1));
            let typed = ev.keystroke.key_char.clone().filter(|_| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform);
            let set = |query: String, at: usize| Some(Modal::Siblings { pane, crumb, query, at });
            match key {
                "escape" => self.modal = None,
                "up" => self.modal = set(query, at.saturating_sub(1)),
                "down" => self.modal = set(query, (at + 1).min(items.len().saturating_sub(1))),
                "enter" => {
                    self.modal = set(query, at);
                    self.modal_accept(window, cx);
                }
                "backspace" => self.modal = set(query.chars().take(query.chars().count().saturating_sub(1)).collect(), 0),
                _ => {
                    if let Some(text) = typed {
                        self.modal = set(query + &text, 0);
                    }
                }
            }
            return;
        }
        // The context menu: Up/Down step over separators, Enter runs, Escape closes.
        if let Some(Modal::Context { entries, at, .. }) = &mut self.modal {
            let commands: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| matches!(e, MenuEntry::Command(_))).map(|(i, _)| i).collect();
            let here = commands.iter().position(|i| i == at).unwrap_or(0);
            match key {
                "escape" => { self.modal = None; self.terminal_ui.path_context = None; },
                "home" => *at = commands.first().copied().unwrap_or(0),
                "end" => *at = commands.last().copied().unwrap_or(0),
                "up" => *at = commands[here.saturating_sub(1)],
                "down" => *at = commands[(here + 1).min(commands.len() - 1)],
                "enter" => {
                    if let Some(MenuEntry::Command(id)) = entries.get(*at).copied() {
                        self.modal = None;
                        self.run(id, window, cx);
                    }
                }
                _ => {}
            }
            self.reveal_context();
            return;
        }
        if let Some(Modal::Declarations { query, semantic }) = &mut self.modal {
            let typed = ev.keystroke.key_char.clone().filter(|_| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform);
            match key {
                // Syntactic declarations or the language servers' symbols: always labelled for what they are.
                "tab" => *semantic = !*semantic && self.session.disabled(CommandId::Symbols).is_none(),
                "escape" => {
                    self.modal = None;
                    return;
                }
                "enter" => {
                    // Keys go to the results, where arrows preview and Enter opens.
                    self.modal = None;
                    self.results_focus = self.session.state().results().is_some();
                    self.tree_focus &= !self.results_focus;
                    return;
                }
                "backspace" => {
                    query.pop();
                }
                _ => match typed {
                    Some(text) => query.push_str(&text),
                    None => return,
                },
            }
            let (q, semantic) = (query.clone(), *semantic);
            self.results_scroll = 0;
            self.dispatch(if semantic { Command::Symbols(q) } else { Command::FindDeclarations(q) }, cx);
            return;
        }
        if let Some(Modal::GoTo { input, error }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "backspace" => {
                    input.pop();
                }
                "enter" => match lenz_content::goto::parse_line_column(input) {
                    Ok(at) => {
                        self.modal = None;
                        self.dispatch(Command::GoToLine(at), cx);
                    }
                    Err(e) => *error = Some(e.to_string()),
                },
                _ => {
                    if let Some(text) = ev.keystroke.key_char.clone().filter(|t| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !t.chars().any(char::is_control)) {
                        input.push_str(&text);
                    }
                }
            }
            return;
        }
        let metadata_input = matches!(self.modal, Some(Modal::SqlMetadata { .. }));
        if let Some(Modal::Root { path } | Modal::SqlMetadata { path }) = &mut self.modal {
            match key {
                "escape" => self.modal = None,
                "backspace" => {
                    path.pop();
                }
                "enter" => {
                    let command = if metadata_input { Command::ConfigureSqlMetadata(path.clone()) } else { Command::SetWorkspaceRoot(Some(PathBuf::from(path.trim())).filter(|p| !p.as_os_str().is_empty())) };
                    self.modal = None;
                    self.dispatch(command, cx);
                }
                _ => {
                    if let Some(text) = ev.keystroke.key_char.clone().filter(|_| !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform) {
                        path.push_str(&text);
                    }
                }
            }
            return;
        }
        if matches!(self.modal, Some(Modal::Search { .. })) && ev.keystroke.modifiers.alt && let Some(option) = match key { "c" => Some(FindOption::Case), "w" => Some(FindOption::Word), "r" => Some(FindOption::Regex), _ => None } {
            self.dispatch(Command::ToggleFindOption(option), cx);
            return;
        }
        let refused = match &self.modal {
            Some(Modal::Search { query, .. }) => self.session.search_refusal(query),
            _ => None,
        };
        if let Some(Modal::Search { query, files, in_files, project, selected }) = &mut self.modal {
            let (mods, empty) = (ev.keystroke.modifiers, query.is_empty());
            let field = if *in_files { &mut *files } else { &mut *query };
            match key {
                "escape" => self.modal = None,
                "enter" if refused.is_some() => {}
                "a" if mods.control || mods.platform => *selected = !field.is_empty(),
                "home" if mods.shift => *selected = !field.is_empty(),
                "left" | "right" | "home" | "end" => *selected = false,
                "tab" if *project => { *in_files = !*in_files; *selected = false; }
                "backspace" => {
                    if std::mem::take(selected) { field.clear() } else if mods.control || mods.alt { code_core::search::drop_word(field) } else { field.pop(); }
                }
                "enter" if !empty => {
                    let scope = match (*project, files.trim()) {
                        (false, _) => SearchIn::ActiveFile,
                        (true, "") => SearchIn::Project,
                        (true, pattern) => SearchIn::Matching(pattern.to_string()),
                    };
                    let query = query.clone();
                    self.modal = None;
                    self.results_scroll = 0;
                    self.dispatch(Command::Search { query, scope }, cx);
                    // Keys go to the results only when a search actually started.
                    self.results_focus = self.session.state().results().is_some();
                    self.tree_focus &= !self.results_focus;
                }
                _ => {
                    if let Some(text) = ev.keystroke.key_char.clone().filter(|_| !mods.control && !mods.platform) {
                        if std::mem::take(selected) { field.clear(); }
                        field.push_str(&text);
                    }
                }
            }
            return;
        }
        let len = match &self.modal {
            // Handled above.
            Some(Modal::StatusDetails { .. } | Modal::Navigation { .. } | Modal::Search { .. } | Modal::Declarations { .. } | Modal::Root { .. } | Modal::SqlMetadata { .. } | Modal::Context { .. } | Modal::GoTo { .. } | Modal::Siblings { .. } | Modal::Places { .. } | Modal::Mnemonic | Modal::Branches { .. } | Modal::BranchMenu { .. } | Modal::NewBranch { .. } | Modal::Revision { .. } | Modal::Create { .. } | Modal::PasteEntry { .. } | Modal::SaveAs { .. } | Modal::NewScratch { .. } | Modal::RenameTerminal { .. } | Modal::Relocate { .. } | Modal::ConfirmDelete { .. } | Modal::Keymap { .. }) => 0,
            Some(Modal::ServerRuntime { .. }) => self.modal_rows().len().saturating_sub(FINDER_HEADER_ROWS),
            Some(Modal::LanguageServer { .. }) => self.server_rows().len(),
            Some(Modal::Commands { .. }) => CommandId::ALL.len(),
            Some(Modal::AppMenu { .. }) => self.session.menu(None).len(),
            Some(Modal::CommandGroup { group, .. }) => self.session.menu(Some(*group)).len(),
            Some(Modal::Themes { ids, .. }) => ids.len(),
            Some(Modal::Recent { .. }) => self.session.state().recent().len(),
            Some(Modal::ScratchFiles { files, .. }) => files.len().max(1),
            None => 0,
        };
        // A menu's separator line is passed over in the direction of the move.
        let separators: Vec<bool> = match &self.modal {
            Some(Modal::AppMenu { .. }) => self.session.menu(None),
            Some(Modal::CommandGroup { group, .. }) => self.session.menu(Some(*group)),
            _ => Vec::new(),
        }.iter().map(|row| row.target == MenuTarget::Separator).collect();
        let Some(modal) = &mut self.modal else { return };
        let (Modal::Commands { at } | Modal::AppMenu { at } | Modal::CommandGroup { at, .. } | Modal::LanguageServer { at } | Modal::ServerRuntime { at, .. } | Modal::Themes { at, .. } | Modal::Recent { at } | Modal::ScratchFiles { at, .. }) = modal else { return };
        // The finder's results may have changed since the highlight moved.
        *at = (*at).min(len.saturating_sub(1));
        let past = |to: usize, back: bool| match separators.get(to) {
            Some(true) if back => to.saturating_sub(1),
            Some(true) => (to + 1).min(len.saturating_sub(1)),
            _ => to,
        };
        match key {
            "escape" => self.close_modal(cx),
            "up" => *at = past(at.saturating_sub(1), true),
            "down" => *at = past((*at + 1).min(len.saturating_sub(1)), false),
            "pageup" => *at = past(at.saturating_sub(10), true),
            "pagedown" => *at = past((*at + 10).min(len.saturating_sub(1)), false),
            "enter" => self.modal_accept(window, cx),
            "home" => *at = 0,
            "end" => *at = len.saturating_sub(1),
            _ => {}
        }
    }

    /// Keys while the results panel has focus: rows preview, Enter opens, Escape dismisses. `false` lets the key fall
    /// through.
    fn results_key(&mut self, key: &str, ctrl: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(r) = self.session.state().results() else {
            self.results_focus = false;
            return false;
        };
        // A line's results share its row: moves start from the line's first.
        let at = r.selected().map_or(0, |i| r.line_hits(i).start);
        // Moves go between the results shown (not folded away), in order; from a folded selection, to its neighbours.
        let shown: Vec<usize> = r.rows().iter().filter_map(|row| match row { Row::Hit(i) => Some(*i), _ => None }).collect();
        let (len, page) = (shown.len(), self.panel_list_rows.max(1));
        let here = shown.partition_point(|&i| i < at);
        let on = shown.get(here) == Some(&at) && r.selected().is_some();
        let pick = |k: usize| shown.get(k.min(len.saturating_sub(1))).copied();
        // A call hierarchy: Right unfolds (or steps into the level), Left folds (or steps out), Tab turns it around.
        if let Some(call) = r.call(at).filter(|_| on) {
            let parent = || (0..at).rev().find(|&i| r.call(i).is_some_and(|c| c.depth < call.depth));
            match (key, ctrl) {
                ("right", false) if matches!(call.fold, Fold::Closed | Fold::Failed(_)) => {
                    self.dispatch(Command::ToggleCall(at), cx);
                    return true;
                }
                ("right", false) if call.fold == Fold::Open => {
                    self.dispatch(Command::SelectResult(at + 1), cx);
                    self.reveal_result(at + 1);
                    return true;
                }
                ("left", false) if call.fold == Fold::Open => {
                    self.dispatch(Command::ToggleCall(at), cx);
                    return true;
                }
                ("left", false) => {
                    if let Some(p) = parent() {
                        self.dispatch(Command::SelectResult(p), cx);
                        self.reveal_result(p);
                    }
                    return true;
                }
                ("tab", false) => {
                    let other = match r.hierarchy_direction().map(Direction::other) {
                        Some(Direction::Callers) => CommandId::CallHierarchyCallers,
                        Some(Direction::Supertypes) => CommandId::TypeHierarchySupertypes,
                        Some(Direction::Subtypes) => CommandId::TypeHierarchySubtypes,
                        Some(Direction::Callees) | None => CommandId::CallHierarchyCallees,
                    };
                    self.run(other, window, cx);
                    return true;
                }
                _ => {}
            }
        }
        let to = match (key, ctrl) {
            ("up", false) => pick(here.saturating_sub(1)),
            ("down", false) => pick(if on { here + 1 } else { here }),
            ("pageup", false) => pick(here.saturating_sub(page)),
            ("pagedown", false) => pick(here + page),
            ("home", false) => pick(0),
            ("end", false) => pick(len.saturating_sub(1)),
            // A project's tree: Left folds the selection's file, then each folder out; Right unfolds them.
            ("left", false) if r.grouped() && r.selected().is_some() => {
                let unfolded = r.groups_of(at).into_iter().rev().find(|g| !r.is_folded(g));
                if let Some(g) = unfolded {
                    self.dispatch(Command::ToggleResultGroup(g), cx);
                }
                return true;
            }
            ("right", false) if r.grouped() && !on && r.selected().is_some() => {
                self.dispatch(Command::RevealResult(at), cx);
                self.reveal_result(at);
                return true;
            }
            ("enter", false) => {
                self.dispatch(Command::OpenResult(at), cx);
                return true;
            }
            // A pending definition is cancelled first; the next Escape hides the results (Show / Hide Results brings
            // them back).
            ("escape", false) if self.session.state().asking().is_some() => {
                self.dispatch(Command::CancelSemantic, cx);
                return true;
            }
            // A project search still running is stopped first, keeping what it found.
            ("escape", false) if self.session.disabled(CommandId::StopSearch).is_none() => {
                self.run(CommandId::StopSearch, window, cx);
                return true;
            }
            ("escape", false) => {
                self.run(CommandId::ToggleResults, window, cx);
                return true;
            }
            // An outline's two views: syntactic declarations, or the language server's symbols.
            ("tab", false) if r.results_kind() == code_core::search::ResultsKind::Outline && self.session.disabled(CommandId::FileSymbols).is_none() => {
                self.run(CommandId::FileSymbols, window, cx);
                return true;
            }
            ("tab", false) if r.results_kind() == code_core::search::ResultsKind::FileSymbols => {
                self.run(CommandId::Outline, window, cx);
                return true;
            }
            _ => return false,
        };
        if let Some(to) = to {
            self.dispatch(Command::SelectResult(to), cx);
            self.reveal_result(to);
        }
        true
    }

    /// Scroll the list so result `i`'s row is in view (unless it is folded away).
    fn reveal_result(&mut self, i: usize) {
        let Some(i) = self.session.state().results().and_then(|r| r.row_of_hit(i)) else { return };
        let rows = self.panel_list_rows.max(1);
        if i < self.results_scroll {
            self.results_scroll = i;
        } else if i >= self.results_scroll + rows {
            self.results_scroll = i + 1 - rows;
        }
    }

    /// The visible result rows and the preview's rows: the selected match's line with context, the match emphasized
    /// over the file's syntax colours. A project's text matches show as a tree: folders, files, then each match's
    /// line number and text.
    pub fn panel_contents(&mut self, list_rows: usize, preview_rows: usize) -> (Vec<ResultRow>, Vec<TextRow>) {
        let Some(r) = self.session.state().results() else { return (Vec::new(), Vec::new()) };
        let palette = self.session.state().palette();
        // Keep the scroll within the rows (they may have been replaced, grown or folded).
        self.results_scroll = self.results_scroll.min(r.rows().len().saturating_sub(list_rows.max(1)));
        let (selected, grouped) = (r.selected(), r.grouped());
        let (match_bg, match_fg) = match_colors(palette);
        // Lines shown without colours yet: the core makes them off the UI thread.
        let mut uncoloured = Vec::new();
        let list: Vec<ResultRow> = r
            .rows()
            .iter()
            .enumerate()
            .skip(self.results_scroll)
            .take(list_rows)
            .map(|(at, row)| {
                let depth = r.row_depth(at);
                // A folder or file of the tree: its icon and name, then how many results it holds, dimmed.
                let node = |label: &str, icon: icons::Icon, open: bool, count: usize| {
                    let count = format!("  {count}");
                    let fg = vec![(label.len()..label.len() + count.len(), palette.dim)];
                    ResultRow { label: format!("{label}{count}"), highlights: Vec::new(), selected: false, stale: false, fg, depth, node: Some((icon, open)), hit: None }
                };
                let i = match row {
                    Row::Dir { label, count, open, .. } => return node(&format!("{label}/"), icons::folder(*open), *open, *count),
                    Row::File { path, count, open } => {
                        let (name, _) = r.file_label(path);
                        return node(name, icons::file(name), *open, *count);
                    }
                    Row::Hit(i) => *i,
                };
                let h = &r.hits()[i];
                let stale = if r.is_stale(i) { "  (changed since)" } else { "" };
                let line = r.line_hits(i);
                let base = ResultRow { label: String::new(), highlights: Vec::new(), selected: selected.is_some_and(|s| line.contains(&s)), stale: r.is_stale(i), fg: Vec::new(), depth: 0, node: None, hit: Some(i) };
                if let Some(call) = r.call(i) {
                    // A call: indented by depth, its fold mark, kind and name, then the call site.
                    let mark = match call.fold {
                        Fold::Open => "▾",
                        Fold::Closed | Fold::Failed(_) => "▸",
                        Fold::Loading => "…",
                        Fold::Recursive => "↻",
                        Fold::Leaf => "·",
                    };
                    let kind = if call.label.is_empty() { String::new() } else { format!("{} ", call.label) };
                    let head = format!("{}{mark} {kind}", "  ".repeat(call.depth as usize));
                    let path = r.shown_path(&h.path);
                    let count = if call.calls > 1 { format!(" ×{}", call.calls) } else { String::new() };
                    let mut tail = format!("{count}  {path}:{}", h.line + 1);
                    match &call.fold {
                        Fold::Failed(why) => tail += &format!("  failed: {why}"),
                        Fold::Loading => tail += "  loading…",
                        _ if call.more > 0 => tail += &format!("  (+{} more not shown)", call.more),
                        // An empty type level says so at its root.
                        Fold::Leaf if call.depth == 0 => match r.hierarchy_direction() {
                            Some(Direction::Subtypes) => tail += "  — No subtypes",
                            Some(Direction::Supertypes) => tail += "  — No supertypes",
                            Some(Direction::Callers | Direction::Callees) | None => {}
                        },
                        _ => {}
                    }
                    let name = head.len()..head.len() + call.name.len();
                    let fg = vec![(0..head.len(), palette.dim), (name.end..name.end + tail.len(), palette.dim)];
                    return ResultRow { label: format!("{head}{}{tail}{stale}", call.name), highlights: vec![name], fg, ..base };
                }
                if let Some((kind, depth)) = h.decl {
                    // A declaration: its kind and name (indented by nesting in an outline), then where it is.
                    let outline = matches!(r.results_kind(), code_core::search::ResultsKind::Outline | code_core::search::ResultsKind::FileSymbols);
                    let indent = if outline { "  ".repeat(depth as usize) } else { String::new() };
                    // An impl block's name already starts with `impl`.
                    let head = match kind {
                        lenz_content::highlight::DeclKind::Impl => indent,
                        _ => format!("{indent}{} ", code_core::semantics::kind_label(kind, &h.path)),
                    };
                    let place = match outline {
                        true => format!("  :{}", h.line + 1),
                        _ => format!("  {}:{}", r.shown_path(&h.path), h.line + 1),
                    };
                    let (name, _) = display::display_line(&h.text);
                    let hl = head.len()..head.len() + name.len();
                    return ResultRow { label: format!("{head}{name}{place}{stale}"), highlights: vec![hl], ..base };
                }
                // A line: under its file its number then its text without the indentation, every result on it
                // emphasized; else its place first.
                let prefix = if grouped { format!("{:>4}  ", h.line + 1) } else { format!("{}: ", r.location(i).unwrap_or_default()) };
                let (shown, map) = display::display_line(&h.text);
                let lead = map[h.indent().min(map.len() - 1)];
                // A long line starts a little before its match, after `…`, so the match is in view.
                let room = (((self.panel_list_w.min(self.width - self.sidebar_w) - 8.0) / self.char_w).max(0.0) as usize)
                    .saturating_sub(2 * depth + prefix.len() + stale.len());
                let range = map[h.range.start.min(map.len() - 1)]..map[h.range.end.min(map.len() - 1)];
                let start = if h.range.end <= h.text.len() { code_core::search::clip_start(&shown, lead, range, room) } else { lead };
                let ellipsis = if start > lead { "…" } else { "" };
                let head = prefix.len() + ellipsis.len();
                let at = |b: usize| (head + map[b.min(map.len() - 1)]).saturating_sub(start).max(head);
                let hls: Vec<_> = r.hits()[line].iter().filter(|h| h.range.end <= h.text.len()).map(|h| at(h.range.start)..at(h.range.end)).collect();
                let spans = r.line_spans(i).unwrap_or_else(|| {
                    uncoloured.push(i);
                    &[]
                });
                let fg = hls.iter().map(|m| (m.clone(), match_fg))
                    .chain([(0..head, palette.dim)])
                    .chain(spans.iter().map(|(sr, c)| (at(sr.start)..at(sr.end), palette.syntax_color(*c))))
                    .collect();
                ResultRow { label: format!("{prefix}{ellipsis}{}{stale}", &shown[start..]), highlights: hls, fg, depth, ..base }
            })
            .collect();
        let preview = match (r.preview(), selected.and_then(|i| r.hits().get(i))) {
            (Some(p), Some(hit)) if preview_rows > 0 => {
                let doc = &p.doc;
                let first = hit.line.saturating_sub(preview_rows / 2).min(doc.line_count().saturating_sub(preview_rows));
                let end = (first + preview_rows).min(doc.line_count());
                let spans = p.highlights().map(|h| h.spans(first..end)).unwrap_or_default();
                (first..end)
                    .enumerate()
                    .map(|(n, l)| {
                        let (text, map) = display::display_line(doc.line(l));
                        let at = |b: usize| map[b.min(map.len() - 1)];
                        // Emphasize only where the line still reads as it did when searched.
                        let same = l == hit.line
                            && match hit.decl {
                                Some(_) => doc.line(l).get(hit.range.clone()) == Some(&*hit.text),
                                None => doc.line(l).chars().zip(hit.text.chars()).all(|(a, b)| a == b),
                            };
                        let found = same.then(|| at(hit.range.start)..at(hit.range.end));
                        let fg = found.iter().map(|m| (m.clone(), match_fg))
                            .chain(spans.get(n).into_iter().flatten().map(|(sr, c)| (at(sr.start)..at(sr.end), palette.syntax_color(*c))))
                            .collect();
                        let bg = found.into_iter().map(|m| (m, match_bg)).collect();
                        TextRow { y: n as f32, number: l as u32 + 1, text, fg, bg, selected: None, caret: None, extra: Vec::new() }
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        if !uncoloured.is_empty() {
            self.session.dispatch(Command::ColourResults(uncoloured));
        }
        (list, preview)
    }

    /// The heading's buttons (Stop, Rerun, Collapse or Expand, Hide, ×), right-aligned in the panel's first row.
    pub fn results_buttons(&self) -> Vec<(Bounds<Pixels>, CommandId, &'static str)> {
        if self.panel_top == f32::MAX { return Vec::new(); }
        let mut right = self.width - 4.0;
        let mut buttons: Vec<_> = self.session.results_controls().into_iter().rev().map(|(id, label)| {
            let w = (lenz_content::wrap::width(label) + 2) as f32 * self.char_w;
            right -= w;
            (Bounds { origin: gpui::point(px(right), px(self.panel_top)), size: gpui::size(px(w), px(line_h())) }, id, label)
        }).collect();
        buttons.reverse();
        buttons
    }

    /// The panel's heading as one line: what was searched, how far it got and its limits, and the keys.
    pub fn results_status(&self) -> Option<String> {
        let h = self.session.results_header()?;
        Some([h.title, h.detail, h.hint].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" — "))
    }

    /// Speed-search input stays in the explorer before editor key bindings can use it.
    fn explorer_search_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        use code_core::session::ExplorerSearchInput as Search;
        let k = &ev.keystroke;
        if !self.tree_focus || k.modifiers.control || k.modifiers.platform || k.modifiers.alt { return false; }
        let active = !self.session.state().explorer_search().query().is_empty();
        let input = match k.key.as_str() {
            "escape" if active => Search::Clear,
            "backspace" if active => Search::Backspace,
            "up" if active => Search::Step { forward: false },
            "down" if active => Search::Step { forward: true },
            "enter" if active => Search::Activate,
            "space" if !active => return false,
            _ => match k.key_char.as_ref().filter(|text| !text.is_empty() && !text.chars().any(char::is_control)) {
                Some(text) => Search::Text(text.clone()),
                None => return false,
            },
        };
        self.remember_explorer();
        self.dispatch(Command::ExplorerSearch(input), cx);
        true
    }

    /// Keys while the project tree has focus; `false` lets the key fall through to the text.
    fn tree_key(&mut self, key: &str, ctrl: bool, cx: &mut Context<Self>) -> bool {
        let (len, at) = (self.tree_len(), self.tree_at());
        let expanded = self.session.state().tree_rows().nth(at).and_then(|r| r.dir);
        match (key, ctrl) {
            ("up", false) => self.tree_move(at.saturating_sub(1)),
            ("down", false) => self.tree_move(at + 1),
            ("home", false) => self.tree_move(0),
            ("end", false) => self.tree_move(len.saturating_sub(1)),
            ("enter", false) => self.activate_row(at, false, cx),
            ("right", false) if expanded == Some(false) => self.activate_row(at, false, cx),
            ("left", false) if expanded == Some(true) => self.activate_row(at, false, cx),
            ("escape", false) if !self.session.state().tree_marks().is_empty() => self.dispatch(Command::ClearTreeMarks, cx),
            // Space marks the highlighted entry for Compare Selected (Ctrl+D), as in the terminal.
            ("space", false) => {
                let path = self.session.state().tree_rows().nth(at).map(|r| r.path.clone());
                if let Some(path) = path {
                    self.dispatch(Command::MarkTreeRow { path, selected: None }, cx);
                }
            }
            ("escape", false) => self.tree_focus = false,
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Extend the selection by `motion` and scroll so its head stays visible.
    fn extend(&mut self, motion: Motion, cx: &mut Context<Self>) {
        self.dispatch(Command::Extend(motion), cx);
    }

    fn on_wheel(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.double_shift.interrupt();
        if let Some(Modal::Navigation { at }) = self.modal {
            let len = self.session.state().navigation().map_or(0, |n| n.rows.len());
            let visible = self.navigation_visible(len);
            self.modal_first(at, at..at.saturating_add(1), len, visible);
            let delta = match ev.delta { ScrollDelta::Pixels(p) => -f32::from(p.y) / (line_h() * 2.0), ScrollDelta::Lines(l) => -l.y * 3.0 };
            self.scroll_modal(delta, len.saturating_sub(visible));
            return cx.notify();
        }
        if let Some(Modal::Context { scroll, .. }) = &mut self.modal {
            let delta = match ev.delta { ScrollDelta::Pixels(p) => -f32::from(p.y), ScrollDelta::Lines(l) => -l.y * line_h() };
            *scroll += delta;
            self.clamp_context();
            return cx.notify();
        }
        if self.modal.is_some() {
            if let Some(layout) = self.form_layout() {
                let delta = match ev.delta { ScrollDelta::Pixels(p) => -f32::from(p.y) / line_h(), ScrollDelta::Lines(l) => -l.y * 3.0 };
                self.scroll_modal(delta, layout.max_scroll);
            } else if let Some(layout) = self.modal_layout() {
                let delta = match ev.delta { ScrollDelta::Pixels(p) => -f32::from(p.y) / line_h(), ScrollDelta::Lines(l) => -l.y * 3.0 };
                self.scroll_modal(delta, layout.lines.len().saturating_sub(layout.visible));
            }
            return cx.notify();
        }
        if self.terminal_wheel(ev, cx) { return; }
        if self.csv_wheel(ev, cx) { return; }
        self.link = None; // the text moves under the pointer; the next move finds it again
        // Over the documentation popup the wheel scrolls it; elsewhere it closes it.
        if self.session.state().completion().is_some() && self.completion_doc_drawn.is_some_and(|b|b.contains(&ev.position)) {
            if let Some(d)=&mut self.completion_doc {
            let dy = match ev.delta { ScrollDelta::Pixels(p) => -f32::from(p.y), ScrollDelta::Lines(l) => -l.y * 3.0 * line_h() };
            d.top = (d.top + dy).clamp(0.0, (d.layout.height - f32::from(d.bounds.size.height)).max(0.0));
            }
            return cx.notify();
        }
        if let Some(d) = self.doc_popup.as_mut().filter(|d| d.bounds.contains(&ev.position)) {
            let dy = match ev.delta {
                ScrollDelta::Pixels(p) => -f32::from(p.y),
                ScrollDelta::Lines(l) => -l.y * 3.0 * line_h(),
            };
            d.top = (d.top + dy).clamp(0.0, (d.layout.height - f32::from(d.bounds.size.height)).max(0.0));
            return cx.notify();
        }
        if self.session.state().documentation().is_some() {
            self.dispatch(Command::CloseDocumentation, cx);
        }
        if let Some(l) = self.activity_layout() && l.bounds.contains(&ev.position) {
            let delta = match ev.delta { ScrollDelta::Pixels(p) => -f32::from(p.y) / line_h(), ScrollDelta::Lines(p) => -p.y * 3.0 };
            let maximum = l.rows.len().saturating_sub(l.visible);
            self.log_scroll = (self.log_scroll as f32 - delta).clamp(0.0, maximum as f32) as usize;
            return cx.notify();
        }
        if f32::from(ev.position.y) >= self.body_bottom() { return; }
        if let Some(pane) = self.panes.iter().find(|g| (g.bar.0..g.bar.1).contains(&f32::from(ev.position.x))
            && (g.bar.2..g.bar.2 + line_h()).contains(&f32::from(ev.position.y))).map(|g| g.pane) {
            let delta = match ev.delta {
                ScrollDelta::Pixels(p) => gpui::point(-f32::from(p.x), -f32::from(p.y)),
                ScrollDelta::Lines(p) => gpui::point(-p.x * line_h() * 3.0, -p.y * line_h() * 3.0),
            };
            let delta = if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y };
            if let Some(scroll) = self.tab_scroll.get_mut(usize::from(pane)) {
                let next = (*scroll + delta).clamp(0.0, self.tab_scroll_max[usize::from(pane)]);
                self.tab_scroll_manual[usize::from(pane)] |= next != *scroll;
                *scroll = next;
                cx.notify();
            }
            return;
        }
        // Over the results panel the wheel scrolls the results.
        if f32::from(ev.position.y) >= self.panel_list_y - line_h() {
            let len = self.session.state().results().map_or(0, |r| r.rows().len());
            let max = len.saturating_sub(self.panel_list_rows.max(1));
            self.results_scroll = list_wheel(&mut self.results_wheel, ev.delta, self.results_scroll, max);
            cx.notify();
            return;
        }
        if f32::from(ev.position.x) < self.sidebar_w {
            let max = self.tree_len().saturating_sub(self.tree_rows.max(1));
            self.tree_scroll = list_wheel(&mut self.tree_wheel, ev.delta, self.tree_scroll, max);
            self.tree_reveal = false;
            cx.notify();
            return;
        }
        if let Some(g) = self.panes.iter().find(|g| g.text.contains(&ev.position) || g.bar_strip.contains(&ev.position)).copied()
            && g.pane != self.session.state().focused_pane()
        {
            self.dispatch(Command::FocusPane(g.pane), cx);
            self.rows = g.rows;
            self.text_bounds = Some(g.text);
        }
        if let Some(m) = self.focused_markdown() {
            let dy = match ev.delta {
                ScrollDelta::Pixels(p) => -f32::from(p.y),
                ScrollDelta::Lines(l) => -l.y * 3.0 * line_h(),
            };
            let to = m.top + dy;
            return self.scroll_preview(self.session.state().focused_pane(), to, cx);
        }
        let delta = lenz_gpui::scroll::shifted(lenz_gpui::scroll::pixels(ev.delta, line_h(), self.char_w), ev.modifiers.shift);
        let pane = self.session.state().focused_pane() as usize;
        let tab = self.session.state().active().and_then(|t| Some((t.id(), self.session.state().view(t.id())?.snapshot().key().version)));
        if self.wheel_tab[pane] != tab || matches!(ev.delta, ScrollDelta::Lines(_)) {
            self.wheel[pane] = Default::default();
            self.wheel_tab[pane] = tab;
        }
        let dx = lenz_gpui::scroll::steps(&mut self.wheel[pane].x, delta.x / self.char_w);
        let rows = lenz_gpui::scroll::steps(&mut self.wheel[pane].y, delta.y / line_h());
        if dx != 0 {
            self.scroll_to_x(self.hscroll() + dx as f32 * self.char_w, cx);
        }
        if rows != 0 {
            self.dispatch(Command::ScrollRows(rows), cx);
        }
        if let Some(t) = self.session.state().active() {
            let at_end = self.wheel[pane].y > 0.0 && self.session.state().view(t.id()).is_some_and(|view| {
                let rows = view.layout_rows(2, self.session.state().wrap_cols(t));
                rows.pending.is_none() && rows.rows.len() < 2
            });
            lenz_gpui::scroll::clamp(&mut self.wheel[pane].y, t.top_line() == 0 && t.top_row() == 0, at_end);
            let at_right = self.wheel[pane].x > 0.0 && t.column() >= self.max_column();
            lenz_gpui::scroll::clamp(&mut self.wheel[pane].x, t.column() == 0, at_right);
        }
    }

    /// Include a partial last column so its final glyph can become fully visible.
    fn max_column(&self) -> usize { (self.max_hscroll() / self.char_w).ceil() as usize }

    fn native_max_scroll(&self, pane: u8, count: usize) -> Option<f32> {
        let state = self.session.state();
        let tab = state.presentation_shown(pane)?;
        let view = state.presentation_view(tab.id())?;
        let g = self.panes.iter().find(|g| g.pane == pane)?;
        let width = (f32::from(g.text.size.width) - num_w() - 16.0).max(1.0);
        let cols = state.wrap_cols(tab);
        let layout = view.layout_rows(count, cols);
        if layout.pending.is_some() { return None; }
        let mut rows = layout.rows;
        if state.sticky_scroll() {
            let code_core::layout::Query::Ready(sticky) = view.layout_sticky(code_core::tabs::sticky_cap(count), cols) else { return None };
            for pinned in sticky.iter() {
                let code_core::layout::Query::Ready(Some(row)) = view.layout_row(pinned.line, 0, cols) else { return None };
                rows.push(row);
            }
        }
        let mut widest = 0.0_f32;
        for row in rows {
            let extent = if cols.is_some() {
                self.source.extent_wrapped(pane, &view, row.line, row.display.start)? + row.indent as f32 * self.char_w + if row.last { 2.0 } else { 0.0 }
            } else { self.source.extent(pane, &view, row.line)? };
            widest = widest.max(extent);
        }
        Some((widest - width).max(0.0))
    }

    fn max_hscroll(&self) -> f32 {
        self.native_max_scroll(self.session.state().focused_pane(), self.rows)
            .unwrap_or_else(|| self.hscroll() + self.text_bounds.map_or(self.width, |bounds| f32::from(bounds.size.width)).max(1.0))
    }

    /// Runs a Git Branches row's action.
    fn branch_action(&mut self, target: &code_core::branches::BranchTarget, action: code_core::branches::BranchAction, window: &mut Window, cx: &mut Context<Self>) {
        use code_core::branches::{BranchAction, BranchTarget};
        let _ = window;
        self.dispatch(Command::CloseBranches, cx);
        match (action, target) {
            (BranchAction::Checkout, t) => {
                if let Some(target) = t.checkout() {
                    self.dispatch(Command::Checkout(target), cx);
                }
            }
            (BranchAction::NewBranchFrom, t) => self.modal = Some(Modal::NewBranch { from: t.rev(), name: String::new(), checkout: true, error: None }),
            (BranchAction::Open, BranchTarget::Worktree(path)) => self.dispatch(Command::OpenWorktree(path.clone()), cx),
            (BranchAction::OpenInNewWindow, BranchTarget::Worktree(path)) => {
                let (make_config, path) = (self.make_config.clone(), path.clone());
                #[cfg(feature = "test-support")]
                self.opened_windows.push(path.clone());
                #[cfg(not(feature = "test-support"))]
                cx.defer(move |cx| {
                    if let Err(e) = open_window(cx, make_config, Some(path)) {
                        eprintln!("lenz-code: cannot open a window: {e}");
                    }
                });
                #[cfg(feature = "test-support")]
                let _ = make_config;
            }
            (BranchAction::CompareWithWorkingTree | BranchAction::ChangesSinceBranching, t) => {
                if let Some(diff) = code_core::branches::GitDiff::of(t, action) {
                    self.git_diff(&diff, cx);
                }
            }
            (BranchAction::Open | BranchAction::OpenInNewWindow, _) => {}
        }
        cx.notify();
    }

    /// Starts Lenz Diff on a Git comparison, or shows why not.
    fn git_diff(&mut self, diff: &code_core::branches::GitDiff, cx: &mut Context<Self>) {
        let exe = self.exe();
        let launch = self.session.git_diff_launch(diff, &exe, false);
        self.start(launch, None, cx);
    }

    /// A details dialog's buttons: the core's checkout choices, the commit popup's actions, else Copy and Close.
    pub(crate) fn details_labels(&self, kind: Details) -> Vec<&'static str> {
        match kind {
            Details::Install => self.session.server_install_actions().into_iter().map(CommandId::label).chain(std::iter::once("Close")).collect(),
            Details::Checkout => self.session.checkout_choices().into_iter().map(|c| c.0).collect(),
            Details::Change => vec!["Show Diff (D)", "Revert This Change (R)", "Close"],
            Details::Commit => vec!["Show Diff", "Copy Revision", "Close"],
            Details::Text => vec!["Copy", "Close"],
            Details::Terminal { .. } => vec![self.session.state().terminal().prompt().map_or("Confirm", |p| p.accept), "Cancel"],
            Details::Sql => vec!["SQL menu (M)", "Copy", "Close"],
            Details::External => code_core::external::Choice::ALL.into_iter().map(|choice|choice.label()).collect(),
            Details::Reconcile => code_core::reconcile::Choice::ALL.into_iter().map(|choice|choice.label()).collect(),
            Details::Replace => code_core::save::ReplaceChoice::ALL.into_iter().map(|choice|choice.label()).collect(),
            Details::Recovered => code_core::recovery::Choice::ALL.into_iter().map(|choice|choice.label()).collect(),
            Details::Close => [code_core::close::Choice::Save,code_core::close::Choice::Discard,code_core::close::Choice::Cancel].into_iter().map(|c|self.session.close_prompt().map_or(c.label(), |p|p.label(c))).collect(),
        }
    }

    /// The annotation under the pointer, as laid out now (none while a dialog is open).
    pub(crate) fn ann_hovered(&self) -> Option<(u8, TabId, usize)> {
        self.ann_hover.filter(|_| self.modal.is_none()).and_then(|at| self.annotation_at(at))
    }

    /// The blame column under a window point: the pane, its tab and the line whose first row is there.
    fn annotation_at(&self, at: gpui::Point<Pixels>) -> Option<(u8, TabId, usize)> {
        let g = self.panes.iter().find(|g| {
            let (x, left) = (f32::from(at.x), f32::from(g.text.origin.x));
            g.ann > 0.0 && (left - g.ann..left).contains(&x) && at.y >= g.text.origin.y && at.y < g.text.origin.y + g.text.size.height
        })?;
        let state = self.session.state();
        let tab = state.shown(g.pane)?;
        if !self.current_source_geometry(tab) { return None; }
        // Beside a sticky line: that line (the rows under the band are hidden).
        if let Some(&(_, _, line)) = self.sticky_hits.iter().find(|(b, pane, _)| *pane == g.pane && at.y >= b.origin.y && at.y < b.origin.y + b.size.height) {
            return Some((g.pane, tab.id(), line));
        }
        let row = ((f32::from(at.y) - f32::from(g.text.origin.y)) / line_h()).floor() as usize;
        let rows = state.presentation_view(tab.id())?.layout_rows(row + 1, state.wrap_cols(tab)).rows;
        let r = rows.get(row).filter(|r| r.index == 0)?;
        Some((g.pane, tab.id(), r.line))
    }

    fn current_source_geometry(&self, tab: &Tab) -> bool {
        self.session.state().presentation_view(tab.id()).is_some_and(|view| {
            self.source_geometry.get(tab.pane() as usize).and_then(Option::as_ref).is_some_and(|g| g.matches(&view, self))
        })
    }

    /// The source byte under a window point in the text area.
    fn byte_at(&self, window: &Window, x: f32, y: f32) -> Option<usize> {
        let tab = self.session.state().active()?;
        let (line, byte) = self.byte_in(window, tab, self.text_bounds?, x, y)?;
        Some(self.session.state().presentation_view(tab.id())?.snapshot().line_range(line)?.start + byte)
    }

    fn byte_in(&self, window: &Window, tab: &Tab, bounds: Bounds<Pixels>, x: f32, y: f32) -> Option<(usize, usize)> {
        if !self.current_source_geometry(tab) || window.viewport_size() != gpui::size(px(self.width), px(self.height)) || y < f32::from(bounds.origin.y) { return None; }
        let state = self.session.state();
        let view = state.presentation_view(tab.id())?;
        let tab = view.tab();
        let at = ((y - f32::from(bounds.origin.y)) / line_h()).floor() as usize;
        let rows = view.layout_rows(at + 1, state.wrap_cols(tab)).rows;
        let row = rows.get(at).or_else(|| rows.last())?;
        let projected = self.ready_source_row(tab, row)?;
        let scroll = tab.column() as f32 * self.char_w;
        let tx = lenz_gpui::text_x(self.metrics(), f32::from(bounds.origin.x), scroll) + row.indent as f32 * self.char_w;
        let byte = projected.hit(window, self.metrics(), x - tx)?;
        Some((row.line, byte.saturating_sub(view.snapshot().line_range(row.line)?.start)))
    }

    fn fold_click(&self, window: &Window, x: f32, y: f32) -> Option<usize> {
        let (tab, bounds) = (self.session.state().active()?, self.text_bounds?);
        if !self.current_source_geometry(tab) || window.viewport_size() != gpui::size(px(self.width), px(self.height)) || y < f32::from(bounds.origin.y) { return None; }
        let state = self.session.state();
        let view = state.presentation_view(tab.id())?;
        let tab = view.tab();
        let at = ((y - f32::from(bounds.origin.y)) / line_h()).floor() as usize;
        let rows = view.layout_rows(at + 1, state.wrap_cols(tab)).rows;
        let row = rows.get(at)?;
        let (_, collapsed) = view.fold_at(row.line)?;
        if row.index == 0 && (num_w() - fold_w()..num_w()).contains(&(x - f32::from(bounds.origin.x))) { return Some(row.line); }
        let projected = self.ready_source_row(tab, row)?;
        if !row.last || !projected.complete() || !collapsed { return None; }
        let tx = lenz_gpui::text_x(self.metrics(), f32::from(bounds.origin.x), tab.column() as f32 * self.char_w) + row.indent as f32 * self.char_w;
        let end = projected.x_for(window, self.metrics(), projected.text.len())?;
        (lenz_gpui::column_at(window, self.metrics(), PLACEHOLDER, x - tx - end) > 0).then_some(row.line)
    }

    pub fn fold_marks(&self) -> Vec<(usize, bool)> {
        let state = self.session.state();
        state.active().map_or_else(Vec::new, |tab| Self::marks(state, tab, self.rows))
    }

    pub fn bookmark_marks(&self) -> Vec<(usize, Option<char>)> {
        let state = self.session.state();
        state.active().map_or_else(Vec::new, |tab| Self::bookmark_rows(state, tab, self.rows))
    }

    fn bookmark_rows(state: &code_core::session::State, tab: &Tab, rows: usize) -> Vec<(usize, Option<char>)> {
        state.presentation_view(tab.id()).map_or_else(Vec::new, |view| view.layout_rows(rows, state.wrap_cols(tab)).rows.iter().enumerate().filter(|(_, r)| r.index == 0).filter_map(|(i, r)| state.places().bookmark(tab.source(), r.line).map(|m| (i, m.mnemonic))).collect())
    }

    fn marks(state: &code_core::session::State, tab: &Tab, rows: usize) -> Vec<(usize, bool)> {
        state.presentation_view(tab.id()).map_or_else(Vec::new, |view| view.layout_rows(rows, state.wrap_cols(tab)).rows.iter().enumerate().filter(|(_, r)| r.index == 0).filter_map(|(i, r)| view.fold_at(r.line).map(|(_, c)| (i, c))).collect())
    }

    /// The identifier a Ctrl-click at a window point would follow, when `held` and the pane shows text there.
    fn link_at(&self, window: &Window, x: f32, y: f32, held: bool) -> Option<(u8, std::ops::Range<usize>)> {
        if !held || self.modal.is_some() || self.drag.is_some() || self.dragging {
            return None;
        }
        let g = self.panes.iter().find(|g| g.text.contains(&gpui::point(px(x), px(y))))?;
        let state = self.session.state();
        let tab = state.presentation_shown(g.pane)?;
        if g.pane == state.focused_pane() && self.session.disabled(CommandId::Definition).is_some() {
            return None;
        }
        if matches!(self.session.state().presentation_view(tab.id()).and_then(|v| v.rendering()), Some(Rendering::Preview(_) | Rendering::Pending)) || state.presentation_view(tab.id()).is_some_and(|view| wanted_image(&view).is_some() || unshowable_image(&view).is_some()) {
            return None; // no text drawn
        }
        // The character under the pointer: the boundary nearest half a cell to its left starts it.
        let (line, col) = self.byte_in(window, tab, g.text, x - self.char_w / 2.0, y)?;
        let view = state.presentation_view(tab.id())?;
        let word = view.linkable(line, col)?;
        let start = view.snapshot().line_range(line)?.start;
        Some((g.pane, start + word.start..start + word.end))
    }

    /// Show the link under the pointer, if it changed.
    fn hover_link(&mut self, window: &Window, x: f32, y: f32, held: bool, cx: &mut Context<Self>) {
        let link = self.link_at(window, x, y, held);
        if link != self.link {
            self.link = link;
            cx.notify();
        }
    }

    /// Documentation on a pointer rest: a name the pointer stops on is asked about after [`DOC_REST`] (not while
    /// Ctrl is held, a button is down or a menu is open, nor when switched off); a popup the pointer opened closes
    /// [`DOC_GRACE`] after the pointer leaves both it and its name.
    fn rest_pointer(&mut self, window: &Window, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let quiet = ev.modifiers.control || ev.modifiers.platform || ev.pressed_button.is_some() || self.modal.is_some() || !self.session.state().documentation_on_hover();
        let state = self.session.state();
        let name = (!quiet).then(|| self.link_at(window, x, y, true)).flatten().and_then(|(pane, r)| Some((state.presentation_shown(pane)?.id(), r)));
        if name != self.doc_rest.0 {
            let task = name.clone().map(|(tab, r)| {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(DOC_REST).await;
                    // A menu or dialog opened during the rest (a right-click) takes the pointer: nothing pops up over it.
                    let _ = this.update(cx, |v, cx| {
                        if v.modal.is_none() {
                            v.dispatch(Command::DocumentationAt { tab, offset: r.start }, cx);
                        }
                    });
                })
            });
            self.doc_rest = (name.clone(), task);
        }
        let shown = self.session.state().documentation().filter(|d| d.trigger == code_core::session::DocTrigger::Pointer);
        let inside = self.doc_popup.as_ref().is_some_and(|d| d.bounds.contains(&ev.position)) || shown.is_some_and(|d| name.as_ref().is_some_and(|(t, r)| *t == d.tab && *r == d.at));
        if shown.is_none() || inside {
            self.doc_grace = None;
        } else if self.doc_grace.is_none() {
            self.doc_grace = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DOC_GRACE).await;
                let _ = this.update(cx, |v, cx| {
                    v.doc_grace = None;
                    if v.session.state().documentation().is_some_and(|d| d.trigger == code_core::session::DocTrigger::Pointer) {
                        v.dispatch(Command::CloseDocumentation, cx);
                    }
                });
            }));
        }
    }

    /// The documentation popup for this frame: laid out for its pane (again only when the documentation or width
    /// changed), below its name or above it when there is no room; `None` lets go of it.
    fn doc_frame(&mut self, window: &Window) -> Option<(Bounds<Pixels>, Arc<preview::Layout>, f32)> {
        if self.modal.is_some() { self.doc_popup = None; return None; }
        let state = self.session.state();
        let Some(d) = state.documentation() else {
            self.doc_popup = None;
            return None;
        };
        let (tab, preview) = (d.tab, d.preview.clone());
        let at = d.at.start;
        let Some(g) = self.panes.iter().find(|g| state.presentation_shown(g.pane).is_some_and(|t| t.id() == tab)).copied() else {
            self.doc_popup = None;
            return None;
        };
        let t = state.presentation_shown(g.pane)?;
        let view = state.presentation_view(t.id())?;
        let (row, anchor_x) = view.layout_rows(g.rows, state.wrap_cols(t)).rows.iter().enumerate().find_map(|(i, r)| {
            let native = self.ready_source_row(t, r)?;
            let local = native.caret(at)?;
            let x = native.x_for(window, self.metrics(), local)?;
            Some((i as f32, r.indent as f32 * self.char_w + x))
        }).unwrap_or((0.0, 0.0));
        let column = t.column();
        let w = (f32::from(g.text.size.width) - 32.0).clamp(120.0, 640.0);
        let fresh = self.doc_popup.as_ref().is_some_and(|p| Arc::ptr_eq(&p.layout.preview, &preview) && p.layout.width == w);
        if !fresh {
            let (style, palette) = (self.reading_style(window), self.session.state().palette().clone());
            let layout = Arc::new(preview::Layout::new(preview, w, style, palette, &|_| 0, window.text_system()));
            self.doc_popup = Some(DocPopup { layout, top: 0.0, bounds: Bounds::default() });
        }
        let popup = self.doc_popup.as_mut()?;
        let h = (popup.layout.height + 2.0).min(self.height * 0.4).max(line_h());
        let (tx, ty) = (f32::from(g.text.origin.x), f32::from(g.text.origin.y));
        let x = (lenz_gpui::text_x(metrics(&self.font), tx, column as f32 * self.char_w) + anchor_x).clamp(0.0, (self.width - w).max(0.0));
        let below = ty + (row + 1.0) * line_h() + 4.0;
        let y = if below + h > self.height { (ty + row * line_h() - h - 4.0).max(0.0) } else { below };
        popup.bounds = Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(w), px(h)) };
        Some((popup.bounds, popup.layout.clone(), popup.top))
    }

    /// The completion popup for this frame: under the name being completed (above it without room), clamped to
    /// the window, at most [`COMPLETION_ROWS`] rows scrolled to the selection. Per row: label, detail, selected,
    /// unavailable.
    fn completion_frame(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<CompletionFrame> {
        if self.modal.is_some() { self.completion_drawn = None; self.completion_doc = None; self.completion_doc_building = None; self.completion_doc_drawn = None; return None; }
        let state = self.session.state();
        let Some(c) = state.completion() else { self.completion_drawn = None; self.completion_doc = None; self.completion_doc_building = None; self.completion_doc_drawn = None; return None };
        let Some(g) = self.panes.iter().find(|g| state.presentation_shown(g.pane).is_some_and(|t| t.id() == c.tab)).copied() else { self.completion_drawn = None; self.completion_doc = None; self.completion_doc_building = None; self.completion_doc_drawn = None; return None };
        let t = state.presentation_shown(g.pane)?;
        let view = state.presentation_view(t.id())?;
        let (row, anchor_x) = view.layout_rows(g.rows, state.wrap_cols(t)).rows.iter().enumerate().find_map(|(i, r)| {
            let native = self.ready_source_row(t, r)?;
            let x = native.x_for(window, self.metrics(), native.caret(c.word())?)?;
            Some((i as f32, r.indent as f32 * self.char_w + x))
        }).unwrap_or((0.0, 0.0));
        let n = c.rows().len();
        let available = if c.documentation_preview().is_some() && self.width < 680.0 {self.height*0.5}else{self.height};
        let shown = n.min(COMPLETION_ROWS).min(((available - 8.0) / line_h()).max(1.0) as usize);
        // Typing can filter every suggestion away: no popup, as in the terminal.
        if shown == 0 { self.completion_drawn = None; self.completion_doc = None; self.completion_doc_building = None; self.completion_doc_drawn = None; return None; }
        let first = c.selected().saturating_sub(shown - 1).min(n - shown);
        let rows: Vec<_> = c.rows().enumerate().skip(first).take(shown).map(|(i, r)| {
            let group = i > 0 && c.rows().nth(i - 1).is_some_and(|previous| previous.kind != r.kind);
            ((r.label.clone(), r.detail.clone().unwrap_or_default(), i == c.selected(), r.unavailable.is_some()), c.matches(&r.label), r.kind, group)
        }).collect();
        let chars = rows.iter().map(|(r, _, kind, _)| r.0.chars().count() + r.1.chars().count() + kind.map_or(0, str::len) + 5).max().unwrap_or(0);
        let w = ((chars as f32 + 2.0) * self.char_w).clamp(120.0_f32.min(self.width), self.width.clamp(1.0, 480.0));
        let h = shown as f32 * line_h() + 4.0;
        let (tx, ty) = (f32::from(g.text.origin.x), f32::from(g.text.origin.y));
        let x = (lenz_gpui::text_x(metrics(&self.font), tx, t.column() as f32 * self.char_w) + anchor_x).clamp(0.0, (self.width - w).max(0.0));
        let below = ty + (row + 1.0) * line_h() + 2.0;
        let y = if below + h > self.height { (ty + row * line_h() - h - 2.0).max(0.0) } else { below };
        let mut bounds = Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(w), px(h)) };
        let preview = c.documentation_preview().cloned();
        // Keep the full Markdown layout; only visible rows are painted.
        let doc = if let Some(preview) = preview {
            let side_room = (self.width - x - w - 4.0).max(x - 4.0);
            let side = side_room >= 180.0;
            let pw = if side { side_room.min(520.0) } else { self.width.min(520.0) }.max(1.0);
            let layout=self.completion_layout(preview,pw,window,cx);
            let ph = layout.as_ref().map_or(line_h()*3.0,|l|l.height).min((self.height * 0.5).max(line_h())).min(self.height.max(1.0));
            let dx = if side && self.width - x - w - 4.0 >= pw {x+w+4.0} else if side {x-pw-4.0} else {x.min((self.width-pw).max(0.0))};
            let dy = if side {y.min((self.height-ph).max(0.0))} else {
                let list_y=y.min((self.height-h-ph-4.0).max(0.0)); bounds.origin.y=px(list_y); (list_y+h+4.0).min((self.height-ph).max(0.0))
            };
            let area=Bounds {origin:gpui::point(px(dx.max(0.0)),px(dy)),size:gpui::size(px(pw),px(ph))};
            self.completion_doc_drawn=Some(area);
            let top=if let Some(d)=&mut self.completion_doc {d.bounds=area;d.top=d.top.clamp(0.0,(d.layout.height-ph).max(0.0));d.top}else{0.0};
            Some((area,layout,top))
        } else {self.completion_doc=None; self.completion_doc_building=None; self.completion_doc_drawn=None; None};
        self.completion_drawn = Some((bounds, first, rows.len()));
        Some((bounds, rows, doc))
    }

    /// The quick-fix list for this frame: under the caret's row (above it without room), clamped to the window, at
    /// most [`COMPLETION_ROWS`] rows scrolled to the selection, then the selected action's reason when it is not applied.
    fn code_actions_frame(&mut self, window: &Window) -> Option<CodeActionFrame> {
        if self.modal.is_some() { self.code_actions_drawn = None; return None; }
        let state = self.session.state();
        let Some(c) = state.code_actions() else { self.code_actions_drawn = None; return None };
        let Some(g) = self.panes.iter().find(|g| state.presentation_shown(g.pane).is_some_and(|t| t.id() == c.tab)).copied() else { self.code_actions_drawn = None; return None };
        let t = state.presentation_shown(g.pane)?;
        let view = state.presentation_view(t.id())?;
        let (row, anchor_x) = view.layout_rows(g.rows, state.wrap_cols(t)).rows.iter().enumerate().find_map(|(i, r)| {
            let native = self.ready_source_row(t, r)?;
            let x = native.x_for(window, self.metrics(), native.caret(c.caret())?)?;
            Some((i as f32, r.indent as f32 * self.char_w + x))
        }).unwrap_or((0.0, 0.0));
        let n = c.actions.len();
        let shown = n.min(COMPLETION_ROWS).min(((self.height - 8.0) / line_h()).max(1.0) as usize);
        let first = c.selected.saturating_sub(shown - 1).min(n - shown);
        let rows: Vec<_> = c.actions.iter().enumerate().skip(first).take(shown)
            .map(|(i, a)| (format!("{}{}", if a.preferred { "★ " } else { "" }, a.title), String::new(), i == c.selected, a.unavailable.is_some())).collect();
        let reason = c.actions.get(c.selected).and_then(|a| a.unavailable.clone());
        let chars = rows.iter().map(|r| r.0.chars().count()).chain(reason.iter().map(|r| r.chars().count())).max().unwrap_or(0);
        let w = ((chars as f32 + 2.0) * self.char_w).clamp(120.0_f32.min(self.width), self.width.clamp(1.0, 640.0));
        let h = (shown + usize::from(reason.is_some())) as f32 * line_h() + 4.0;
        let (tx, ty) = (f32::from(g.text.origin.x), f32::from(g.text.origin.y));
        let x = (lenz_gpui::text_x(metrics(&self.font), tx, t.column() as f32 * self.char_w) + anchor_x).clamp(0.0, (self.width - w).max(0.0));
        let below = ty + (row + 1.0) * line_h() + 2.0;
        let y = if below + h > self.height { (ty + row * line_h() - h - 2.0).max(0.0) } else { below };
        let bounds = Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(w), px(h)) };
        self.code_actions_drawn = Some((bounds, first, rows.len()));
        Some((bounds, rows, reason))
    }

    /// The quick-fix list's box, while one is drawn.
    pub fn code_actions_bounds(&self) -> Option<Bounds<Pixels>> {
        self.code_actions_drawn.filter(|_| self.modal.is_none() && self.session.state().code_actions().is_some()).map(|d| d.0)
    }

    /// The parameter information box for this frame: above the caret's row (under it without room), with the
    /// active parameter marked, the overload count and the documentation, clamped to the window.
    fn signature_frame(&mut self, window: &Window) -> Option<(Bounds<Pixels>, Vec<SignatureLine>)> {
        if self.modal.is_some() { self.signature_drawn = None; return None; }
        let state = self.session.state();
        let Some(h) = state.signature_help() else { self.signature_drawn = None; return None };
        let Some(sig) = h.active_signature() else { self.signature_drawn = None; return None };
        let Some(g) = self.panes.iter().find(|g| state.presentation_shown(g.pane).is_some_and(|t| t.id() == h.tab)).copied() else { self.signature_drawn = None; return None };
        let t = state.presentation_shown(g.pane)?;
        let view = state.presentation_view(t.id())?;
        let (row, anchor_x) = view.layout_rows(g.rows, state.wrap_cols(t)).rows.iter().enumerate().find_map(|(i, r)| {
            let native = self.ready_source_row(t, r)?;
            let x = native.x_for(window, self.metrics(), native.caret(h.caret())?)?;
            Some((i as f32, r.indent as f32 * self.char_w + x))
        }).unwrap_or((0.0, 0.0));
        let cols = ((self.width / self.char_w) as usize).saturating_sub(4).clamp(8, 80);
        let mut lines: Vec<SignatureLine> = vec![(sig.label.clone(), sig.active_parameter.map(|p| sig.parameters[p].label.clone()).filter(|r| !r.is_empty()), false)];
        if h.signatures.len() > 1 { lines.push((format!("{} of {} signatures", h.active + 1, h.signatures.len()), None, true)); }
        if let Some(doc) = sig.active_parameter.and_then(|p| sig.parameters[p].documentation.as_deref()) { lines.extend(wrap_text(doc, cols, 4).into_iter().map(|l| (l, None, false))); }
        if let Some(doc) = sig.documentation.as_deref() { lines.extend(wrap_text(doc, cols, 4).into_iter().map(|l| (l, None, true))); }
        let chars = lines.iter().map(|l| l.0.chars().count()).max().unwrap_or(1);
        let w = ((chars as f32 + 2.0) * self.char_w).min(self.width.max(1.0));
        let hgt = lines.len() as f32 * line_h() + 4.0;
        let (tx, ty) = (f32::from(g.text.origin.x), f32::from(g.text.origin.y));
        let x = (lenz_gpui::text_x(metrics(&self.font), tx, t.column() as f32 * self.char_w) + anchor_x).clamp(0.0, (self.width - w).max(0.0));
        let above = ty + row * line_h() - hgt - 2.0;
        let y = if above >= 0.0 { above } else { ty + (row + 1.0) * line_h() + 2.0 };
        let bounds = Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(w), px(hgt)) };
        self.signature_drawn = Some(bounds);
        Some((bounds, lines))
    }

    /// The parameter information box, while one is drawn.
    pub fn signature_bounds(&self) -> Option<Bounds<Pixels>> {
        self.signature_drawn.filter(|_| self.modal.is_none() && self.session.state().signature_help().is_some())
    }

    /// The completion popup's box, while one is drawn.
    pub fn completion_bounds(&self) -> Option<Bounds<Pixels>> {
        self.completion_drawn.filter(|_| self.modal.is_none() && self.session.state().completion().is_some()).map(|d| d.0)
    }

    /// The documentation popup's box, while one is drawn.
    pub fn doc_popup_bounds(&self) -> Option<Bounds<Pixels>> {
        self.doc_popup.as_ref().filter(|_| self.modal.is_none()).map(|d| d.bounds)
    }

    fn on_modifiers(&mut self, ev: &gpui::ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() || self.session.state().terminal().focused() {
            self.double_shift.reset();
        } else if self.double_shift.update(ev.modifiers.shift, ev.modifiers.control || ev.modifiers.alt || ev.modifiers.platform || ev.modifiers.function, std::time::Instant::now()) {
            self.open_navigation(code_core::navigation::Scope::All, cx);
        }
        let at = window.mouse_position();
        self.hover_link(window, f32::from(at.x), f32::from(at.y), ev.modifiers.control || ev.modifiers.platform, cx);
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.reset_caret_blink();
        if self.modal.is_none() && self.input_state.capture.is_some() { self.cancel_input(true, cx); cx.stop_propagation(); return; }
        self.double_shift.interrupt();
        window.focus(&self.focus, cx);
        if matches!(self.modal, Some(Modal::Navigation { .. })) {
            return self.navigation_click(ev.position, window, cx);
        }
        if matches!(self.modal, Some(Modal::Search { .. })) {
            return self.search_click(ev.position, cx);
        }
        // A click on a context menu entry runs it; anywhere else closes the menu.
        if let Some((_, rows)) = self.context_layout() {
            let hit = self.context_entry_at(ev.position).map(|i| rows[i].2);
            self.modal = None;
            if let Some(MenuEntry::Command(id)) = hit {
                self.run(id, window, cx);
            } else { self.terminal_ui.path_context = None; }
            return cx.notify();
        }
        if self.form_click(ev.position, window, cx) { return; }
        // A click on a modal's row picks it (see `modal_click`); on its header, input or padding it does nothing; one
        // outside dismisses the modal. Go to Line has no rows: any click dismisses it, as before.
        if self.modal.is_some() {
            let inside = self.modal_layout().filter(|l| l.bounds.contains(&ev.position) && !matches!(self.modal, Some(Modal::GoTo { .. })));
            match inside {
                Some(l) => {
                    if let Some(Modal::StatusDetails { text, kind, .. }) = &self.modal {
                        let (text, kind) = (text.clone(), *kind);
                        let hit = l.details_buttons(self.char_w, &self.details_labels(kind)).iter().position(|b| b.contains(&ev.position));
                        match (kind, hit) {
                            (Details::Terminal { ticket }, Some(i)) => self.dispatch(Command::Terminal(code_core::terminal::Command::Confirm { ticket, accept: i == 0 }), cx),
                            (Details::Install, Some(i)) => {
                                if let Some(id) = self.session.server_install_actions().get(i).copied() { self.run(id,window,cx); }
                                else { self.close_modal(cx); }
                            }
                            (Details::External,Some(i)) => {if let Some(choice)=code_core::external::Choice::ALL.get(i) {self.choose_external(*choice,cx);}}
                            (Details::Reconcile,Some(i)) => {if let Some(choice)=code_core::reconcile::Choice::ALL.get(i) {self.choose_reconcile(*choice,cx);}}
                            (Details::Replace,Some(i)) => {if let Some(choice)=code_core::save::ReplaceChoice::ALL.get(i) {self.choose_replace(*choice,cx);}}
                            (Details::Close,Some(i)) => {if let Some(choice)=[code_core::close::Choice::Save,code_core::close::Choice::Discard,code_core::close::Choice::Cancel].get(i) {self.choose_close(*choice,cx);}}
                            (Details::Recovered,Some(i)) => {if let Some(choice)=code_core::recovery::Choice::ALL.get(i) {self.choose_recovery(*choice,cx);}}
                            (Details::Checkout, Some(i)) => {
                                if let Some(&(_, c)) = self.session.checkout_choices().get(i) {
                                    self.dispatch(Command::CheckoutChoice(c), cx);
                                }
                            }
                            (Details::Change, Some(0)) => self.run(CommandId::ShowChangeDiff,window,cx),
                            (Details::Change, Some(1)) => self.run(CommandId::RevertChange,window,cx),
                            (Details::Commit, Some(0)) => self.run(CommandId::ShowCommitDiff, window, cx),
                            (Details::Commit, Some(1)) => self.run(CommandId::CopyRevision, window, cx),
                            (Details::Sql, Some(0)) => self.modal = Some(Modal::CommandGroup { group: CommandGroup::Sql, at: 1 }),
                            (Details::Text, Some(0)) | (Details::Sql, Some(1)) => self.dispatch(Command::CopyText(text), cx),
                            (_, Some(_)) => self.close_modal(cx),
                            (_, None) => {}
                        }
                        return cx.notify();
                    }
                    let n = ((f32::from(ev.position.y) - f32::from(l.bounds.origin.y) - l.body_top) / line_h()).floor();
                    if n >= 0.0 && (n as usize) < l.visible && let Some(&(row, ..)) = l.lines.get(l.first + n as usize) {
                        self.modal_click(row, window, cx);
                    }
                    cx.notify();
                }
                None => self.close_modal(cx),
            }
            return;
        }
        if self.terminal_click(ev, window, cx) { return; }
        self.blur_terminal(cx);
        if self.csv_field().is_some() { return; }
        if self.csv_click(ev, cx) { return; }
        // A find box keeps a click: a toggle flips its option, anywhere else gives it the keys.
        if self.modal.is_none() && let Some((pane, b)) = self.panes.iter().find_map(|g| self.find_box(g).filter(|b| b.bounds.contains(&ev.position)).map(|b| (g.pane, b))) {
            match b.option_at(ev.position) {
                Some(option) => self.dispatch(Command::ToggleFindOption(option), cx),
                None => {
                    if pane != self.session.state().focused_pane() {
                        self.dispatch(Command::FocusPane(pane), cx);
                    }
                    self.find_focus = Some(pane);
                    self.find_replace = b.replace_at(ev.position);
                    cx.notify();
                }
            }
            return;
        }
        // A click elsewhere gives the keys back to the text; the box stays open.
        self.find_focus = None;
        self.find_replace = false;
        // A click on a quick fix applies it; one elsewhere closes the list (and goes on).
        if let Some((b, first, n)) = self.code_actions_drawn.filter(|_| self.session.state().code_actions().is_some()) {
            if b.contains(&ev.position) {
                let row = first + ((f32::from(ev.position.y - b.origin.y) - 2.0).max(0.0) / line_h()) as usize;
                if row < first + n && ev.button == MouseButton::Left {
                    self.dispatch(Command::SelectCodeAction(row), cx);
                    self.dispatch(Command::ApplyCodeAction, cx);
                }
                return;
            }
            self.dispatch(Command::CloseCodeActions, cx);
        }
        if self.session.state().completion().is_some() && self.completion_doc_drawn.is_some_and(|b|b.contains(&ev.position)) { return; }
        // A click on a suggestion accepts it; one elsewhere closes the popup (and goes on).
        if let Some((b, first, n)) = self.completion_drawn.filter(|_| self.session.state().completion().is_some()) {
            if b.contains(&ev.position) {
                let row = first + ((f32::from(ev.position.y - b.origin.y) - 2.0).max(0.0) / line_h()) as usize;
                if row < first + n && ev.button == MouseButton::Left {
                    self.dispatch(Command::SelectCompletion(row), cx);
                    self.dispatch(Command::AcceptCompletion, cx);
                }
                return;
            }
            self.dispatch(Command::CloseCompletion, cx);
        }
        // The documentation popup keeps a click inside it; one elsewhere closes it (and goes on).
        if self.doc_popup.as_ref().is_some_and(|d| d.bounds.contains(&ev.position)) {
            return;
        }
        if self.session.state().documentation().is_some() {
            self.dispatch(Command::CloseDocumentation, cx);
        }
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if self.activity_button().is_some_and(|(b, _)| b.contains(&ev.position)) { return self.run(CommandId::ToggleActivityLog, window, cx); }
        if self.results_button().is_some_and(|(b, _)| b.contains(&ev.position)) { return self.run(CommandId::ToggleResults, window, cx); }
        if let Some(l) = self.activity_layout() && l.bounds.contains(&ev.position) {
            self.log_focus = true;
            if let Some((_, id, _)) = l.buttons.iter().find(|(b, _, _)| b.contains(&ev.position)) { self.run(*id, window, cx); }
            return cx.notify();
        }
        self.log_focus = false;
        if y >= self.height - self.status_h() {
            if self.commands_button().is_some_and(|(b, _)| b.contains(&ev.position)) {
                self.open_navigation(code_core::navigation::Scope::Actions, cx);
                return;
            }
            let (server, _, _, sql, _) = self.status_segments();
            let id = if server.0.contains(&ev.position) { CommandId::LanguageServer } else if sql.0.contains(&ev.position) { CommandId::SqlDialect } else { CommandId::StatusDetails };
            self.run(id, window, cx);
            return;
        }
        // A divider: dragged until the button is released.
        if let Some(divider) = self.divider_at(x, y) {
            self.drag = Some(divider);
            return;
        }
        if let Some(g) = self.panes.iter().find(|g| g.bar_strip.contains(&ev.position)).copied() {
            return self.scrollbar_press(g, y, cx);
        }
        // A breadcrumb: a crumb shows its folder or file or goes to its declaration; its `›` lists what is inside.
        if let Some(h) = self.crumb_hits.iter().find(|h| h.bounds.contains(&ev.position)).cloned() {
            self.crumb_focus = None;
            let Some(i) = h.crumb else { return cx.notify() };
            if h.chevron {
                self.modal = Some(Modal::Siblings { pane: h.pane, crumb: i, query: String::new(), at: 0 });
                return cx.notify();
            }
            return self.follow_crumb(h.pane, i, window, cx);
        }
        self.crumb_focus = None;
        // A start screen row: its command, or its recent folder opened.
        if let Some(&(_, i)) = self.start_hits.iter().find(|h| h.0.contains(&ev.position)) {
            match self.session.state().start_items().into_iter().nth(i) {
                Some(StartItem::Command(id)) => self.run(id, window, cx),
                Some(StartItem::RecentFolder(root)) => self.open_root(root, cx),
                Some(StartItem::RecentFile(path)) => self.open(path, cx),
                None => {}
            }
            return cx.notify();
        }
        // A sticky line: its pane gets the keys and the caret goes to the line (Back returns).
        if let Some(&(_, pane, line)) = self.sticky_hits.iter().find(|h| h.0.contains(&ev.position) && self.session.state().presentation_shown(h.1).is_some_and(|tab| self.current_source_geometry(tab))) {
            if pane != self.session.state().focused_pane() {
                self.dispatch(Command::FocusPane(pane), cx);
            }
            let line = u32::try_from(line + 1).unwrap_or(u32::MAX);
            return self.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line, column: None }), cx);
        }
        // The panel's heading: its buttons.
        if let Some((_, id, _)) = self.results_buttons().into_iter().find(|(b, _, _)| b.contains(&ev.position)) {
            self.run(id, window, cx);
            return;
        }
        // The results list: a click selects for preview, a double-click opens.
        if y >= self.panel_list_y && y < self.panel_list_y + self.panel_list_rows as f32 * line_h() && x < self.panel_list_w {
            let at = self.results_scroll + ((y - self.panel_list_y) / line_h()) as usize;
            let row = self.session.state().results().and_then(|r| r.rows().get(at).cloned());
            if row.is_some() {
                self.results_focus = true;
                self.tree_focus = false;
            }
            match row {
                // A folder or file folds or unfolds.
                Some(Row::Dir { dir, .. }) => self.dispatch(Command::ToggleResultGroup(Group::Dir(dir)), cx),
                Some(Row::File { path, .. }) => self.dispatch(Command::ToggleResultGroup(Group::File(path)), cx),
                // A call hierarchy's fold mark unfolds or folds its row.
                Some(Row::Hit(i)) if self.session.state().results().and_then(|r| r.call(i)).is_some_and(|c| ((x - 8.0) / self.char_w).floor() as usize == 2 * c.depth as usize) => {
                    self.dispatch(Command::SelectResult(i), cx);
                    self.dispatch(Command::ToggleCall(i), cx);
                }
                Some(Row::Hit(i)) => {
                    self.dispatch(Command::SelectResult(i), cx);
                    if ev.click_count >= 2 {
                        self.dispatch(Command::OpenResult(i), cx);
                    }
                }
                None => {}
            }
            return;
        }
        if y >= self.panel_list_y - line_h() {
            return; // the panel's status line and preview
        }
        self.results_focus = false;
        if self.app_menu_button().contains(&ev.position) {
            self.modal = Some(Modal::AppMenu { at: 0 });
            return cx.notify();
        }
        // The toolbar row: the explorer's title and its buttons, then the files panel's buttons and tabs.
        if let Some((_, _, id, _, _)) = self.nav_buttons().into_iter().filter(|_| (0.0..toolbar_h()).contains(&y))
            .chain(self.explorer_buttons().into_iter().filter(|_| (toolbar_h()..toolbar_h() + line_h()).contains(&y))).find(|b| (b.0..b.1).contains(&x))
        {
            return self.run(id, window, cx);
        }
        if y < toolbar_h() || (y < toolbar_h() + line_h() && x < self.sidebar_w) {
            return;
        }
        if let Some((pane, id)) = self.panes.iter().find_map(|g| self.preview_button(*g).filter(|(b, _)| b.contains(&ev.position)).map(|(_, action)| (g.pane, action.id))) {
            self.tree_focus = false;
            self.dispatch(Command::FocusPane(pane), cx);
            return self.run(id, window, cx);
        }
        // A pane's tab bar: a tab is shown (its pane focused) and can be dragged; elsewhere the click focuses the pane.
        if let Some(g) = self.panes.iter().find(|g| (g.bar.2..g.bar.2 + line_h()).contains(&y) && (g.bar.0..g.bar.1).contains(&x)).copied() {
            // Only this bar's tabs: bars one above the other start at different x, so their tabs overlap.
            let here = |id: &TabId| self.session.state().pane_tabs(g.pane).any(|t| t.id() == *id);
            match self.tab_hits.iter().find(|(x0, x1, id)| (*x0..*x1).contains(&x) && here(id)).copied() {
                // The tab's last cells are its close mark; a double-click keeps it open.
                Some((_, x1, id)) if x >= x1 - self.char_w * 2.0 => {
                    self.dispatch(Command::CloseTab(id), cx);
                }
                Some((_, _, id)) if ev.click_count >= 2 => {
                    self.dispatch(Command::Pin(id), cx);
                }
                Some((_, _, id)) => {
                    self.dispatch(Command::ActivateTab(id), cx);
                    self.drag = Some(Drag::Tab { id, x, y, moved: false });
                }
                None => {
                    self.dispatch(Command::FocusPane(g.pane), cx);
                }
            }
            return;
        }
        if (0.0..line_h()).contains(&y) && x >= self.sidebar_w {
            return; // the toolbar row past the buttons
        }
        if x < self.sidebar_w {
            self.tree_focus = true;
            // Rows as the tree is drawn: from below its title row, whatever the panes' bars and breadcrumbs take.
            let row = ((y - toolbar_h() - line_h()).max(0.0) / line_h()) as usize;
            let i = self.tree_scroll + row;
            if row < self.tree_rows && i < self.tree_len() {
                if ev.button == MouseButton::Left && !ev.modifiers.control && !ev.modifiers.platform {
                    self.terminal_ui.explorer_drag = self.session.state().tree_rows().nth(i).map(|r| (r.path.clone(), x, y, false));
                }
                // Ctrl+click (Cmd+click on macOS) marks it for Compare Selected; a plain click forgets the marks.
                if ev.modifiers.control || ev.modifiers.platform {
                    // The row drawn highlighted, which is the first one when nothing was picked yet.
                    let selected = self.session.state().tree_rows().nth(self.tree_at()).map(|r| r.path.clone());
                    let path = self.session.state().tree_rows().nth(i).map(|r| r.path.clone());
                    if let Some(path) = path {
                        self.tree_sel = Some(path.clone());
                        self.dispatch(Command::MarkTreeRow { path, selected }, cx);
                    }
                } else {
                    self.dispatch(Command::ClearTreeMarks, cx);
                    let clicked = self.session.state().tree_rows().nth(i).map(|row| (row.dir.is_none(), row.depth));
                    if let Some((file, depth)) = clicked {
                        let arrow = 6.0 + depth as f32 * 2.0 * self.char_w;
                        let activate = file || (arrow..arrow + 1.5 * self.char_w).contains(&x) || ev.click_count == 2;
                        if activate { self.activate_row(i, file && ev.click_count >= 2, cx); }
                        else { self.tree_move(i); }
                    }
                }
            }
            cx.notify();
            return;
        }
        self.tree_focus = false;
        // A click on an annotation opens its commit.
        if let Some((pane, tab, line)) = self.annotation_at(ev.position) {
            if pane != self.session.state().focused_pane() {
                self.dispatch(Command::FocusPane(pane), cx);
            }
            return self.dispatch(Command::ShowCommitAt { tab, line }, cx);
        }
        // The pane under the pointer takes focus first.
        if let Some(g) = self.panes.iter().find(|g| g.text.contains(&ev.position)).copied()
            && g.pane != self.session.state().focused_pane()
        {
            self.dispatch(Command::FocusPane(g.pane), cx);
            self.rows = g.rows;
            self.text_bounds = Some(g.text);
        }
        if self.focused_markdown().is_some() {
            // Its rendered text: a click places, a double-click takes a word, Shift extends, a drag selects; the core
            // learns the position (Show in Source, Follow Link), and Ctrl+click (Cmd+click) goes there at once.
            let Some(at) = self.preview_byte(x, y) else { return cx.notify() };
            if let Some(point) = self.preview_point(at) {
                if ev.modifiers.control || ev.modifiers.platform {
                    return self.dispatch(Command::ActivatePreview(point), cx);
                }
                self.dispatch(Command::PointPreview(point), cx);
            }
            let m = self.focused_markdown();
            let selection = match (m.and_then(|m| m.selection), m.and_then(|m| m.layout.clone())) {
                (Some((anchor, _)), _) if ev.modifiers.shift => Some((anchor, at)),
                (_, Some(layout)) if ev.click_count >= 2 => Some(word_around(&layout.preview.doc.text, at)),
                _ => Some((at, at)),
            };
            self.preview_drag = ev.click_count < 2;
            return self.select_preview(selection, cx);
        }
        if let Some(tab) = self.session.state().active()
            && let Some(bounds) = self.text_bounds
            && x >= f32::from(bounds.origin.x) && x < f32::from(bounds.origin.x) + 2.0 + self.char_w*2.0
            && let Some((line,_)) = self.byte_in(window,tab,bounds,x,y)
            && self.session.state().markers(tab.id()).is_some_and(|m|m.change(line).is_some() || m.deleted_above(line)>0)
        { return self.dispatch(Command::InspectChangeAt {tab:tab.id(),line},cx); }
        if let Some(line) = self.fold_click(window, x, y) {
            return self.dispatch(Command::ToggleFold(line), cx);
        }
        let Some(at) = self.byte_at(window, x, y) else { return };
        // Ctrl-click (Cmd-click on macOS) follows the definition there; an ordinary click only selects.
        if ev.modifiers.control || ev.modifiers.platform {
            self.dispatch(Command::Select(Selection { anchor: at, head: at }), cx);
            self.run(CommandId::Definition, window, cx);
            return;
        }
        if let Some(s) = self.session.state().selection().filter(|_| ev.modifiers.shift) {
            self.dragging = true;
            return self.dispatch(Command::Select(Selection { anchor: s.anchor, head: at }), cx);
        }
        // One click places the caret (a drag selects), two take the word, three its syntax group; then again.
        let clicks = ((ev.click_count.max(1) - 1) % 3 + 1) as u8;
        self.dragging = clicks == 1;
        self.dispatch(Command::ClickSelect { at, clicks }, cx);
    }

    /// A mouse side button: Back or Forward, unless a menu or dialog is open.
    fn side_button(&mut self, id: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        self.double_shift.interrupt();
        if self.modal.is_none() {
            self.run(id, window, cx);
        }
    }

    /// A right-click in a pane's text: that pane takes focus, the caret moves to the pointer unless it is inside
    /// the selection, and the context menu opens there.
    fn on_right_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_none() && self.activity_layout().is_some_and(|l| l.bounds.contains(&ev.position)) { return; }
        if self.modal.is_none() && self.input_state.capture.is_some() { self.cancel_input(true, cx); cx.stop_propagation(); return; }
        self.double_shift.interrupt();
        // A Git Branches row: its actions.
        if let Some(Modal::Branches { query, .. }) = self.modal.clone()
            && let Some(l) = self.modal_layout().filter(|l| l.bounds.contains(&ev.position))
        {
            let n = ((f32::from(ev.position.y) - f32::from(l.bounds.origin.y) - l.body_top) / line_h()).floor();
            let row = (n >= 0.0 && (n as usize) < l.visible).then(|| l.lines.get(l.first + n as usize).map(|r| r.0)).flatten().and_then(|r| r.checked_sub(FINDER_HEADER_ROWS));
            if let Some(i) = row
                && let Some(r) = self.session.state().branch_rows(&query).into_iter().nth(i)
                && !self.session.branch_actions(&r.target).is_empty()
            {
                self.modal = Some(Modal::BranchMenu { target: r.target, label: r.label, at: 0, query, row: i });
            }
            return cx.notify();
        }
        if self.modal.as_ref().is_some_and(|m| !matches!(m, Modal::Context { .. })) {
            return;
        }
        window.focus(&self.focus, cx);
        if self.terminal_click(ev, window, cx) { return; }
        self.blur_terminal(cx);
        if self.csv_field().is_some() { return; }
        self.close_modal(cx);
        if self.csv_click(ev, cx) {
            let entries = self.session.context_menu();
            self.modal = Some(Modal::Context { entries, at: 0, x: f32::from(ev.position.x), y: f32::from(ev.position.y), scroll: 0.0 });
            return cx.notify();
        }
        if self.session.state().documentation().is_some() {
            self.dispatch(Command::CloseDocumentation, cx);
        }
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(tab) = self.panes.iter().find_map(|g| {
            if !(g.bar.2..g.bar.2 + line_h()).contains(&y) || !(g.bar.0..g.bar.1).contains(&x) { return None; }
            self.tab_hits.iter().find(|(x0, x1, id)| (*x0..*x1).contains(&x) && self.session.state().pane_tabs(g.pane).any(|t| t.id() == *id)).map(|(_, _, id)| *id)
        }) {
            self.dispatch(Command::ActivateTab(tab), cx);
            self.tree_focus = false;
            self.results_focus = false;
            let mut entries = self.session.tab_menu();
            if !entries.contains(&MenuEntry::Command(CommandId::PasteTerminalPath)) { entries.push(MenuEntry::Command(CommandId::PasteTerminalPath)); }
            self.terminal_ui.path_context = Some(self.session.terminal_path_command(None));
            self.modal = Some(Modal::Context { entries, at: 0, x, y, scroll: 0.0 });
            return cx.notify();
        }
        // In the explorer: the row under the pointer is highlighted, and its menu offers what can be made there.
        if self.session.state().has_tree() && x < self.sidebar_w && y >= toolbar_h() + line_h() {
            let row = ((y - toolbar_h() - line_h()) / line_h()) as usize;
            let i = self.tree_scroll + row;
            if let Some(path) = self.session.state().tree_rows().nth(i).map(|r| r.path.clone()) {
                self.tree_sel = Some(path);
            }
            self.tree_focus = true;
            self.tree_shown = true;
            self.remember_explorer();
            let mut entries = self.session.tree_menu();
            if !entries.contains(&MenuEntry::Command(CommandId::PasteTerminalPath)) { entries.push(MenuEntry::Command(CommandId::PasteTerminalPath)); }
            self.terminal_ui.path_context = self.tree_sel.as_ref().map(|path| self.session.terminal_path_command(Some(path)));
            self.modal = Some(Modal::Context { entries, at: 0, x, y, scroll: 0.0 });
            return cx.notify();
        }
        // On the blame column: the caret goes to that line, so the commit commands act on it.
        if let Some((pane, tab, line)) = self.annotation_at(ev.position) {
            if pane != self.session.state().focused_pane() {
                self.dispatch(Command::FocusPane(pane), cx);
            }
            let start = self.session.state().presentation_view(tab).and_then(|view| view.snapshot().line_range(line)).map(|r| r.start);
            if let Some(start) = start {
                self.dispatch(Command::Select(Selection { anchor: start, head: start }), cx);
            }
            let entries = self.session.context_menu();
            let at = entries.iter().position(|e| matches!(e, MenuEntry::Command(_))).unwrap_or(0);
            self.modal = Some(Modal::Context { entries, at, x, y, scroll: 0.0 });
            return cx.notify();
        }
        let Some(g) = self.panes.iter().find(|g| g.text.contains(&ev.position)).copied() else { return };
        if g.pane != self.session.state().focused_pane() {
            self.dispatch(Command::FocusPane(g.pane), cx);
            self.rows = g.rows;
            self.text_bounds = Some(g.text);
        }
        if self.focused_markdown().is_none()
            && let Some(at) = self.byte_at(window, x, y)
            && !self.session.state().selection().is_some_and(|s| s.range().contains(&at))
        {
            self.dispatch(Command::Select(Selection { anchor: at, head: at }), cx);
        }
        // In a preview, the menu's Show in Source and Follow Link act where it was opened.
        if let Some(point) = self.preview_byte(x, y).and_then(|at| self.preview_point(at)) {
            self.dispatch(Command::PointPreview(point), cx);
        }
        let entries = self.session.context_menu();
        let at = entries.iter().position(|e| matches!(e, MenuEntry::Command(_))).unwrap_or(0);
        self.modal = Some(Modal::Context { entries, at, x, y, scroll: 0.0 });
        cx.notify();
    }

    /// The open context menu's box and its rows (top, height, entry), kept inside the window: it opens to the
    /// left of or above the pointer when there is no room to the right or below.
    fn context_layout(&self) -> Option<(Bounds<Pixels>, Vec<MenuRow>)> {
        let Some(Modal::Context { entries, x, y, scroll, .. }) = &self.modal else { return None };
        let label = |e: &MenuEntry| match e {
            MenuEntry::Command(id) => id.label().chars().count() + self.key_hint(*id).map_or(0, |k| k.chars().count() + 4),
            MenuEntry::Separator => 0,
        };
        let w = ((entries.iter().map(label).max().unwrap_or(0) as f32 + 4.0) * self.char_w).min(self.width.max(0.0));
        let h = (entries.iter().map(|e| context_row_height(*e)).sum::<f32>() + 8.0).min(self.height.max(0.0));
        let left = if x + w > self.width { x - w } else { *x }.clamp(0.0, (self.width - w).max(0.0));
        let top = if y + h > self.height { y - h } else { *y }.clamp(0.0, (self.height - h).max(0.0));
        let mut row = top + 4.0 - scroll;
        let rows = entries
            .iter()
            .map(|e| {
                let h = context_row_height(*e);
                row += h;
                (row - h, h, *e)
            })
            .collect();
        Some((Bounds { origin: gpui::point(px(left), px(top)), size: gpui::size(px(w), px(h)) }, rows))
    }

    fn clamp_context(&mut self) {
        if let Some(Modal::Context { entries, scroll, .. }) = &mut self.modal {
            let total: f32 = entries.iter().map(|e| context_row_height(*e)).sum();
            *scroll = scroll.clamp(0.0, (total - (self.height - 8.0).max(0.0)).max(0.0));
        }
    }

    fn reveal_context(&mut self) {
        let Some(Modal::Context { entries, at, scroll, .. }) = &mut self.modal else { return };
        let visible = (self.height - 8.0).max(0.0);
        let total: f32 = entries.iter().map(|e| context_row_height(*e)).sum();
        *scroll = scroll.clamp(0.0, (total - visible).max(0.0));
        let top: f32 = entries.iter().take(*at).map(|e| context_row_height(*e)).sum();
        let bottom = top + entries.get(*at).map_or(0.0, |e| context_row_height(*e));
        if top < *scroll { *scroll = top; }
        else if bottom > *scroll + visible { *scroll = (bottom - visible).max(0.0); }
    }

    /// The index of the open context menu's entry (a separator too) under `position`, if any.
    fn context_entry_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        let (b, rows) = self.context_layout()?;
        let y = f32::from(position.y);
        b.contains(&position).then(|| rows.iter().position(|(top, h, _)| (*top..top + h).contains(&y))).flatten()
    }

    /// The open context menu's box, for tests.
    pub fn context_bounds(&self) -> Option<Bounds<Pixels>> {
        self.context_layout().map(|(b, _)| b)
    }

    /// The top of the context menu's row labelled `label`, for tests.
    pub fn context_row_y(&self, label: &str) -> Option<f32> {
        self.context_layout()?.1.into_iter().find(|(_, _, e)| matches!(e, MenuEntry::Command(id) if id.label() == label)).map(|r| r.0)
    }

    /// The mouse pointer's shape: a resize arrow over or while dragging a divider, a hand over a link or a button, an
    /// I-beam over text; `None` keeps the platform's default.
    pub fn pointer(&self) -> Option<gpui::CursorStyle> {
        match self.drag.or(self.hover_divider) {
            Some(Drag::Explorer | Drag::Annotations { .. }) => Some(gpui::CursorStyle::ResizeLeftRight),
            Some(Drag::Split) if self.session.state().split() == Some(Split::Columns) => Some(gpui::CursorStyle::ResizeLeftRight),
            Some(Drag::Split | Drag::Results) => Some(gpui::CursorStyle::ResizeUpDown),
            None if self.link.is_some() || self.app_menu_hover || self.commands_hover || self.nav_hover.is_some() || self.preview_hover.is_some() => Some(gpui::CursorStyle::PointingHand),
            None if self.text_hover && self.modal.is_none() => Some(gpui::CursorStyle::IBeam),
            _ => None,
        }
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_none() && self.input_state.capture.is_some() { self.link = None; self.doc_rest = (None, None); return; }
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let over = self.annotation_at(ev.position);
        if over != self.ann_hovered() {
            cx.notify();
        }
        self.ann_hover = over.is_some().then_some(ev.position);
        let app_menu_hover = self.app_menu_button().contains(&ev.position);
        let commands_hover = self.modal.is_none() && self.commands_button().is_some_and(|(b, _)| b.contains(&ev.position));
        if (app_menu_hover, commands_hover) != (self.app_menu_hover, self.commands_hover) {
            self.app_menu_hover = app_menu_hover;
            self.commands_hover = commands_hover;
            cx.notify();
        }
        if matches!(self.modal, Some(Modal::AppMenu { .. } | Modal::CommandGroup { .. }))
            && let Some(l) = self.modal_layout().filter(|l| l.bounds.contains(&ev.position))
        {
            let n = ((y - f32::from(l.bounds.origin.y) - l.body_top) / line_h()).floor();
            if n >= 0.0 && (n as usize) < l.visible && let Some(&(row, ..)) = l.lines.get(l.first + n as usize)
                && let Some(Modal::AppMenu { at } | Modal::CommandGroup { at, .. }) = &mut self.modal
                && *at != row
            {
                *at = row;
                cx.notify();
            }
        }
        let modal_hover = if let Some(form) = self.form_layout() {
            form.buttons.into_iter().find(|b| b.contains(&ev.position))
        } else if let Some(Modal::StatusDetails { kind, .. }) = self.modal.as_ref() {
            self.modal_layout().and_then(|l| l.details_buttons(self.char_w, &self.details_labels(*kind)).into_iter().find(|b| b.contains(&ev.position)))
        } else { None };
        if self.modal_hover != modal_hover { self.modal_hover = modal_hover; cx.notify(); }
        let hover = self.modal.is_none().then(|| self.nav_buttons().into_iter().filter(|_| (0.0..toolbar_h()).contains(&y))
            .chain(self.explorer_buttons().into_iter().filter(|_| (toolbar_h()..toolbar_h() + line_h()).contains(&y))).find(|b| (b.0..b.1).contains(&x)).map(|b| b.2)).flatten().or_else(|| {
                if self.modal.is_none() && y >= self.height - self.status_h() && self.status_h() > 0.0 {
                    let (server, message, _, sql, markers) = self.status_segments();
                    if server.0.contains(&ev.position) { Some(CommandId::LanguageServer) }
                    else if sql.0.contains(&ev.position) { Some(CommandId::SqlDialect) }
                    else if markers.0.contains(&ev.position) || message.0.contains(&ev.position) { Some(CommandId::StatusDetails) } else { None }
                } else { None }
            });
        let preview_hover = self.modal.is_none().then(|| self.panes.iter().find(|g| self.preview_button(**g).is_some_and(|(b, _)| b.contains(&ev.position))).map(|g| g.pane)).flatten();
        if preview_hover != self.preview_hover {
            self.preview_hover = preview_hover;
            cx.notify();
        }
        if hover != self.nav_hover {
            self.nav_hover = hover;
            cx.notify();
        }
        // The pointer over a context-menu entry highlights it, as Up and Down do (a separator keeps the highlight).
        if let Some(i) = self.context_entry_at(ev.position)
            && let Some(Modal::Context { entries, at, .. }) = &mut self.modal
            && matches!(entries[i], MenuEntry::Command(_))
            && *at != i
        {
            *at = i;
            cx.notify();
        }
        if self.modal.is_some() {
            self.dragging = false;
            self.drag = None;
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            self.dragging = false; // released (perhaps outside the window)
        }
        if self.terminal_drag_move(ev, cx) { return; }
        self.hover_link(window, x, y, ev.modifiers.control || ev.modifiers.platform, cx);
        self.rest_pointer(window, ev, cx);
        let gutter = self.panes.iter().find(|g| g.text.contains(&ev.position) && x < f32::from(g.text.origin.x) + num_w()).map(|g| g.pane);
        let text_hover = self.panes.iter().any(|g| g.text.contains(&ev.position) && {
            let i = g.pane as usize;
            self.markdown[i].is_some() || self.source_geometry[i].is_some() && x >= f32::from(g.text.origin.x) + num_w()
        });
        if text_hover != self.text_hover {
            self.text_hover = text_hover;
            cx.notify();
        }
        if gutter != self.gutter_hover {
            self.gutter_hover = gutter;
            cx.notify();
        }
        let divider = self.divider_at(x, y);
        if divider != self.hover_divider {
            self.hover_divider = divider;
            cx.notify();
        }
        if let Some(drag) = self.drag {
            if ev.pressed_button != Some(MouseButton::Left) {
                return self.end_drag(cx); // released outside the window
            }
            self.drag_to(drag, x, y, cx);
            return cx.notify();
        }
        if self.preview_drag && ev.pressed_button == Some(MouseButton::Left) {
            let anchor = self.focused_markdown().and_then(|m| m.selection).map(|s| s.0);
            if let (Some(anchor), Some(at)) = (anchor, self.preview_byte(x, y)) {
                self.select_preview(Some((anchor, at)), cx);
            }
            return;
        }
        self.preview_drag = false;
        if !self.dragging || ev.pressed_button != Some(MouseButton::Left) {
            self.dragging = false;
            return;
        }
        let (Some(at), Some(s)) = (self.byte_at(window, f32::from(ev.position.x), f32::from(ev.position.y)), self.session.state().selection()) else { return };
        self.dispatch(Command::Select(Selection { head: at, ..s }), cx);
    }

    fn on_mouse_up(&mut self, ev: &gpui::MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.modal.is_some() { self.drag = None; self.terminal_ui.explorer_drag = None; self.terminal_ui.mouse_capture = None; return; }
        if self.terminal_drag_end(ev, cx) { self.drag = None; self.tab_drop = None; return; }
        self.preview_drag = false;
        if let Some(Drag::Scrollbar { pane, shown, moved: false, .. }) = self.drag && self.shown_buffer(pane) == Some(shown) { self.scrollbar_release(pane, f32::from(ev.position.y), cx); }
        self.end_drag(cx);
    }

    /// The divider at a window point, if any: the explorer's edge, the results panel's top, the split.
    fn divider_at(&self, x: f32, y: f32) -> Option<Drag> {
        let near = |a: f32, b: f32| (a - b).abs() <= 3.0;
        for g in &self.panes {
            if g.ann > 0.0 && near(x, f32::from(g.text.origin.x)) && y >= f32::from(g.text.top()) && y < f32::from(g.text.bottom()) {
                let tab = self.session.state().shown(g.pane)?;
                let view = self.session.state().view(tab.id())?;
                return Some(Drag::Annotations { pane: g.pane, tab: tab.id(), key: view.snapshot().key() });
            }
        }
        let below_toolbar = y >= toolbar_h() + line_h() && y < self.panel_top.min(self.body_bottom());
        if self.panel_top < f32::MAX && near(y, self.panel_top) {
            return Some(Drag::Results);
        }
        if self.sidebar_w > 0.0 && self.sidebar_w < self.width && near(x, self.sidebar_w) && below_toolbar {
            return Some(Drag::Explorer);
        }
        match self.session.state().split() {
            Some(Split::Columns) if near(x, self.split_at) && below_toolbar => Some(Drag::Split),
            Some(Split::Rows) if self.panes.len() > 1 && x > self.sidebar_w && near(y, self.split_at) => Some(Drag::Split),
            _ => None,
        }
    }

    /// Follow the pointer: a divider moves (sizes stay within reach of every panel), a tab shows where it would land.
    fn drag_to(&mut self, drag: Drag, x: f32, y: f32, cx: &mut Context<Self>) {
        let permille = |k: f32| Some((k * 1000.0).round() as u32);
        let top = toolbar_h() + line_h();
        match drag {
            Drag::Annotations { pane, tab, key } => {
                if self.session.state().shown(pane).is_none_or(|t| t.id() != tab) || self.session.state().view(tab).is_none_or(|v| v.snapshot().key() != key || v.tab().annotation_cols() == 0) { self.drag = None; return; }
                let Some(g) = self.panes.iter().find(|g| g.pane == pane) else { self.drag = None; return };
                let left = f32::from(g.text.origin.x) - g.ann;
                let max = ((f32::from(g.text.right()) - left - num_w() - 52.0) / self.char_w).floor().max(12.0) as usize;
                let width = ((x - left - 12.0) / self.char_w).round().max(12.0) as usize;
                self.dispatch(Command::ResizeAnnotations { width: width.min(max), commit: false }, cx);
            }
            Drag::Explorer => self.layout.explorer_width = Some(x.clamp(120.0, (self.width - 160.0).max(120.0)).round() as u32),
            Drag::Results => {
                let bottom = self.body_bottom();
                self.layout.results_permille = permille(((bottom - y) / (bottom - top).max(1.0)).clamp(0.05, 0.95));
            }
            Drag::Split => {
                let k = match self.session.state().split() {
                    Some(Split::Rows) => (y - top) / (self.panel_top.min(self.body_bottom()) - top - line_h()).max(1.0),
                    _ => (x - self.sidebar_w) / (self.width - self.sidebar_w).max(1.0),
                };
                self.layout.split_permille = permille(k.clamp(0.1, 0.9));
            }
            Drag::Tab { id, x: x0, y: y0, moved } => {
                let moved = moved || (x - x0).hypot(y - y0) > 6.0;
                self.drag = Some(Drag::Tab { id, x: x0, y: y0, moved });
                self.tab_drop = if moved { self.drop_target(id, x, y) } else { None };
            }
            Drag::Scrollbar { pane, shown, grab, y: y0, moved } => {
                if self.shown_buffer(pane) != Some(shown) { self.drag = None; return; }
                let moved = moved || (y - y0).abs() > 2.0;
                self.drag = Some(Drag::Scrollbar { pane, shown, grab, y: y0, moved });
                let Some(g) = self.panes.iter().find(|g| g.pane == pane).copied().filter(|_| moved) else { return };
                let Some((bar, _)) = self.scrollbar(&g) else { return };
                let len = f32::from(g.bar_strip.size.height);
                self.scrollbar_to(pane, bar.top_at(y - f32::from(g.bar_strip.origin.y) - grab, len, THUMB_MIN), cx);
            }
        }
    }

    /// Where tab `id` dropped at a window point would go: a pane's tab bar (between its tabs) or its text (last).
    fn drop_target(&self, id: TabId, x: f32, y: f32) -> Option<(u8, usize, f32)> {
        let g = self.panes.iter().find(|g| (g.bar.2..g.bar.2 + line_h()).contains(&y) && (g.bar.0..g.bar.1).contains(&x) || g.text.contains(&gpui::point(px(x), px(y))) || g.bar_strip.contains(&gpui::point(px(x), px(y))))?;
        let state = self.session.state();
        let in_bar = (g.bar.2..g.bar.2 + line_h()).contains(&y);
        if !in_bar && state.tabs().iter().any(|t| t.id() == id && t.pane() == g.pane) {
            return None; // over its own pane's text: it stays
        }
        let others: Vec<(f32, f32)> = state.pane_tabs(g.pane).filter(|t| t.id() != id).filter_map(|t| self.tab_hits.iter().find(|h| h.2 == t.id()).map(|h| (h.0, h.1))).collect();
        let index = if in_bar { others.iter().filter(|(a, b)| (a + b) / 2.0 < x).count() } else { others.len() };
        let mark = others.get(index).map_or_else(|| others.last().map_or(g.bar.0, |t| t.1), |t| t.0);
        Some((g.pane, index, mark))
    }

    /// A drag ends: a moved divider's sizes are kept (and remembered), a moved tab goes where it was dropped.
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        match self.drag.take() {
            Some(Drag::Annotations { pane, tab, key }) => {
                if self.session.state().shown(pane).is_some_and(|t| t.id() == tab) && self.session.state().view(tab).is_some_and(|v| v.snapshot().key() == key && v.tab().annotation_cols() != 0) {
                    self.dispatch(Command::ResizeAnnotations { width: self.layout.annotation_cols(), commit: true }, cx);
                }
            }
            Some(Drag::Tab { id, moved: true, .. }) => {
                if let Some((pane, index, _)) = self.tab_drop {
                    self.dispatch(Command::MoveTab { tab: id, pane, index }, cx);
                }
            }
            Some(Drag::Tab { .. } | Drag::Scrollbar { .. }) | None => {}
            Some(Drag::Explorer | Drag::Results | Drag::Split) => self.dispatch(Command::SetLayout(self.layout), cx),
        }
        self.tab_drop = None;
        cx.notify();
    }

    /// The tree's visible lines (from its scroll), with `selected` the highlighted row's index.
    pub fn tree_lines(&self, selected: usize) -> Vec<TreeLine> {
        self.session.state().tree_rows().enumerate().skip(self.tree_scroll).take(self.tree_rows).map(|(i, r)| TreeLine {
                depth: r.depth,
                expanded: r.dir,
                icon: match r.dir { Some(open) => icons::folder(open), None => icons::file(&r.name) },
                name: r.name.to_string(),
                highlight: self.session.state().explorer_search().ranges(&r.name).collect(),
                error: self.session.state().row_problem_count(r) > 0,
                selected: i == selected,
                marked: self.session.state().tree_marks().contains(&r.path),
                hidden: r.hidden(),
                status: self.session.state().row_status(r),
                dirty: self.session.state().row_dirty(r),
            }).collect()
    }

    fn app_menu_button(&self) -> Bounds<Pixels> {
        Bounds { origin: gpui::point(px(4.0), px(0.0)), size: gpui::size(px((self.char_w * 3.0).max(line_h()).min((self.width - 4.0).max(0.0))), px(toolbar_h())) }
    }

    fn commands_button(&self) -> Option<(Bounds<Pixels>, &'static str)> {
        if self.status_h() == 0.0 || self.width < 240.0 { return None; }
        let label = if self.width < 640.0 { "Commands" } else if cfg!(target_os = "macos") { "Commands  Cmd+Shift+P" } else { "Commands  Ctrl+Shift+P" };
        let w = (label.len() as f32 + 2.0) * self.char_w;
        Some((Bounds { origin: gpui::point(px(self.width - w - STATUS_PAD), px(self.height - self.status_h())), size: gpui::size(px(w), px(self.status_h())) }, label))
    }

    /// Independent footer cells; narrow windows keep recovery and message targets accessible.
    fn status_segments(&self) -> (StatusCell, StatusCell, StatusCell, StatusCell, StatusCell) {
        let left = STATUS_PAD.min(self.width / 2.0);
        let right = self.results_button().or_else(|| self.activity_button()).map_or(self.width - left, |(b, _)| f32::from(b.origin.x) - STATUS_PAD).max(left);
        let room = right - left;
        let position = self.session.status_position();
        let position_w = if self.width >= 480.0 { (12.0 * self.char_w).min(room * 0.3) } else { 0.0 };
        let label = self.session.state().language_server_label();
        // The SQL dialect (or "not set") beside the server status, never assumed.
        let sql = self.session.sql_dialect_label().unwrap_or_default();
        let server_w = ((lenz_content::wrap::width(&label) as f32 + 2.0) * self.char_w).min(room * if sql.is_empty() { 0.5 } else { 0.4 }).min(room - position_w);
        let sql_w = if sql.is_empty() { 0.0 } else { ((lenz_content::wrap::width(&sql) as f32 + 2.0) * self.char_w).min(room * 0.35).min((room - position_w - server_w).max(0.0)) };
        // The open draft's change-marker baseline: which version its gutter letters are compared against.
        let markers = self.session.markers_baseline().map(|b| format!("vs {}", b.short())).unwrap_or_default();
        let markers_w = if markers.is_empty() || self.width < 480.0 { 0.0 } else { ((lenz_content::wrap::width(&markers) as f32 + 2.0) * self.char_w).min(room * 0.15).min((room - position_w - server_w - sql_w).max(0.0)) };
        let cell = |x: f32, w: f32, text: &str| {
            let cols = (w.max(0.0) / self.char_w).floor() as usize;
            (Bounds { origin: gpui::point(px(x), px(self.height - self.status_h())), size: gpui::size(px(w.max(0.0)), px(self.status_h())) }, code_core::chrome::ellipsize(text, cols))
        };
        let message_w = (room - server_w - sql_w - markers_w - position_w).max(0.0);
        (cell(left + position_w, server_w, &label), cell(left + position_w + server_w + sql_w + markers_w, message_w, &self.session.status_message()), cell(left, position_w, &position), cell(left + position_w + server_w, sql_w, &sql), cell(left + position_w + server_w + sql_w, markers_w, &markers))
    }

    /// Toolbar buttons share the core's command order; omitted buttons remain in Commands.
    pub fn nav_buttons(&self) -> Vec<(f32, f32, CommandId, bool, String)> {
        let mut x = f32::from(self.app_menu_button().right()) + 6.0;
        self.session.toolbar().into_iter().filter_map(|(id, label)| {
            let label = if id == CommandId::GitBranches { self.session.branch_label().unwrap_or_else(|| label.into()) } else { label.into() };
            let room = (self.width - x - 4.0).max(0.0);
            let label = code_core::chrome::ellipsize(&label, ((room / self.char_w).floor() as usize).saturating_sub(2));
            if label.is_empty() { return None; }
            if matches!(id, CommandId::Open | CommandId::ImageSource) { x += 8.0; }
            let drawn = matches!(id, CommandId::ToggleExplorer | CommandId::Back | CommandId::Forward);
            let end = x + if drawn { (self.char_w * 3.0).max(line_h()) } else { (lenz_content::wrap::width(&label) as f32 + 2.0) * self.char_w };
            let button = (x, end, id, self.session.disabled(id).is_none(), label);
            x = end + 4.0;
            (end <= self.width - 4.0).then_some(button)
        }).collect()
    }

    /// Reserve the right edge of the file header; narrow panes keep the keyboard/menu action.
    pub fn preview_button(&self, pane: PaneGeom) -> Option<(Bounds<Pixels>, code_core::session::Action)> {
        let control = self.session.csv_control(pane.pane).or_else(|| self.session.preview_control(pane.pane))?;
        let width = 9.0 * self.char_w;
        let x = pane.bar.1 - width - 4.0;
        if x < pane.bar.0 + 8.0 * self.char_w || pane.bar.2 + line_h() > self.body_bottom() { return None; }
        Some((Bounds { origin: gpui::point(px(x), px(pane.bar.2)), size: gpui::size(px(width), px(line_h())) }, control))
    }

    /// The explorer's title-row buttons, in window coordinates (as [`Self::nav_buttons`]): select the opened file.
    pub fn explorer_buttons(&self) -> Vec<(f32, f32, CommandId, bool, String)> {
        if self.sidebar_w <= 0.0 {
            return Vec::new();
        }
        let id = CommandId::LocateFile;
        vec![(self.sidebar_w - 4.0 - line_h(), self.sidebar_w - 4.0, id, self.session.disabled(id).is_none(), String::new())]
    }

    /// Scroll the preview to `to` pixels (clamped), reporting the reading line it now shows to the core.
    fn focused_markdown(&self) -> Option<&PreviewView> {
        self.markdown[self.session.state().focused_pane() as usize].as_ref()
    }

    /// The focused preview's rendered-text byte under a window point.
    fn preview_byte(&self, x: f32, y: f32) -> Option<usize> {
        let (m, b) = (self.focused_markdown()?, self.text_bounds?);
        m.layout.as_ref()?.byte_at(x - f32::from(b.origin.x), y - f32::from(b.origin.y) + m.top)
    }

    /// Rendered byte `at` of the focused preview's layout as the core names it: the tab and the version that layout
    /// was made from (a layout standing in for a newer preview names its own, older version).
    fn preview_point(&self, at: usize) -> Option<code_core::session::PreviewPoint> {
        let m = self.focused_markdown()?;
        let layout = m.layout.as_ref()?;
        Some(code_core::session::PreviewPoint { tab: m.tab, version: layout.preview.version, rendered: at })
    }

    /// The focused preview's selected rendered text (empty when nothing is selected).
    fn preview_text(&self) -> Option<String> {
        let m = self.focused_markdown()?;
        let layout = m.layout.as_ref()?;
        Some(m.selection.map(|(a, h)| layout.text_of(a.min(h)..a.max(h))).unwrap_or_default())
    }

    /// Select the focused preview's rendered text; on Linux a non-empty selection becomes the primary selection.
    fn select_preview(&mut self, selection: Option<(usize, usize)>, cx: &mut Context<Self>) {
        let i = self.session.state().focused_pane() as usize;
        let Some(m) = self.markdown[i].as_mut() else { return };
        if m.selection == selection {
            return;
        }
        m.selection = selection;
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if let Some(text) = self.preview_text().filter(|t| !t.is_empty()) {
            cx.write_to_primary(ClipboardItem::new_string(text));
        }
        cx.notify();
    }

    /// For tests: the window point over the second character of `word`'s first appearance in the focused preview.
    pub fn preview_point_of(&self, word: &str) -> Option<(f32, f32)> {
        self.preview_point_in(self.session.state().focused_pane(), word)
    }

    /// For tests: the window point over the second character of `word`'s first appearance in pane `pane`'s preview.
    pub fn preview_point_in(&self, pane: u8, word: &str) -> Option<(f32, f32)> {
        let m = self.markdown.get(pane as usize)?.as_ref()?;
        let b = self.panes.iter().find(|g| g.pane == pane)?.text;
        let layout = m.layout.as_ref()?;
        let text = &layout.preview.doc.text;
        layout.rows.iter().flat_map(|r| r.pieces.iter().map(move |p| (r, p))).find_map(|(r, p)| {
            let at = text.get(p.range.clone())?.find(word)?;
            let x = f32::from(b.origin.x) + p.x + f32::from(p.shaped.x_for_index(at + 1));
            Some((x, f32::from(b.origin.y) + r.y - m.top + r.h / 2.0))
        })
    }

    /// For tests: the window point over byte `at` of the focused text, on the first shown line, unwrapped, ASCII.
    pub fn text_point_of(&self, at: usize) -> Option<(f32, f32)> {
        let b = self.text_bounds?;
        let line = self.session.state().presentation_view(self.session.state().active()?.id())?.snapshot().line_range(0)?;
        let x = f32::from(b.origin.x) + num_w() + 8.0 + (at.checked_sub(line.start)? as f32 + 0.5) * self.char_w;
        Some((x, f32::from(b.origin.y) + line_h() / 2.0))
    }

    /// Pane `g`'s scrollbar: the core's over shown lines, with its marks; a Markdown preview's (pixels) or a CSV
    /// table's (rows) measured here, without marks. `None` without a strip or anything to measure.
    fn scrollbar(&self, g: &PaneGeom) -> Option<(Scrollbar, Vec<Mark>)> {
        if g.bar_strip.size.width <= px(0.0) { return None; }
        let state = self.session.state();
        if let Some(bar) = state.scrollbar(g.pane) { return Some(bar); }
        if let Some(view) = state.shown(g.pane).and_then(|t| state.csv(t.id())) {
            return Some((Scrollbar { total: view.row_count() as f64, top: view.top as f64, visible: view.rows.max(1) as f64 }, Vec::new()));
        }
        let m = self.markdown[g.pane as usize].as_ref()?;
        Some((Scrollbar { total: f64::from(m.layout.as_ref()?.height), top: f64::from(m.top), visible: f64::from(g.rows as f32 * line_h()) }, Vec::new()))
    }

    /// Scroll `pane` so its [`Self::scrollbar`] top is `top`: a table's row, a preview's pixel, else a shown line.
    fn scrollbar_to(&mut self, pane: u8, top: f64, cx: &mut Context<Self>) {
        let state = self.session.state();
        let Some(tab) = state.shown(pane) else { return };
        if let Some(view) = state.csv(tab.id()) {
            let rows = top.round() as isize - view.top as isize;
            return self.dispatch(Command::Csv(code_core::csv::Command::Scroll { rows, columns: 0 }), cx);
        }
        if self.markdown[pane as usize].as_ref().is_some_and(|m| m.layout.is_some()) { return self.scroll_preview(pane, top as f32, cx); }
        let Some(line) = state.view(tab.id()).map(|view| view.visible().line_at_row(top.round().max(0.0) as usize)) else { return };
        self.dispatch(Command::Scroll(line), cx);
    }

    /// A press on pane `g`'s scrollbar at window `y`, its pane focused first (as the wheel does): the thumb starts a
    /// drag, a mark goes to its line (Back returns), the track pages toward the press.
    fn scrollbar_press(&mut self, g: PaneGeom, y: f32, cx: &mut Context<Self>) {
        if g.pane != self.session.state().focused_pane() {
            self.dispatch(Command::FocusPane(g.pane), cx);
            self.rows = g.rows;
            self.text_bounds = Some(g.text);
        }
        let Some((bar, marks)) = self.scrollbar(&g) else { return };
        let (at, len) = (y - f32::from(g.bar_strip.origin.y), f32::from(g.bar_strip.size.height));
        let placed = scrollbar::place(&marks, bar.total, track_cells(len));
        match scrollbar::hit(&bar, &placed, &marks, at, len, THUMB_MIN) {
            Hit::Thumb => {
                let Some(shown) = self.shown_buffer(g.pane) else { return };
                let start = bar.thumb(len, THUMB_MIN).map_or(0.0, |t| t.0);
                self.drag = Some(Drag::Scrollbar { pane: g.pane, shown, grab: at - start, y, moved: false });
            }
            hit => {
                // the keys act on the text after a mark or a page, as after a click on the text
                (self.tree_focus, self.results_focus) = (false, false);
                match hit {
                    Hit::Mark(i) => self.go_to_mark(marks[i].target, cx),
                    Hit::Above => self.scrollbar_to(g.pane, bar.page(false), cx),
                    _ => self.scrollbar_to(g.pane, bar.page(true), cx),
                }
            }
        }
    }

    /// A thumb released where it was pressed: the mark under it, if any, as a click on the mark.
    fn scrollbar_release(&mut self, pane: u8, y: f32, cx: &mut Context<Self>) {
        let Some(g) = self.panes.iter().find(|g| g.pane == pane).copied() else { return };
        let Some((bar, marks)) = self.scrollbar(&g) else { return };
        let len = f32::from(g.bar_strip.size.height);
        let placed = scrollbar::place(&marks, bar.total, track_cells(len));
        if let Some(i) = scrollbar::mark_at(&placed, &marks, y - f32::from(g.bar_strip.origin.y), len) { self.go_to_mark(marks[i].target, cx); }
    }

    /// What pane `pane` shows: its tab and that tab's file.
    fn shown_buffer(&self, pane: u8) -> Option<(TabId, code_core::text::BufferId)> {
        let state = self.session.state();
        let tab = state.shown(pane)?.id();
        Some((tab, state.view(tab)?.snapshot().key().buffer))
    }

    fn go_to_mark(&mut self, line: usize, cx: &mut Context<Self>) {
        self.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: u32::try_from(line + 1).unwrap_or(u32::MAX), column: None }), cx);
    }

    /// Scroll pane `pane`'s preview to pixel `to`; the source follows when the pane is focused (a session scroll moves
    /// the active tab).
    fn scroll_preview(&mut self, pane: u8, to: f32, cx: &mut Context<Self>) {
        let focused = pane == self.session.state().focused_pane();
        let Some(rows) = self.panes.iter().find(|g| g.pane == pane).map(|g| g.rows) else { return };
        let view = rows as f32 * line_h();
        let Some(m) = self.markdown[pane as usize].as_mut() else { return };
        let Some(layout) = &m.layout else { return };
        m.top = to.min(layout.height - view).max(0.0);
        let line = layout.row_at(m.top).map(|r| r.line);
        cx.notify();
        if let Some(line) = line.filter(|&l| focused && l != m.line) {
            m.line = line;
            self.dispatch(Command::Scroll(line), cx);
        }
    }

    /// The reading style for the current palette.
    fn reading_style(&mut self, window: &Window) -> Arc<lenz_gpui::markdown::ReadingStyle> {
        let palette = self.session.state().palette().clone();
        if let Some((p, style)) = &self.reading
            && Arc::ptr_eq(p, &palette)
        {
            return style.clone();
        }
        let installed = window.text_system().all_font_names();
        let body = gpui::font(lenz_gpui::markdown::sans_family(fonts().reading_font.as_deref(), &installed).to_string());
        let style = Arc::new(lenz_gpui::markdown::ReadingStyle::new(&palette, self.font.clone(), body, font_size()));
        self.reading = Some((palette, style.clone()));
        style
    }

    /// The preview to draw now, laid out for the text area `width` wide and scrolled to the core's reading line
    /// when it moved; textures for its visible images (made off the UI thread). `None` when no preview is shown.
    fn preview_frame(&mut self, pane: u8, window: &Window, width: f32, rows: usize, cx: &mut Context<Self>) -> Option<PreviewFrame> {
        let state = self.session.state();
        let i = pane as usize;
        let shown = state.presentation_shown(pane);
        let source = shown.and_then(|tab| state.presentation_view(tab.id()));
        let (Some(Rendering::Preview(p)), Some(tab), Some(doc)) = (source.as_ref().and_then(|v| v.rendering()), shown, source.as_ref().map(|v| v.snapshot())) else {
            self.markdown[i] = None; // its layout and textures go with it
            return None;
        };
        let (p, line, tab, doc, palette) = (p.clone(), tab.top_line(), tab.id(), doc.clone(), state.palette().clone());
        match self.markdown[i].as_mut() {
            Some(m) if m.tab == tab && m.doc.ptr_eq(&Arc::downgrade(&p.doc)) => {}
            // The same tab's next version (an edit followed): its layout stands in until the new one is shaped, so
            // a preview beside the source never blanks while typing; the old selection does not carry over.
            Some(m) if m.tab == tab && m.layout.is_some() => {
                m.doc = Arc::downgrade(&p.doc);
                m.selection = None;
                m.resized = None;
            }
            _ => self.markdown[i] = Some(PreviewView { tab, doc: Arc::downgrade(&p.doc), layout: None, building: None, top: 0.0, line: usize::MAX, textures: Default::default(), resized: None, settle: None, selection: None }),
        }
        let style = self.reading_style(window);
        let m = self.markdown[i].as_mut()?;
        let fresh = m.layout.as_ref().is_some_and(|l| Arc::ptr_eq(&l.preview, &p) && l.width == width && Arc::ptr_eq(&l.style, &style) && Arc::ptr_eq(&l.palette, &palette));
        let building = m.building.as_ref().is_some_and(|(q, w, _)| q.upgrade().is_some_and(|q| Arc::ptr_eq(&q, &p)) && *w == width);
        // Only the width changed: the current layout stands in until the new width has held for SETTLE.
        let resizing = !fresh && m.layout.as_ref().is_some_and(|l| Arc::ptr_eq(&l.preview, &p) && Arc::ptr_eq(&l.style, &style) && Arc::ptr_eq(&l.palette, &palette));
        let settled = match m.resized {
            Some((w, since)) if w == width => since.elapsed() >= SETTLE,
            _ => false,
        };
        if resizing && !settled && !building {
            if m.resized.is_none_or(|(w, _)| w != width) {
                m.resized = Some((width, std::time::Instant::now()));
                m.settle = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SETTLE).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                }));
            }
        } else if !fresh && !building {
            m.resized = None;
            // Shaped with a text system of its own: the window's per-frame layout cache is for the frame drawn.
            let ts = gpui::WindowTextSystem::new(cx.text_system().clone());
            let (job, weak) = (p.clone(), Arc::downgrade(&p));
            let task = cx.spawn(async move |this, cx| {
                let layout = cx.background_executor().spawn(async move { preview::Layout::new(job, width, style, palette, &|b| doc.line_of(b), &ts) }).await;
                let _ = this.update(cx, |v, cx| {
                    if let Some(m) = v.markdown[i].as_mut().filter(|m| m.building.as_ref().is_some_and(|(q, w, _)| q.ptr_eq(&Arc::downgrade(&layout.preview)) && *w == layout.width)) {
                        m.building = None;
                        m.layout = Some(Arc::new(layout));
                        m.line = usize::MAX; // placed at the reading line again
                        cx.notify();
                    }
                });
            });
            m.building = Some((weak, width, task));
        }
        let layout = m.layout.clone()?;
        let view = rows as f32 * line_h();
        if m.line != line {
            m.top = layout.y_of_line(line);
            m.line = line;
        }
        m.top = m.top.min(layout.height - view).max(0.0);
        // Textures for the images in view; the others' are released.
        let visible: Vec<usize> = layout.visible(m.top, view).iter().filter_map(|r| match r.kind { preview::RowKind::Image { block, .. } => Some(block), _ => None }).collect();
        m.textures.retain(|b, _| visible.contains(b));
        let mut shown = Vec::new();
        for b in visible {
            let Some(Ok(pixels)) = layout.preview.images.as_ref().and_then(|i| i.get(&b)) else { continue };
            match m.textures.get(&b) {
                Some((weak, texture)) if weak.upgrade().is_some_and(|q| Arc::ptr_eq(&q, pixels)) => shown.extend(texture.clone().map(|t| (b, t))),
                _ => {
                    let (pixels, weak) = (pixels.clone(), Arc::downgrade(pixels));
                    m.textures.insert(b, (weak.clone(), None));
                    cx.spawn(async move |this, cx| {
                        let texture = cx.background_executor().spawn(async move { lenz_gpui::image::texture((*pixels).clone()) }).await;
                        let _ = this.update(cx, |v, cx| {
                            if let Some(slot) = v.markdown[i].as_mut().and_then(|m| m.textures.get_mut(&b)).filter(|(w, _)| w.ptr_eq(&weak)) {
                                slot.1 = Some(texture);
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
            }
        }
        let selection = m.selection.map(|(a, h)| a.min(h)..a.max(h));
        Some((layout, m.top, shown, selection))
    }

    /// The preview's rows in view, for tests: each row's source line and text.
    pub fn preview_rows(&self) -> Vec<(usize, String)> {
        self.preview_rows_in(self.session.state().focused_pane())
    }

    /// Pane `pane`'s preview rows in view, for tests: each row's source line and text.
    pub fn preview_rows_in(&self, pane: u8) -> Vec<(usize, String)> {
        let Some((m, layout)) = self.markdown.get(pane as usize).and_then(|m| m.as_ref()).and_then(|m| Some((m, m.layout.as_ref()?))) else { return Vec::new() };
        let rows = self.panes.iter().find(|g| g.pane == pane).map_or(self.rows, |g| g.rows);
        let doc = &layout.preview.doc;
        layout.visible(m.top, rows as f32 * line_h()).iter().map(|r| (r.line, r.pieces.iter().map(|p| doc.text.get(p.range.clone()).unwrap_or_default()).collect::<Vec<_>>().join(" | "))).collect()
    }

    /// The width the preview is laid out for, for tests.
    pub fn preview_width(&self) -> Option<f32> {
        self.focused_markdown()?.layout.as_ref().map(|l| l.width)
    }

    /// The preview's scroll in pixels and image textures held, for tests.
    pub fn preview_state(&self) -> Option<(f32, usize)> {
        self.focused_markdown().map(|m| (m.top, m.textures.values().filter(|t| t.1.is_some()).count()))
    }

    /// Rows for the visible lines: display text, syntax colours and the selection in display offsets.
    pub fn visible_rows(&self) -> Vec<TextRow> {
        let state = self.session.state();
        state.active().map_or_else(Vec::new, |t| {
            let mut rows = self.tab_rows(t, self.rows).0;
            for row in &mut rows { row.number = display_number(row.number, state.relative_number_origin(t.id())); }
            rows
        })
    }

    /// Rows for `tab`'s first `rows` visible lines.
    /// Lay out each breadcrumb bar (`(pane, x, y, width)`): crumbs and a `›` after each, right to left from the
    /// innermost; when they do not fit, folders and then outer declarations shrink to one "…" each run, keeping the
    /// file and the innermost declaration. Returns the hits and the cells to paint (a row's background first).
    fn layout_crumbs(&self, rows: &[(u8, f32, f32, f32)]) -> (Vec<CrumbHit>, Vec<CrumbCell>) {
        let state = self.session.state();
        let palette = state.palette();
        let cw = self.char_w;
        let (mut hits, mut cells) = (Vec::new(), Vec::new());
        for &(pane, x, y, w) in rows {
            cells.push(CrumbCell { x, y, w, text: String::new(), color: palette.dim, bg: Some(palette.bg) });
            let Some(bar) = state.breadcrumbs_for(pane) else { continue };
            let names: Vec<String> = bar.crumbs.iter().map(|c| c.name().to_string()).collect();
            let n = names.len();
            let width = |label: &str| label.chars().count() as f32 * cw + 8.0;
            let chevron = 2.0 * cw;
            let note = if bar.large { "large file" } else { "" };
            let avail = w - 8.0 - if note.is_empty() { 0.0 } else { width(note) };
            let kept = bar.fit(width, chevron, avail, self.crumb_focus.filter(|f| f.0 == pane).map(|f| f.1));
            let mut at = x + 4.0;
            let mut gap = false;
            for i in 0..n {
                let (label, crumb) = if kept[i] {
                    gap = false;
                    (names[i].clone(), Some(i))
                } else if !gap {
                    gap = true;
                    ("…".to_string(), None)
                } else {
                    continue;
                };
                let lw = width(&label);
                let focused = crumb.is_some() && self.crumb_focus == Some((pane, i));
                let color = if crumb == Some(n - 1) { palette.fg } else { palette.dim };
                hits.push(CrumbHit { bounds: Bounds { origin: gpui::point(px(at), px(y)), size: gpui::size(px(lw), px(line_h())) }, pane, crumb, chevron: false, label: label.clone() });
                cells.push(CrumbCell { x: at, y, w: lw, text: label, color, bg: focused.then_some(palette.selected_bg) });
                at += lw;
                if let Some(i) = crumb {
                    hits.push(CrumbHit { bounds: Bounds { origin: gpui::point(px(at), px(y)), size: gpui::size(px(chevron), px(line_h())) }, pane, crumb: Some(i), chevron: true, label: "›".into() });
                }
                cells.push(CrumbCell { x: at - 4.0, y, w: chevron, text: "›".into(), color: palette.dim, bg: None });
                at += chevron;
            }
            if !note.is_empty() {
                cells.push(CrumbCell { x: at, y, w: width(note), text: note.into(), color: palette.dim, bg: None });
            }
        }
        (hits, cells)
    }

    /// Follow crumb `i` of pane `pane`'s bar: a folder or file shows in the explorer, a declaration gets the caret.
    fn follow_crumb(&mut self, pane: u8, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.session.state().breadcrumbs_for(pane).and_then(|b| b.crumbs.get(i).cloned()) else { return };
        if pane != self.session.state().focused_pane() {
            self.dispatch(Command::FocusPane(pane), cx);
        }
        match c {
            code_core::session::Crumb::Folder { rel, .. } => self.show_in_tree(rel, cx),
            code_core::session::Crumb::File { .. } => self.run(CommandId::LocateFile, window, cx),
            code_core::session::Crumb::Decl { line, .. } => self.go_to(line, cx),
        }
    }

    /// Follow an entry of a crumb's list.
    fn follow_sibling(&mut self, target: code_core::session::SiblingTarget, cx: &mut Context<Self>) {
        match target {
            code_core::session::SiblingTarget::Line(line) => self.go_to(line, cx),
            code_core::session::SiblingTarget::Folder(rel) => self.show_in_tree(rel, cx),
            code_core::session::SiblingTarget::File(rel) => {
                if let Some(path) = self.session.state().project().map(|p| p.root().join(&*rel)) {
                    self.open(path, cx);
                }
            }
        }
    }

    /// The caret to the start of source line `line` (history and unfolding as Go to Line).
    fn go_to(&mut self, line: usize, cx: &mut Context<Self>) {
        let line = u32::try_from(line + 1).unwrap_or(u32::MAX);
        self.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line, column: None }), cx);
    }

    /// A project folder or file opened and selected in the explorer, which takes the keys.
    fn show_in_tree(&mut self, rel: std::sync::Arc<str>, cx: &mut Context<Self>) {
        self.dispatch(Command::RevealInTree(rel.clone()), cx);
        self.tree_sel = Some(rel);
        self.tree_reveal = true;
        self.tree_shown = true;
        self.tree_focus = true;
        self.results_focus = false;
        cx.notify();
    }

    /// The entries of a crumb's list that hold `query` (any case).
    fn sibling_items(&self, pane: u8, crumb: usize, query: &str) -> Vec<code_core::session::Sibling> {
        let q = query.to_lowercase();
        self.session.state().siblings(pane, crumb).into_iter().filter(|s| s.label.to_lowercase().contains(&q)).collect()
    }

    fn source_row(&self, tab: &Tab, row: &code_core::layout::Row) -> Option<&source::Row> {
        let state = self.session.state();
        let view = state.presentation_view(tab.id())?;
        let tab = view.tab();
        self.source.drawn(tab.pane(), &view, row.line, state.wrap_cols(tab).map(|_| row.display.start))
    }

    fn ready_source_row(&self, tab: &Tab, row: &code_core::layout::Row) -> Option<&source::Row> {
        let state = self.session.state();
        let view = state.presentation_view(tab.id())?;
        let tab = view.tab();
        let g = self.panes.iter().find(|g| g.pane == tab.pane())?;
        let width = (f32::from(g.text.size.width) - num_w() - 16.0).max(1.0);
        let pan = tab.column() as f32 * self.char_w;
        if state.wrap_cols(tab).is_some() {
            self.source.get_wrapped(g.pane, &view, row.line, row.display.start, pan - row.indent as f32 * self.char_w, width).filter(|row| !row.pending())
        } else { self.source.get(g.pane, &view, row.line, pan, width).filter(|row| !row.pending()) }
    }

    fn source_parts(&self, tab: &Tab, logical: &code_core::layout::Row, row: &TextRow, window: &Window) -> Vec<(std::ops::Range<usize>, f32)> {
        let Some(native) = self.source_row(tab, logical) else { return Vec::new() };
        let indent = logical.indent;
        let offset = indent as f32 * self.char_w;
        let correction = tab.column() as f32 * self.char_w - (native.skip() + offset);
        let mut parts = Vec::new();
        if indent > 0 { parts.push((0..indent, correction)); }
        parts.extend(native.fragments.iter().map(|(r, x)| (r.start + indent..r.end + indent, x + offset + correction)));
        if row.text.len() > native.text.len() + indent {
            parts.push((native.text.len() + indent..row.text.len(), native.x_for(window, self.metrics(), native.text.len()).unwrap_or(0.0) + offset + correction));
        }
        if parts.is_empty() { parts.push((0..0, offset + correction)); }
        parts
    }

    fn prepare_source(&mut self, panes: &[PaneGeom], window: &mut Window) {
        use code_core::layout::Query;
        let font = self.font.clone();
        let metrics = metrics(&font);
        self.source.begin(metrics);
        for g in panes {
            if self.session.state().shown(g.pane).is_some_and(|t| self.session.state().csv(t.id()).is_some()) { continue; }
            let context = self.source.context(g.pane);
            if self.session.native_context(g.pane) != Some(context)
                && self.session.dispatch(Command::SetNativeContext { pane: g.pane, context }).repaint { window.request_animation_frame(); }
        }
        let mut commands = Vec::new();
        for g in panes {
            let state = self.session.state();
            let Some(tab) = state.presentation_shown(g.pane) else { continue };
            if state.csv(tab.id()).is_some() || g.rows == 0 || matches!(self.session.state().presentation_view(tab.id()).and_then(|v| v.rendering()), Some(Rendering::Preview(_) | Rendering::Pending)) { continue; }
            let Some(view) = state.presentation_view(tab.id()) else { continue };
            if wanted_image(&view).is_some() || unshowable_image(&view).is_some() { continue; }
            let cols = state.wrap_cols(tab);
            let layout = view.layout_rows(g.rows, cols);
            let mut pending = layout.pending;
            let mut rows = layout.rows;
            let batch = self.session.projection(g.pane).cloned();
            if let Some(goal) = self.session.native_reveal(g.pane).cloned() {
                self.source.prioritize(&goal);
                let wrapped = if cols.is_some() {
                    match view.layout_position(goal.source, cols) {
                        Query::Ready(Some(position)) => match view.layout_row(position.line, position.row, cols) {
                            Query::Ready(Some(row)) => match view.layout_cursor(row.line, row.display.start, cols) {
                                Query::Ready(Some(cursor)) => Some((row, cursor)),
                                Query::Ready(None) => None,
                                Query::Pending(work) => { pending = Some(work); None },
                            },
                            Query::Ready(None) => None,
                            Query::Pending(work) => { pending = Some(work); None },
                        },
                        Query::Ready(None) => None,
                        Query::Pending(work) => { pending = Some(work); None },
                    }
                } else { None };
                if (cols.is_none() || wrapped.is_some())
                    && let Some(bounds) = self.source.reveal(&view, &goal, wrapped.as_ref().map(|(row, cursor)| (row, cursor.clone())), batch.as_deref(), window, metrics, self.char_w)
                    && let Some((cols, _)) = state.text_size(g.pane) {
                    commands.push(Command::MeasuredReveal { goal, cols, bounds });
                }
            }
            if state.sticky_scroll() {
                match view.layout_sticky(code_core::tabs::sticky_cap(g.rows), cols) {
                    Query::Ready(sticky) => for pinned in sticky.iter() {
                        match view.layout_row(pinned.line, 0, cols) {
                            Query::Ready(Some(row)) => rows.push(row),
                            Query::Ready(None) => {},
                            Query::Pending(work) => { pending = pending.or(Some(work)); },
                        }
                    },
                    Query::Pending(work) => { pending = pending.or(Some(work)); },
                }
            }
            let width = (f32::from(g.text.size.width) - num_w() - 16.0).max(1.0);
            for row in rows {
                if cols.is_some() {
                    let cursor = self.source.cursor(g.pane, &view, row.line, row.display.start);
                    let cursor = if cursor.is_some() { cursor } else {
                        match view.layout_cursor(row.line, row.display.start, cols) {
                            Query::Ready(cursor) => cursor,
                            Query::Pending(work) => { pending = pending.or(Some(work)); None },
                        }
                    };
                    if let Some(cursor) = cursor {
                        self.source.wrapped(g.pane, &view, cursor, row.display.clone(), row.complete.then(|| row.end_byte()), tab.column() as f32 * self.char_w - row.indent as f32 * self.char_w, width, batch.as_deref(), window, metrics);
                    }
                } else {
                    self.source.row(g.pane, &view, row.line, tab.column() as f32 * self.char_w, width, batch.as_deref(), window, metrics);
                }
            }
            if let Some(work) = pending { commands.push(Command::PrepareLayout { pane: g.pane, tab: tab.id(), work }); }
            if let Some(max) = self.native_max_scroll(g.pane, g.rows)
                && let Some(viewport) = self.session.native_viewport(g.pane) {
                commands.push(Command::ClampColumn { viewport, max: (max / self.char_w).ceil() as usize });
            }
        }
        commands.extend(self.source.finish());
        for command in commands {
            if self.session.dispatch(command).repaint { window.request_animation_frame(); }
        }
        self.sync_prompts();
    }

    #[allow(clippy::type_complexity)]
    fn projected_row(&self, tab: &Tab, r: &code_core::layout::Row, y: f32, sticky: bool) -> Option<(TextRow, Vec<(f32, std::ops::Range<usize>, u32, u8)>)> {
        let state = self.session.state();
        let view = state.presentation_view(tab.id())?;
        let tab = view.tab();
        let projected = self.source_row(tab, r)?;
        let range = view.snapshot().line_range(r.line)?;
        let palette = state.palette();
        let selection = (!sticky).then(|| tab.selection()).flatten();
        let transient = state.view(tab.id()).is_some_and(|base| base.snapshot().key() != view.snapshot().key());
        let occurrences = state.occurrences().filter(|_| !transient && !sticky && state.active().is_some_and(|a| a.id() == tab.id()));
        let (read, write) = (lenz_content::theme::mix(palette.bg, palette.accent, 0.18), lenz_content::theme::mix(palette.bg, palette.accent, 0.32));
        let mut fg: Vec<_> = projected.hints().into_iter().map(|r| (r, palette.num_fg)).collect();
        let mut decor = Vec::new();
        // Look up only the displayed bytes; a long source line never builds a full span list.
        let mut spans: Vec<(std::ops::Range<usize>, Option<lenz_content::highlight::Capture>, u8)> = Vec::new();
        for (at, c) in projected.text.char_indices() {
            let byte = projected.source_at(at)?;
            let (capture, mods) = view.style_at(r.line, byte.saturating_sub(range.start));
            if let Some((old, old_capture, old_mods)) = spans.last_mut()
                && *old_capture == capture && *old_mods == mods { old.end = at + c.len_utf8(); }
            else { spans.push((at..at + c.len_utf8(), capture, mods)); }
        }
        for (range, capture, mods) in spans {
            let color = capture.map_or(palette.fg, |capture| palette.syntax_color(capture));
            if capture.is_some() { fg.push((range.clone(), color)); }
            if mods != 0 { decor.push((y, range, color, mods)); }
        }
        if !sticky && self.input_state.capture.as_ref().is_some_and(|capture| capture.tab == tab.id())
            && let Some(marked) = self.marked_bytes() {
            let marked = projected.range(marked);
            if !marked.is_empty() { decor.push((y, marked, palette.accent, code_core::decor::MUTABLE)); }
        }
        // Diagnostics of exactly the version shown: errors and warnings in their colour, the rest dimmed.
        if !sticky && !transient && let Some(found) = state.diagnostics(tab.id()) {
            use code_core::semantics::diagnostics::Severity;
            for d in found.items.iter().filter(|d| d.range.start <= range.end && d.range.end >= range.start) {
                // An empty range marks the character it starts at (a short mark at a line's end).
                let shown = if d.range.is_empty() {
                    let Some(c) = projected.caret(d.range.start) else { continue };
                    c..c + projected.text.get(c..).and_then(|t| t.chars().next()).map_or(0, char::len_utf8)
                } else { projected.range(d.range.clone()) };
                if shown.is_empty() && !d.range.is_empty() { continue; }
                let color = match d.severity { Severity::Error => palette.error, Severity::Warning => palette.accent, Severity::Information | Severity::Hint => palette.dim };
                decor.push((y, shown, color, code_core::decor::PROBLEM));
            }
        }
        let selected_range = |sel: Selection| {
            let mut selected = projected.range(sel.range());
            if !sticky && r.last && projected.complete() && view.snapshot().line_ending_selected(r.line, sel.range()) {
                if selected.is_empty() { selected.start = projected.text.len(); }
                selected.end = projected.text.len() + 1;
            }
            selected
        };
        let selected = selection.map(selected_range).filter(|r| !r.is_empty());
        let show_caret = !self.session.state().terminal().focused() && (self.caret_shown || self.session.state().active().is_none_or(|active| active.id() != tab.id()));
        let caret = selection.filter(|_| show_caret).and_then(|sel| projected.caret(sel.head));
        // Every other selection, as the part this row shows (an empty one keeps its caret's place).
        let extra: Vec<_> = if sticky { Vec::new() } else {
            tab.selections().skip(1).filter_map(|sel| {
                let caret = (show_caret && sel.range().is_empty()).then(|| projected.caret(sel.head)).flatten();
                let range = if caret.is_some() { 0..0 } else { selected_range(sel) };
                (caret.is_some() || !range.is_empty()).then_some((range, caret))
            }).collect()
        };
        let mut bg: Vec<_> = occurrences.map_or_else(Vec::new, |o| o.ranges.iter().filter_map(|(r, w)| { let r = projected.range(r.clone()); (!r.is_empty()).then_some((r, if *w { write } else { read })) }).collect());
        if !sticky { bg.extend(tab.find().map_or(&[][..], |f| f.in_line(r.line)).iter().filter_map(|(_, r)| { let r = projected.range(range.start + r.start..range.start + r.end); (!r.is_empty()).then_some((r, palette.found_bg)) })); }
        if !transient && self.session.state().comparison_changed_line(tab.id(), r.line) { bg.insert(0, (0..projected.text.len(), palette.block[2])); }
        let mut row = TextRow { y, number: if r.index == 0 { r.line as u32 + 1 } else { 0 }, text: projected.text.clone(), fg, bg, selected, caret, extra };
        if row.selected.as_ref().is_some_and(|r| r.end > row.text.len()) || row.extra.iter().any(|(r, _)| r.end > row.text.len()) {
            row.text.push(' '); // A selected line ending occupies one display cell without changing source bytes.
        }
        if let Some((fold, true)) = view.fold_at(r.line) && r.last && projected.complete() {
            let at = row.text.len();
            row.text.push_str(PLACEHOLDER);
            row.bg.push((at + 1..at + PLACEHOLDER.len() - 1, lenz_content::theme::mix(palette.bg, palette.num_fg, 0.25)));
            row.fg.push((at..at + PLACEHOLDER.len(), palette.num_fg));
            if fold.closes && let Some(last) = view.snapshot().line_range(fold.last as usize)
                && let Some(text) = view.line_window(fold.last as usize, last.start, 512) { row.text.push_str(text.text.trim()); }
            if selection.is_some_and(|s| s.range().start <= range.end && s.range().end > range.end) { row.selected = Some(row.selected.map_or(at, |r| r.start)..row.text.len()); }
        }
        if r.indent > 0 {
            let prefix = " ".repeat(r.indent);
            let offset = prefix.len();
            row.text.insert_str(0, &prefix);
            for (range, _) in &mut row.fg { range.start += offset; range.end += offset; }
            for (range, _) in &mut row.bg { range.start += offset; range.end += offset; }
            if let Some(range) = &mut row.selected { range.start += offset; range.end += offset; }
            if let Some(caret) = &mut row.caret { *caret += offset; }
            for (range, caret) in &mut row.extra {
                range.start += offset; range.end += offset;
                if let Some(caret) = caret { *caret += offset; }
            }
            for (_, range, _, _) in &mut decor { range.start += offset; range.end += offset; }
        }
        Some((row, decor))
    }

    #[allow(clippy::type_complexity)]
    fn sticky_rows(&self, tab: &Tab, max: usize) -> (Vec<(usize, TextRow)>, Vec<(f32, std::ops::Range<usize>, u32, u8)>) {
        use code_core::layout::Query;
        let state = self.session.state();
        let Some(view) = state.presentation_view(tab.id()) else { return Default::default() };
        let cols = state.wrap_cols(tab);
        let Query::Ready(sticky) = view.layout_sticky(max, cols) else { return Default::default() };
        let mut rows = Vec::new();
        let mut decor = Vec::new();
        for pinned in sticky.iter() {
            if let Query::Ready(Some(r)) = view.layout_row(pinned.line, 0, cols)
                && let Some((row, marks)) = self.projected_row(tab, &r, pinned.row as f32, true) {
                rows.push((pinned.line, row));
                decor.extend(marks);
            }
        }
        (rows, decor)
    }

    /// Current snapshot windows paired with authoritative logical rows.
    #[allow(clippy::type_complexity)]
    fn tab_rows(&self, tab: &Tab, rows: usize) -> (Vec<TextRow>, Vec<(f32, std::ops::Range<usize>, u32, u8)>) {
        if matches!(self.session.state().presentation_view(tab.id()).and_then(|v| v.rendering()), Some(Rendering::Preview(_) | Rendering::Pending)) { return Default::default(); }
        let state = self.session.state();
        let Some(view) = state.presentation_view(tab.id()) else { return Default::default() };
        let mut out = Vec::new();
        let mut decor = Vec::new();
        for (i, r) in view.layout_rows(rows.max(1), state.wrap_cols(tab)).rows.iter().enumerate() {
            if let Some((row, marks)) = self.projected_row(tab, r, i as f32, false) {
                out.push(row);
                decor.extend(marks);
            }
        }
        (out, decor)
    }

    /// The active tab's image as a texture, when it shows one (a raster image, or an SVG not shown as source).
    /// The active tab's image texture, when it shows one (a raster image, or an SVG not shown as source). A new
    /// snapshot's texture is made on a background thread (a copy of its pixels in GPUI's order); until then
    /// nothing is drawn.
    fn shown_image(&mut self, pane: u8, cx: &mut Context<Self>) -> Option<(Arc<gpui::RenderImage>, (u32, u32))> {
        let i = pane as usize;
        let wanted = self.session.state().presentation_shown(pane).and_then(|tab| self.session.state().presentation_view(tab.id())).and_then(|view| wanted_image(&view));
        let Some(pixels) = wanted else {
            self.image[i] = None; // its texture's CPU pixels go too, not only its atlas entry
            return None;
        };
        if !self.image[i].as_ref().is_some_and(|s| s.pixels.upgrade().is_some_and(|p| Arc::ptr_eq(&p, &pixels))) {
            self.image[i] = Some(Shown { pixels: Arc::downgrade(&pixels), texture: None, size: pixels.dimensions() });
            let weak = Arc::downgrade(&pixels);
            cx.spawn(async move |this, cx| {
                let texture = cx.background_executor().spawn(async move { lenz_gpui::image::texture((*pixels).clone()) }).await;
                let _ = this.update(cx, |v, cx| {
                    // Only for the image still shown: a late texture for another one is dropped.
                    if let Some(s) = v.image[i].as_mut().filter(|s| s.pixels.ptr_eq(&weak)) {
                        s.texture = Some(texture);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.image[i].as_ref().and_then(|s| Some((s.texture.clone()?, s.size)))
    }

    /// Why the active raster image cannot be shown, when it cannot.
    pub fn unshowable_image(&self) -> Option<String> {
        self.session.state().active().and_then(|tab| self.session.state().presentation_view(tab.id())).and_then(|view| unshowable_image(&view))
    }

    /// The focused pane's image drawn now: its size in pixels, for tests.
    pub fn image_shown(&self) -> Option<(u32, u32)> {
        self.image[self.session.state().focused_pane() as usize].as_ref().filter(|s| s.texture.is_some() && !self.uploads.is_empty()).map(|s| s.size)
    }

    /// Full diagnostic status, retained for adapter checks and troubleshooting.
    pub fn status(&self) -> String { self.session.status_details() }

    /// Characters that fit across `width` pixels of text.
    fn columns(&self, width: f32) -> usize {
        ((width / self.char_w).floor() as usize).max(1)
    }

    /// The elastic message cell's displayed text (full content is in Status details).
    pub fn status_line(&self) -> String { self.status_segments().1.1 }
    #[cfg(feature = "test-support")]
    pub fn status_position(&self) -> StatusCell { self.status_segments().2 }
    /// For tests: the toolbar's branch selector.
    pub fn toolbar_branch(&self) -> Option<(f32, f32, String)> { self.nav_buttons().into_iter().find(|b| b.2 == CommandId::GitBranches).map(|b| (b.0, b.1, b.4)) }
    pub fn status_sql(&self) -> StatusCell { self.status_segments().3 }
    /// For tests: the change-marker letters painted in the gutters, and the colour each was painted in.
    #[cfg(feature = "test-support")]
    pub fn marker_letters(&self) -> Vec<(char, u32)> { self.painted_markers.clone() }
    /// For tests: the browsed suggestion's documentation lines as last painted beside the completion list.
    #[cfg(feature = "test-support")]
    pub fn completion_documentation_lines(&self) -> Option<Vec<String>> { self.painted_completion_doc.clone() }
    /// For tests: the diagnostic underlines of the active tab's shown rows, as row text and colour, in row order.
    #[cfg(feature = "test-support")]
    pub fn problem_marks(&self) -> Vec<(String, u32)> {
        let state = self.session.state();
        let Some(tab) = state.active() else { return Vec::new() };
        let (rows, decor) = self.tab_rows(tab, self.rows.max(1));
        decor.into_iter().filter(|d| d.3 & code_core::decor::PROBLEM != 0)
            .filter_map(|(y, r, color, _)| Some((rows.iter().find(|row| row.y == y)?.text.get(r)?.to_owned(), color))).collect()
    }
    /// For adapter checks: the visible inline diagnostic labels, logical source lines and colours.
    #[cfg(feature = "test-support")]
    pub fn inline_problem_labels(&self) -> Vec<(usize, String, u32)> {
        self.painted_inline.iter().map(|(_, annotation)| (annotation.line, annotation.text.clone(), annotation.color)).collect()
    }
    /// For tests: the status bar's change-marker baseline cell.
    pub fn status_markers(&self) -> StatusCell { self.status_segments().4 }

    /// The status bar's height: none in a window too short for it and the toolbar.
    fn status_h(&self) -> f32 {
        if self.height >= status_h() + toolbar_h() + line_h() { status_h() } else { 0.0 }
    }

    /// Where the panes, the explorer and the results panel end: the status bar's top.
    fn body_bottom(&self) -> f32 {
        (self.height - self.status_h() - self.activity_height() - self.terminal_height()).max(0.0)
    }

    /// The modal's rows, each wrapped to the menu's width, with the index of the row each line belongs to.
    fn modal_shortcuts(&self) -> Vec<(usize, String)> {
        let ids: Vec<_> = match self.modal.as_ref() {
            Some(Modal::Commands { .. }) => CommandId::ALL.iter().copied().enumerate().collect(),
            Some(Modal::LanguageServer { .. }) => self.server_rows().into_iter().enumerate().filter_map(|(row, r)| r.1.map(|id| (row, id))).collect(),
            Some(m @ (Modal::AppMenu { .. } | Modal::CommandGroup { .. })) => {
                let group = match m { Modal::CommandGroup { group, .. } => Some(*group), _ => None };
                self.session.menu(group).into_iter().enumerate().filter_map(|(row, r)| match r.target { MenuTarget::Run(id) => Some((row, id)), _ => None }).collect()
            }
            _ => Vec::new(),
        };
        ids.into_iter().filter_map(|(row, id)| self.key_hint(id).map(|key| (row, key))).collect()
    }

    pub fn modal_lines(&self) -> Vec<(usize, String, bool)> {
        let cols = self.columns(self.modal_width() - 24.0);
        let shortcuts = self.modal_shortcuts();
        let actions = matches!(self.modal, Some(Modal::Commands { .. } | Modal::AppMenu { .. } | Modal::CommandGroup { .. } | Modal::LanguageServer { .. }));
        self.modal_rows()
            .into_iter()
            .enumerate()
            .flat_map(|(i, (label, why))| {
                let text = match why { Some(why) if !actions => format!("{label} — {why}"), _ => label };
                let cols = shortcuts.iter().find(|(row, key)| *row == i && lenz_content::wrap::width(key) + 3 < cols / 2).map_or(cols, |(_, key)| cols.saturating_sub(lenz_content::wrap::width(key) + 3).max(1));
                text.split('\n').flat_map(|line| wrap(line, cols)).collect::<Vec<_>>().into_iter().map(move |l| (i, l, why.is_some()))
            })
            .collect()
    }

    fn action_menu_help(&self) -> Option<&'static str> {
        let modal = self.modal.as_ref()?;
        if !matches!(modal, Modal::Commands { .. } | Modal::AppMenu { .. } | Modal::CommandGroup { .. } | Modal::LanguageServer { .. }) { return None; }
        self.modal_rows().get(modal.at())?.1
    }

    fn modal_width(&self) -> f32 {
        if matches!(self.modal, Some(Modal::AppMenu { .. })) {
            menu_width(self.width).min(24.0 + 24.0 * self.char_w)
        } else if matches!(self.modal, Some(Modal::Themes { .. } | Modal::StatusDetails { .. } | Modal::LanguageServer { .. })) {
            (self.width - 16.0).clamp(0.0, 600.0)
        } else { menu_width(self.width) }
    }

    /// Clamp the popup viewport, revealing a selection only after keyboard or pointer navigation.
    fn modal_first(&self, at: usize, selected: std::ops::Range<usize>, len: usize, visible: usize) -> usize {
        let identity = self.modal.as_ref().map(|m| (std::mem::discriminant(m), at));
        let previous = self.modal_seen.replace(identity);
        let same_kind = previous.zip(identity).is_some_and(|(old, new)| old.0 == new.0);
        let mut first = if same_kind { self.modal_scroll.get() } else { self.modal_wheel.set(0.0); 0 };
        if previous != identity {
            if selected.start < first { first = selected.start; }
            else if selected.end > first + visible { first = selected.end.saturating_sub(visible); }
        }
        first = first.min(len.saturating_sub(visible.max(1)));
        self.modal_scroll.set(first);
        first
    }

    fn scroll_modal(&self, delta: f32, max: usize) {
        let accumulated = self.modal_wheel.get() + delta;
        let steps = accumulated.trunc() as isize;
        self.modal_wheel.set(accumulated - steps as f32);
        self.modal_scroll.set(self.modal_scroll.get().saturating_add_signed(steps).min(max));
    }

    /// The open modal's box and wrapped lines. Paint and hit testing share the independent viewport.
    fn modal_layout(&self) -> Option<ModalLayout> {
        if self.modal.as_ref().is_some_and(Modal::is_form) { return None; }
        let at = self.modal.as_ref().filter(|m| !matches!(m, Modal::Context { .. } | Modal::Navigation { .. } | Modal::Search { .. }))?.at();
        let mut lines = self.modal_lines();
        let at = at.min(lines.last().map_or(0, |r| r.0));
        let mw = self.modal_width();
        if let Some(Modal::StatusDetails { title, kind, .. }) = &self.modal {
            lines.retain(|(row, ..)| *row != 0);
            let heading = Some((*title).to_string());
            let body_top = line_h() + 28.0;
            let cap = (self.height - 16.0).clamp(0.0, 600.0);
            let (_, button_h) = ModalLayout::button_grid(mw, self.char_w, &self.details_labels(*kind), (cap - body_top - line_h() - 24.0).max(0.0));
            let footer_h = button_h + 24.0;
            let natural = body_top + lines.len().max(1) as f32 * line_h() + footer_h;
            let h = natural.min(cap);
            let visible = (((h - body_top - footer_h).max(0.0) / line_h()).floor() as usize).min(lines.len());
            let first = self.modal_scroll.get().min(lines.len().saturating_sub(visible));
            self.modal_scroll.set(first);
            let bounds = Bounds { origin: gpui::point(px((self.width - mw) / 2.0), px((self.height - h) / 2.0)), size: gpui::size(px(mw), px(h)) };
            return Some(ModalLayout { bounds, lines, first, visible, at, footer: Vec::new(), heading, query: None, body_top, shortcuts: Vec::new() });
        }
        if let Some(Modal::Branches { query, .. }) = &self.modal {
            let heading = self.session.state().branches().map(|b| b.heading().to_string()).unwrap_or_else(|| "Branches".into());
            let hint = self.modal_rows().get(1).map(|r| r.0.clone()).unwrap_or_default();
            let footer = wrap(&hint, self.columns(mw - 24.0));
            lines.retain(|(row, ..)| *row >= FINDER_HEADER_ROWS);
            if lines.is_empty() { lines.push((usize::MAX, "No matching branches".into(), true)); }
            let body_top = 2.0 * line_h() + 40.0;
            let cap = (self.height - 16.0).clamp(0.0, 540.0);
            let visible = (((cap - body_top - (footer.len() as f32 * line_h() + 20.0)).max(0.0) / line_h()).floor() as usize).min(lines.len());
            let start = lines.iter().position(|(i, _, _)| *i == at).unwrap_or(0);
            let end = lines.iter().rposition(|(i, _, _)| *i == at).map_or(0, |i| i + 1);
            let first = self.modal_first(at, start..end, lines.len(), visible);
            let h = (body_top + (visible + footer.len()) as f32 * line_h() + 20.0).min(cap);
            let bounds = Bounds { origin: gpui::point(px((self.width - mw) / 2.0), px((self.height - h) / 2.0)), size: gpui::size(px(mw), px(h)) };
            return Some(ModalLayout { bounds, lines, first, visible, at, footer, heading: Some(heading), query: Some(query.clone()), body_top, shortcuts: Vec::new() });
        }
        if matches!(self.modal, Some(Modal::Themes { .. })) {
            let h = (self.height - 16.0).clamp(0.0, 400.0);
            let visible = (((h - 8.0).max(0.0) / line_h()).floor() as usize).min(lines.len());
            let start = lines.iter().position(|(i, _, _)| *i == at).unwrap_or(0);
            let end = lines.iter().rposition(|(i, _, _)| *i == at).map_or(0, |i| i + 1);
            let first = self.modal_first(at, start..end, lines.len(), visible);
            let bounds = Bounds { origin: gpui::point(px((self.width - mw) / 2.0), px((self.height - h) / 2.0)), size: gpui::size(px(mw), px(h)) };
            return Some(ModalLayout { bounds, lines, first, visible, at, footer: Vec::new(), heading: None, query: None, body_top: 4.0, shortcuts: Vec::new() });
        }
        let heading = if matches!(self.modal, Some(Modal::LanguageServer { .. })) {
            for (row, title) in [(8, "Indexing"), (4, "Setup")] {
                if let Some(at) = lines.iter().position(|r| r.0 == row) { lines.insert(at, (usize::MAX, title.into(), true)); }
            }
            Some("Language Server".to_string())
        } else { None };
        let body_top = if heading.is_some() { line_h() + 28.0 } else { 4.0 };
        let top = if matches!(self.modal, Some(Modal::AppMenu { .. } | Modal::CommandGroup { .. })) { toolbar_h().min((self.height - 16.0).max(0.0)) } else { 8.0 };
        let capacity = ((self.height - top - body_top - 4.0) / line_h()).floor().max(0.0) as usize;
        let footer = self.action_menu_help().map(|help| wrap(help, self.columns(mw - 24.0)).into_iter().take((capacity / 2).min(4)).collect::<Vec<_>>()).unwrap_or_default();
        let visible = capacity.saturating_sub(footer.len()).min(lines.len());
        let start = lines.iter().position(|(i, _, _)| *i == at).unwrap_or(0);
        let end = lines.iter().rposition(|(i, _, _)| *i == at).map_or(0, |i| i + 1);
        let first = self.modal_first(at, start..end, lines.len(), visible);
        let bounds = Bounds { origin: gpui::point(px(if matches!(self.modal, Some(Modal::AppMenu { .. } | Modal::CommandGroup { .. })) { 4.0_f32.min((self.width - mw).max(0.0)) } else { (self.width - mw) / 2.0 }), px(top)), size: gpui::size(px(mw), px(((visible + footer.len()) as f32 * line_h() + body_top + 4.0).min((self.height - top).max(0.0)))) };
        let shortcuts = self.modal_shortcuts();
        Some(ModalLayout { bounds, lines, first, visible, at, footer, heading, query: None, body_top, shortcuts })
    }

    /// The key chooser's rows after its prompt: the bookmarks that have a key (each label starts with it).
    fn keyed_bookmarks(&self) -> Vec<code_core::session::PlaceRow> {
        self.session.state().place_rows(PlaceList::Bookmarks, "").into_iter().filter(|r| !r.label.starts_with('·')).collect()
    }

    fn server_rows(&self) -> Vec<(String, Option<CommandId>)> {
        self.session.language_server_rows()
    }

    /// The modal's rows: label and disabled reason. Search headers precede their result rows.
    pub fn modal_rows(&self) -> Vec<(String, Option<&'static str>)> {
        match &self.modal {
            Some(Modal::StatusDetails { title, text, .. }) => std::iter::once((title.to_string(), None)).chain(text.lines().map(|line| (line.to_owned(), None))).collect(),
            Some(Modal::Navigation { .. }) => self.navigation_rows(),
            Some(Modal::ServerRuntime { path, .. }) => {
                let model = self.session.state().server_runtime();
                let mut rows = vec![(format!("{} › {path}", model.map_or("Server runtime executable / install folder", |m| m.kind.input_label())), None), ("Enter a path or select an installed runtime; Escape closes".to_string(), None)];
                if let Some(model) = model {
                    rows.extend(model.choices(path).into_iter().map(|(label, _)| (label, self.session.disabled(CommandId::ServerRuntime))));
                    rows.extend(wrap(&model.message, self.columns(menu_width(self.width) - 24.0)).into_iter().map(|line| (line, None)));
                }
                rows
            }
            Some(Modal::LanguageServer { .. }) => self.server_rows().into_iter().map(|(text, action)| (text, action.and_then(|id| self.session.disabled(id)))).collect(),
            Some(Modal::AppMenu { .. }) => self.session.menu(None).into_iter().map(|r| (r.label, r.disabled)).collect(),
            Some(Modal::CommandGroup { group, .. }) => self.session.menu(Some(*group)).into_iter().map(|r| (r.label, r.disabled)).collect(),
            Some(Modal::Commands { .. }) => self.session.actions().into_iter().map(|a| (match a.on {
                Some(true) => format!("{} (on)", a.label),
                Some(false) => format!("{} (off)", a.label),
                None => a.label.to_string(),
            }, a.disabled)).collect(),
            Some(Modal::Context { entries, .. }) => entries
                .iter()
                .map(|e| match e {
                    MenuEntry::Command(id) => (id.label().to_string(), self.session.disabled(*id)),
                    MenuEntry::Separator => ("—".to_string(), None),
                })
                .collect(),
            Some(Modal::Themes { ids, .. }) => [
                ("Theme · ↑/↓ choose · Enter select · Esc close".to_string(), None),
                (format!("Current: {}", self.session.theme_source()), None),
            ].into_iter().chain(ids.iter().map(|(_, label)| (label.clone(), None))).collect(),
            Some(Modal::ScratchFiles { files, .. }) => {
                let mut rows = vec![("Personal scratch files — ↑/↓ choose · Enter open · Escape close".into(), None)];
                if files.is_empty() { rows.push(("No scratch files yet — New Scratch File… creates one".into(), Some("No files"))); }
                else { rows.extend(files.iter().map(|p| (p.file_name().unwrap_or_default().to_string_lossy().into_owned(), None))); }
                rows
            },
            Some(Modal::Recent { .. }) => self.session.state().recent().iter().map(|r| (r.display().to_string(), None)).collect(),
            Some(Modal::Keymap { capture, .. }) => {
                let hint = if *capture {
                    "Press the new shortcut (Esc cancels); its command keeps its menu entry either way".to_string()
                } else {
                    format!("Keymap: {} · Enter rebinds the selected command · Delete restores its preset key · Esc closes", self.session.keymap().preset().label())
                };
                std::iter::once((hint, None))
                    .chain(self.session.keymap().rows().into_iter().map(|r| {
                        let note = r.note.map_or_else(String::new, |note| format!(" ({note})"));
                        (format!("{} — {}{note}", r.label, r.chords), None)
                    }))
                    .collect()
            }
            Some(Modal::Declarations { query, semantic: false }) => {
                let tab = if self.session.disabled(CommandId::Symbols).is_none() { "; Tab: language server symbols" } else { "" };
                vec![(format!("› {query}"), None), (format!("Find declarations by name (syntactic candidates) — results below; Enter to browse them{tab}"), None)]
            }
            Some(Modal::Declarations { query, semantic: true }) => {
                vec![(format!("› {query}"), None), ("Find symbols (language server) — results below; Enter to browse them; Tab: syntactic declarations".to_string(), None)]
            }
            Some(Modal::SqlMetadata { path }) => vec![(format!("› {path}"), None), (self.session.sql_metadata_prompt().into(), None)],
            Some(Modal::Root { path }) => {
                vec![(format!("› {path}"), None), ("Workspace root for this file's language server — Enter; empty uses its own discovery".to_string(), None)]
            }
            Some(Modal::GoTo { input, error }) => {
                let hint = error.clone().unwrap_or_else(|| "Go to line or line:column — Enter".to_string());
                vec![(format!("› {input}"), None), (hint, None)]
            }
            Some(Modal::Search { query, project: false, .. }) => {
                vec![(format!("› {query}"), None), ("Search this file for this text (literal, any case) — Enter".to_string(), None)]
            }
            Some(Modal::Search { query, files, .. }) => {
                let files = if files.is_empty() { "all files".to_string() } else { files.clone() };
                let o = self.session.find_options();
                let marks: Vec<String> = [(o.case, "Aa"), (o.word, "W"), (o.regex, ".*")].into_iter().map(|(on, m)| if on { format!("[{m}]") } else { m.to_string() }).collect();
                let hint = match self.session.search_refusal(query) {
                    Some(e) => format!("{e} — fix the expression to search"),
                    None => "Search the project for this text — Enter; Tab: which files, e.g. *.rs, src/, !*.test.ts".to_string(),
                };
                vec![
                    (format!("Text  › {query}"), None),
                    (format!("Files › {files}"), None),
                    (format!("Options: {} — Alt+C match case, Alt+W whole word, Alt+R regular expression", marks.join(" ")), None),
                    (hint, None),
                ]
            }
            Some(Modal::Siblings { pane, crumb, query, .. }) => {
                let mut rows = vec![(format!("› {query}"), None), ("Type to filter — Enter goes there, Escape back".to_string(), None)];
                rows.extend(self.sibling_items(*pane, *crumb, query).into_iter().map(|s| (if s.dir { format!("{}/", s.label) } else { s.label }, None)));
                rows
            }
            Some(Modal::Branches { query, .. }) => {
                let hint = match self.session.state().branches() {
                    None => String::new(),
                    Some(b) => match b.state_text() {
                        Some(state) => format!("{} — {state}", b.heading()),
                        None if b.worktrees_only => format!("{} — type to filter; Enter compares with the working tree, Right more, Escape closes", b.heading()),
                        None if b.revisions_only => format!("{} — type to filter or a revision; Enter compares with the working tree, Escape closes", b.heading()),
                        None => format!("{} — type to filter; Enter checks out (or opens a worktree), Right more, Escape closes", b.heading()),
                    },
                };
                let mut rows = vec![(format!("› {query}"), None), (hint, None)];
                rows.extend(self.session.state().branch_rows(query).into_iter().map(|r| (code_core::branches::row_text(&r), None)));
                rows
            }
            Some(Modal::BranchMenu { target, label, .. }) => {
                let mut rows = vec![(format!("‹ {label} — Escape back"), None)];
                rows.extend(self.session.branch_actions(target).into_iter().map(|(l, _)| (l.to_string(), None)));
                rows
            }
            Some(Modal::NewBranch { from, name, checkout, error }) => {
                let start = from.as_deref().map_or("HEAD", code_core::branches::short_ref);
                let hint = error.clone().unwrap_or_else(|| format!("New branch from {start} — Enter; Tab: check it out or not"));
                vec![(format!("› {name}"), None), (format!("[{}] Check out the new branch", if *checkout { "x" } else { " " }), None), (hint, None)]
            }
            Some(Modal::SaveAs { input, selected, error, .. }) => {
                vec![(format!("› {input}{}", if *selected { "  (selected)" } else { "" }), None),
                    (error.clone().unwrap_or_else(|| "Save As — type a path; Enter saves, Escape cancels; Ctrl+A selects all".into()), None)]
            }
            Some(Modal::NewScratch { input, selected, error } | Modal::RenameTerminal { input, selected, error, .. }) => {
                vec![(format!("› {input}{}", if *selected { "  (selected)" } else { "" }), None),
                    (error.clone().unwrap_or_else(|| "New scratch file — name with extension chooses language; Enter creates, Escape cancels; Ctrl+A selects all".into()), None)]
            }
            Some(Modal::Create { kind, dir, name, error }) => {
                let where_ = if dir.is_empty() { "the project root" } else { dir.as_str() };
                let hint = error.clone().unwrap_or_else(|| format!("{} in {where_} — Enter creates, Escape cancels", kind.label()));
                vec![(format!("› {name}"), None), (hint, None)]
            }
            Some(Modal::PasteEntry { request, name, selected, error }) => {
                let folder = if request.folder().is_empty() { "the project root" } else { request.folder() };
                vec![(format!("› {name}{}", if *selected { "  (selected)" } else { "" }), None),
                    (error.clone().unwrap_or_else(|| format!("Paste {} in {folder} — saved contents; Enter copies, Escape cancels; Ctrl+A selects name", request.source().display())), None)]
            }
            Some(Modal::ConfirmDelete { removal, from, error }) => {
                let hint = match error {
                    Some(why) => format!("{why} — Escape closes"),
                    None => format!("Enter: {} — Escape cancels", removal.label()),
                };
                vec![(self.session.delete_prompt(*removal, from), None), (hint, None)]
            }
            Some(Modal::Relocate { kind, from, input, error }) => {
                let hint = error.clone().unwrap_or_else(|| format!("{} {from}: type {} — Enter, Escape cancels. References in other files are not changed.", kind.label(), kind.asks()));
                vec![(format!("› {input}"), None), (hint, None)]
            }
            Some(Modal::Revision { input, error }) => {
                let hint = error.clone().unwrap_or_else(|| "Checkout tag or revision (a tag, branch, commit id or HEAD~2) — Enter".to_string());
                vec![(format!("› {input}"), None), (hint, None)]
            }
            Some(Modal::Places { list, query, .. }) => {
                let hint = match list {
                    PlaceList::Bookmarks => "Bookmarks — type to filter; Enter goes there, Delete removes, Escape back",
                    PlaceList::RecentFiles => "Recent files — type to filter; Enter opens, Delete forgets, Escape back",
                    PlaceList::RecentLocations => "Recent locations — type to filter; Enter goes there, Delete forgets, Escape back",
                };
                let mut rows = vec![(format!("› {query}"), None), (hint.to_string(), None)];
                rows.extend(self.session.state().place_rows(*list, query).into_iter().map(|r| {
                    let missing = if r.missing { "  (missing)" } else { "" };
                    let text = if r.detail.is_empty() { format!("{}{missing}", r.label) } else { format!("{} — {}{missing}", r.label, r.detail) };
                    (text, r.missing.then_some("missing"))
                }));
                rows
            }
            Some(Modal::Mnemonic) => {
                let mut rows = vec![("Bookmark key: press 0–9 or A–Z (its own key removes it) — Escape cancels".to_string(), None)];
                rows.extend(self.keyed_bookmarks().into_iter().map(|r| (r.label, None)));
                rows
            }
            None => Vec::new(),
        }
    }
}

impl Render for ReaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.synchronize_input(window, cx);
        #[cfg(feature = "perf-probe")]
        let render_started = std::time::Instant::now();
        #[cfg(test)]
        self.painted_annotations.clear();
        // A menu or dialog takes the pointer: documentation shown, asked for or waiting on a rest closes before it
        // is painted over them (an answer on its way is dropped with its request).
        if self.modal.is_some() && (self.doc_rest.0.is_some() || self.session.state().documentation().is_some()) {
            self.doc_rest = (None, None);
            self.doc_grace = None;
            self.dispatch(Command::CloseDocumentation, cx);
        }
        if !std::mem::replace(&mut self.themes_started, true) {
            cx.observe_window_activation(window, |v, window, cx| {
                if !window.is_window_active() { v.double_shift.reset(); }
                v.reset_caret_blink(); cx.notify();
            }).detach();
            cx.on_focus(&self.focus, window, |v, _, cx| { v.reset_caret_blink(); cx.notify(); }).detach();
            cx.on_blur(&self.focus, window, |v, _, cx| { v.reset_caret_blink(); cx.notify(); }).detach();
            // Theme I/O starts once the first frame is on its way.
            self.session.start_themes();
            let this = cx.weak_entity();
            let id = window.window_handle().window_id();
            cx.on_window_closed(move |cx, closed| {
                if closed == id { let _ = this.update(cx, |view, _| { view.remember_explorer(); view.session.dispatch(Command::SaveWorkspace); }); }
            }).detach();
        }
        let palette = self.session.state().palette().clone();
        let viewport = window.viewport_size();
        let run = gpui::TextRun { len: 10, font: self.font.clone(), color: gpui::Hsla::default(), background_color: None, underline: None, strikethrough: None };
        let shaped = window.text_system().shape_line("0123456789".into(), px(font_size()), &[run], None);
        self.char_w = (f32::from(shaped.width) / 10.0).max(1.0);
        #[cfg(feature = "perf-probe")]
        probe::native_widths(window, self.metrics());
        self.width = f32::from(viewport.width);
        self.height = f32::from(viewport.height);
        // The explorer takes a share of the width (or what its divider was dragged to); below 320 px it shows only
        // while it has focus. The files panel, its toolbar and tabs, takes the rest.
        let narrow = self.width < 320.0;
        let explorer_w = self.layout.explorer_width.map_or((self.width * 0.35).clamp(120.0, 280.0), |w| (w as f32).clamp(120.0, (self.width - 160.0).max(120.0)));
        self.sidebar_w = if self.session.state().has_tree() && self.tree_shown && (!narrow || self.tree_focus) { if narrow { self.width } else { explorer_w } } else { 0.0 };
        let (server_cell, message_cell, position_cell, sql_cell, markers_cell) = self.status_segments();
        let attention = self.session.state().provider().is_some_and(|v| matches!(v.status, code_core::semantics::Status::Missing(_) | code_core::semantics::Status::Failed(_) | code_core::semantics::Status::Disabled));
        let server_color = if attention { palette.error } else { palette.accent };
        // The toolbar row on top: the explorer's title over the tree; over the text the buttons, then the first
        // pane's tabs.
        let tabs_h = line_h();
        let nav = self.nav_buttons();
        let explorer_buttons = self.explorer_buttons();
        let explorer_title = self.session.state().explorer_search().label().unwrap_or_else(|| self.session.state().project().map(|p| p.root().file_name().map_or_else(|| p.root().display().to_string(), |n| n.to_string_lossy().into_owned())).unwrap_or_else(|| "Files".into()));
        let nav_end = self.sidebar_w;
        let body_top = toolbar_h() + tabs_h;
        // The results panel: a status line, the list and a preview, below the panes and above the status bar: 45% of
        // the height (at most 15 rows) unless its divider was dragged.
        let status_h = self.status_h();
        let body_rows = ((self.body_bottom() - body_top) / line_h()).floor().max(0.0) as usize;
        let least = 3.min(body_rows);
        let panel_rows = match (self.session.state().results(), self.layout.results_permille) {
            (None, _) => 0,
            (Some(_), None) => ((body_rows as f32 * 0.45) as usize).clamp(least, 15),
            (Some(_), Some(k)) => ((body_rows as f32 * k as f32 / 1000.0).round() as usize).clamp(least, body_rows.saturating_sub(2).max(least)),
        };
        // A wide window shows the preview beside the list, each with the panel's full height; else below it.
        let side = self.width >= SIDE_PREVIEW_MIN_W;
        self.panel_list_w = if side { (self.width * 0.45).round() } else { f32::MAX };
        let list_rows = if side { panel_rows.saturating_sub(1) } else { panel_rows.saturating_sub(1).div_ceil(2) };
        let preview_rows = if side { list_rows } else { panel_rows.saturating_sub(1 + list_rows) };
        self.panel_list_rows = list_rows;
        let text_rows = body_rows - panel_rows;
        self.tree_rows = text_rows;
        let panel_top = body_top + text_rows as f32 * line_h();
        self.panel_top = if panel_rows > 0 { panel_top } else { f32::MAX };
        self.panel_list_y = if panel_rows > 0 { panel_top + line_h() } else { f32::MAX };
        let status = self.session.results_header();
        let results_buttons = self.results_buttons();
        let list_w = self.panel_list_w;
        let (list, preview) = self.panel_contents(list_rows, preview_rows.saturating_sub(1));
        let preview_title = self.session.state().results().and_then(|r| r.preview().and_then(|p| r.location(p.index)));
        // The panes: one, or two side by side or one above the other, split where the divider was dragged (half by
        // default). Rows split on whole lines; the lower pane has a tab bar of its own.
        let split = self.session.state().split();
        let (fx, fw) = (self.sidebar_w, (self.width - self.sidebar_w).max(0.0));
        let share = self.layout.split_permille.map_or(0.5, |k| (k as f32 / 1000.0).clamp(0.1, 0.9));
        let area = |x: f32, y: f32, w: f32, rows: usize| Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(w.max(0.0)), px(rows as f32 * line_h())) };
        let mut panes = Vec::new();
        match split {
            None => panes.push(PaneGeom { pane: 0, bar: (nav_end, self.width, toolbar_h()), text: area(fx, body_top, fw, text_rows), rows: text_rows, ann: 0.0, bar_strip: Bounds::default() }),
            Some(Split::Columns) => {
                let x = if fw >= 120.0 { (fx + (fw * share).round()).clamp(fx + 60.0, self.width - 60.0) } else { fx + (fw / 2.0).floor() };
                self.split_at = x;
                panes.push(PaneGeom { pane: 0, bar: (nav_end, x, toolbar_h()), text: area(fx, body_top, x - fx, text_rows), rows: text_rows, ann: 0.0, bar_strip: Bounds::default() });
                panes.push(PaneGeom { pane: 1, bar: (x + 1.0, self.width, toolbar_h()), text: area(x + 1.0, body_top, self.width - x - 1.0, text_rows), rows: text_rows, ann: 0.0, bar_strip: Bounds::default() });
            }
            Some(Split::Rows) => {
                // Too short for two panes and a bar: the first pane alone, until there is room.
                let top = if text_rows >= 3 { ((((text_rows - 1) as f32) * share).round() as usize).clamp(1, text_rows - 2) } else { text_rows };
                let y = body_top + top as f32 * line_h();
                self.split_at = y;
                panes.push(PaneGeom { pane: 0, bar: (nav_end, self.width, toolbar_h()), text: area(fx, body_top, fw, top), rows: top, ann: 0.0, bar_strip: Bounds::default() });
                if text_rows >= 3 {
                    panes.push(PaneGeom { pane: 1, bar: (fx, self.width, y), text: area(fx, y + line_h(), fw, text_rows - 1 - top), rows: text_rows - 1 - top, ann: 0.0, bar_strip: Bounds::default() });
                }
            }
        }
        // The breadcrumb bar: one row at the top of each pane showing a tab, taken from its text.
        let mut crumb_rows: Vec<(u8, f32, f32, f32)> = Vec::new();
        if self.session.state().breadcrumbs() {
            for g in &mut panes {
                if g.rows >= 2 && self.session.state().presentation_shown(g.pane).is_some() {
                    crumb_rows.push((g.pane, f32::from(g.text.origin.x), f32::from(g.text.origin.y), f32::from(g.text.size.width)));
                    g.text.origin.y += px(line_h());
                    g.text.size.height -= px(line_h());
                    g.rows -= 1;
                }
            }
        }
        // The blame column, left of the line numbers, while the pane's tab is annotated and shows its text; none when
        // it would leave the text less than 40 px.
        for g in &mut panes {
            let state = self.session.state();
            let Some(t) = state.presentation_shown(g.pane) else { continue };
            let text = state.csv(t.id()).is_none() && state.presentation_view(t.id()).is_some_and(|view| view.snapshot().image().is_none() || t.image_source()) && !matches!(state.presentation_view(t.id()).and_then(|v| v.rendering()), Some(Rendering::Preview(_) | Rendering::Pending));
            let w = state.annotation_cols(t) as f32 * self.char_w + 12.0;
            if text && w > 12.0 && f32::from(g.text.size.width) - w - num_w() - 16.0 >= 40.0 {
                g.ann = w;
                g.text.origin.x += px(w);
                g.text.size.width -= px(w);
            }
        }
        // The scrollbar: a strip at the right edge of a pane showing text, a preview or a table, taken from its text
        // (so lines wrap before it); none once the text right of the gutter (a preview or a table has none) would be
        // under two strips wide (the strip goes before the text does).
        let strip_w = 12.0 * font_size() / 13.0;
        for g in &mut panes {
            let state = self.session.state();
            let Some(t) = state.presentation_shown(g.pane) else { continue };
            let Some(view) = state.presentation_view(t.id()) else { continue };
            if view.snapshot().image().is_some() && !t.image_source() { continue }
            let gutter = if state.csv(t.id()).is_some() || matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { 0.0 } else { num_w() };
            if f32::from(g.text.size.width) - gutter - 16.0 >= 2.0 * strip_w {
                g.text.size.width -= px(strip_w);
                g.bar_strip = Bounds { origin: gpui::point(g.text.right(), g.text.origin.y), size: gpui::size(px(strip_w), g.text.size.height) };
            }
        }
        let (crumb_hits, crumb_cells) = self.layout_crumbs(&crumb_rows);
        self.crumb_hits = crumb_hits;
        if self.crumb_focus.is_some_and(|(pane, i)| !self.crumb_hits.iter().any(|h| h.pane == pane && h.crumb == Some(i))) {
            self.crumb_focus = None;
        }
        // A pane not shown (unsplit, or too short) keeps no preview or image.
        for i in 0..2 {
            if !panes.iter().any(|g| g.pane as usize == i) {
                self.markdown[i] = None;
                self.image[i] = None;
            }
        }
        let focused = self.session.state().focused_pane();
        let main = panes.iter().find(|g| g.pane == focused).or(panes.first()).copied();
        self.rows = main.map_or(0, |g| g.rows);
        self.text_bounds = main.map(|g| g.text);
        // Manual strip scrolling stands until a different tab is activated; then reveal that tab.
        let mut hits = Vec::new();
        let mut bars: Vec<Vec<TabCell>> = Vec::new();
        for g in &panes {
            let state = self.session.state();
            let shown = state.presentation_shown(g.pane).map(|t| t.id());
            let mut x = g.bar.0;
            let mut row: Vec<(TabCell, TabId)> = state.pane_tabs(g.pane).map(|t| {
                let title = format!("{}{}", state.problem_badge(t.id()), t.title());
                let (dirty, status) = (state.tab_dirty(t.id()), state.tab_status(t.id()));
                let recovered = t.recovered();
                let w = (title.chars().count() as f32 + 5.0 + if dirty { 2.0 } else { 0.0 } + if recovered { 2.0 } else { 0.0 } + if status.is_some() { 2.0 } else { 0.0 }) * self.char_w + icon_w(state.file_icons());
                x += w;
                ((title, t.retained(), Some(t.id()) == shown, x - w, x, dirty, status, recovered), t.id())
            }).collect();
            let right = self.preview_button(*g).map_or(g.bar.1, |(b, _)| f32::from(b.left()) - 4.0);
            let pane = usize::from(g.pane);
            let maximum = (x - right).max(0.0);
            self.tab_scroll_max[pane] = maximum;
            let mut shift = self.tab_scroll[pane].clamp(0.0, maximum);
            if self.tab_scroll_seen[pane] != shown { self.tab_scroll_manual[pane] = false; }
            if !self.tab_scroll_manual[pane] {
                if let Some((tab, _)) = row.iter().find(|r| r.0.2) {
                    if tab.3 - shift < g.bar.0 { shift = tab.3 - g.bar.0; }
                    else if tab.4 - shift > right { shift = (tab.4 - right).min(tab.3 - g.bar.0).max(0.0); }
                }
                self.tab_scroll_seen[pane] = shown;
            }
            self.tab_scroll[pane] = shift.clamp(0.0, maximum);
            for (r, _) in &mut row {
                r.3 -= shift;
                r.4 -= shift;
            }
            hits.extend(row.iter().map(|(r, id)| (r.3, r.4, *id)));
            bars.push(row.into_iter().map(|(r, _)| r).collect());
        }
        self.tab_hits = hits;
        self.panes = panes.clone();
        // The core places matches (and wraps lines) by each pane's text size.
        for g in &panes {
            if self.session.state().shown(g.pane).is_some_and(|t| self.session.state().csv(t.id()).is_some()) { continue; }
            let cols = ((f32::from(g.text.size.width) - num_w() - 16.0) / self.char_w).floor().max(1.0) as usize;
            if let Some(sent) = self.sent_size.get_mut(g.pane as usize).filter(|s| **s != (cols, g.rows)) {
                *sent = (cols, g.rows);
                let _ = self.session.dispatch(Command::SetTextSize { pane: g.pane, cols, rows: g.rows });
            }
        }
        // Where a dragged tab would land, drawn as a mark in that pane's bar.
        let drop_x = self.tab_drop.filter(|_| matches!(self.drag, Some(Drag::Tab { moved: true, .. }))).map(|(pane, _, x)| (pane, x));
        let cursor = self.pointer();
        // Bring a newly highlighted tree row into view once; the wheel's scroll otherwise stands, within the rows.
        let (tree_len, tree_at) = (self.tree_len(), self.tree_at());
        if std::mem::take(&mut self.tree_reveal) {
            if tree_at < self.tree_scroll {
                self.tree_scroll = tree_at;
            } else if self.tree_rows > 0 && tree_at >= self.tree_scroll + self.tree_rows {
                self.tree_scroll = tree_at + 1 - self.tree_rows;
            }
        }
        self.tree_scroll = self.tree_scroll.min(tree_len.saturating_sub(self.tree_rows.max(1)));
        let tree = self.tree_lines(tree_at);
        #[cfg(test)]
        {
            self.painted_badges.clear();
            let tabs = bars.iter().flatten().map(|t| (t.5, t.6));
            let rows = tree.iter().map(|l| (l.dirty, l.status));
            for (dirty, status) in tabs.chain(rows) {
                self.painted_badges.extend(dirty.then_some("●").into_iter().chain(status.map(|s| s.letter())));
            }
        }
        if self.icon_font.is_none() {
            self.icon_font = Some(icons::nerd_font(&window.text_system().all_font_names()));
        }
        let icon_font = self.icon_font.clone().flatten();
        // Logos shown in the tree and the tabs, drawn once per display scale; GPUI's atlas keeps every one drawn so far
        // (a few dozen at most), released with the window or when the scale changes.
        let logo_px = (icons::SIZE * window.scale_factor()).round() as u32;
        if logo_px != self.logo_px {
            self.logos.clear();
            self.logo_px = logo_px;
        }
        let shown = tree.iter().map(|l| l.icon).chain(bars.iter().flatten().map(|t| icons::file(&t.0))).chain(list.iter().filter_map(|r| r.node.map(|n| n.0)));
        for logo in shown.filter_map(|i| i.logo) {
            if !self.logos.contains_key(logo.name)
                && let Some(pixels) = logo.rasterize(logo_px)
            {
                self.logos.insert(logo.name, lenz_gpui::image::texture(pixels));
            }
        }
        self.logo_uploads.keep(self.logos.values().cloned().collect(), window);
        // About's logo stays in the atlas only while About is open.
        let about = match &self.modal {
            Some(Modal::StatusDetails { logo: Some(logo), .. }) => vec![logo.0.clone()],
            _ => Vec::new(),
        };
        self.about_upload.keep(about, window);
        let logos = self.logos.clone();
        let file_icons = self.session.state().file_icons();
        let icon_w = icon_w(file_icons);
        let (sidebar_w, tree_focus) = (self.sidebar_w, self.tree_focus);
        // Clamp owned scroll to the current layout; a tab still loading keeps its position.
        self.prepare_source(&panes, window);
        self.sync_caret_blink(window, cx);
        // Each pane's content: an image, a Markdown preview laid out for its width, or its text rows.
        let mut frames = Vec::new();
        self.csv_ui.hits_clear();
        let mut sticky_hits = Vec::new();
        let mut start_hits = Vec::new();
        let mut marks_drawn = Vec::new();
        #[cfg(feature = "test-support")]
        let mut painted_markers = Vec::new();
        #[cfg(feature = "test-support")]
        let mut painted_scrollbars = Vec::new();
        // Reused each frame (the test accessor): no allocation once warm.
        let mut folds_drawn = std::mem::take(&mut self.fold_marks_drawn);
        folds_drawn.clear();
        for (g, tabs) in panes.iter().zip(bars) {
            let csv = self.csv_frame(*g);
            let picture = self.shown_image(g.pane, cx);
            let markdown = self.preview_frame(g.pane, window, f32::from(g.text.size.width), g.rows, cx);
            // Its scrollbar: bounded by the marks the core holds and the track's cells.
            let scrollbar = self.scrollbar(g).map_or_else(Default::default, |(bar, marks)| {
                let len = f32::from(g.bar_strip.size.height);
                let placed = scrollbar::place(&marks, bar.total, track_cells(len));
                (bar.thumb(len, THUMB_MIN), placed.iter().enumerate().filter_map(|(cell, i)| Some((cell, marks[(*i)?].kind))).collect::<Vec<_>>())
            });
            #[cfg(feature = "test-support")]
            if g.bar_strip.size.width > px(0.0) { painted_scrollbars.push((g.pane, scrollbar.0, scrollbar.1.iter().map(|m| m.1).collect())); }
            let state = self.session.state();
            let tab = state.presentation_shown(g.pane);
            let pending = matches!(tab.and_then(|tab| state.presentation_view(tab.id())).and_then(|v| v.rendering()), Some(Rendering::Pending)) || (self.markdown[g.pane as usize].is_some() && markdown.is_none());
            let unshowable = tab.and_then(|tab| state.presentation_view(tab.id())).and_then(|view| unshowable_image(&view));
            let (mut rows, decor) = match tab {
                Some(t) if csv.is_none() && picture.is_none() && unshowable.is_none() => self.tab_rows(t, g.rows),
                _ => Default::default(),
            };
            // The Ctrl-hovered identifier, in the link colour and underlined.
            let link = self.link.clone().filter(|l| l.0 == g.pane).and_then(|(_, range)| {
                let tab = tab?;
                let view = state.presentation_view(tab.id())?;
                let logical = view.layout_rows(g.rows, state.wrap_cols(tab)).rows;
                let (at, r) = logical.iter().enumerate().find(|(_, r)| r.source.start <= range.start && range.start < r.source.end)?;
                let native = self.source_row(tab, r)?;
                let shown = native.range(range);
                if shown.is_empty() { return None; }
                let row = rows.iter_mut().find(|row| row.y == at as f32)?;
                let shown = shown.start + r.indent..shown.end + r.indent;
                row.fg.insert(0, (shown.clone(), state.palette().syntax.link));
                Some((row.y, shown))
            });
            // Nothing open: the start screen's commands and recent folders, each a click target.
            let items = if tab.is_none() && g.pane == 0 { state.start_items() } else { Vec::new() };
            let (mut start, mut row, mut heading) = (Vec::new(), 0.0, None);
            for (i, item) in items.iter().enumerate() {
                let label = match item {
                    StartItem::Command(id) => self.key_hint(*id).map_or_else(|| id.label().to_string(), |key| format!("{}  {key}", id.label())),
                    StartItem::RecentFolder(path) | StartItem::RecentFile(path) => path.display().to_string(),
                };
                // Each group of recents under a blank row and its heading.
                if item.heading() != heading {
                    heading = item.heading();
                    if let Some(h) = heading {
                        start.push((row + 1.0, h.to_string(), false));
                        row += 2.0;
                    }
                }
                // Only what the pane shows takes clicks: a row past its edge would cover the panel below.
                let (top, width) = (24.0 + row * line_h(), (lenz_content::wrap::width(&label) as f32 * self.char_w).min(f32::from(g.text.size.width) - 24.0));
                if top + line_h() <= f32::from(g.text.size.height) && width > 0.0 {
                    let at = gpui::point(g.text.origin.x + px(24.0), g.text.origin.y + px(top));
                    start_hits.push((Bounds { origin: at, size: gpui::size(px(width), px(line_h())) }, i));
                }
                start.push((row, label, true));
                row += 1.0;
            }
            let hint = match tab {
                Some(t) if state.presentation_view(t.id()).is_some() || state.loading().is_some() => None,
                None if !start.is_empty() => None,
                None if split.is_some() => Some("Open a file here, or drag a tab to this pane"),
                _ if state.project().is_some() => Some("Pick a file in the tree, or find one with Ctrl+P"),
                _ => Some("Open a file to read it (Ctrl+O)"),
            };
            let hscroll = tab.map_or(0.0, |t| t.column() as f32 * self.char_w);
            // Fold marks, bright on the caret's row or while the pointer is over this gutter.
            let caret_line = tab.and_then(|t| Some(state.presentation_view(t.id())?.snapshot().line_of(t.selection()?.head)));
            let mut marks = match tab {
                Some(t) if !rows.is_empty() => Self::marks(state, t, g.rows).into_iter().map(|(i, c)| (i, c, self.gutter_hover == Some(g.pane) || rows.get(i).is_some_and(|r| r.number > 0 && Some(r.number as usize - 1) == caret_line))).collect(),
                _ => Vec::new(),
            };
            // Sticky lines: the enclosing declarations' first lines over the top rows (at most a third, none under 6).
            let (sticky, sticky_decor) = match tab {
                Some(t) if !rows.is_empty() && state.sticky_scroll() => self.sticky_rows(t, code_core::tabs::sticky_cap(g.rows)),
                _ => Default::default(),
            };
            // Bookmarks: a mark at the gutter's left edge, on text rows and sticky lines (row in lines, key).
            let mut bookmarks: Vec<(f32, Option<char>)> = match tab {
                Some(t) if !rows.is_empty() => Self::bookmark_rows(state, t, g.rows).into_iter().map(|(i, k)| (i as f32, k)).collect(),
                _ => Vec::new(),
            };
            let comparison_marks = tab.map_or_else(Vec::new, |t| rows.iter().filter(|row| row.number > 0 && state.comparison_changed_line(t.id(), row.number as usize - 1)).map(|row| (row.y, Some('▌'))).collect());
            // The draft's change against its baseline: the change's letter, then a deletion above the line.
            let change_marks = tab.map_or_else(Vec::new, |t| state.markers(t.id()).map_or_else(Vec::new, |markers| {
                rows.iter().filter(|row| row.number > 0).flat_map(|row| {
                    let line = row.number as usize - 1;
                    let change = markers.change(line).map(|change| (row.y, change.letter().chars().next().unwrap_or(' '), change.color(&palette)));
                    let deleted = (markers.deleted_above(line) > 0).then(|| (row.y, 'D', code_core::markers::deleted_color(&palette)));
                    change.into_iter().chain(deleted)
                }).collect()
            }));
            let mut sticky_bookmarks = Vec::new();
            let mut sticky_change = Vec::new();
            for (line, row) in &sticky {
                let at = gpui::point(g.text.origin.x, g.text.origin.y + px(row.y * line_h()));
                sticky_hits.push((Bounds { origin: at, size: gpui::size(g.text.size.width, px(line_h())) }, g.pane, *line));
                if let Some(m) = tab.and_then(|t| state.places().bookmark(t.source(), *line)) {
                    sticky_bookmarks.push((row.y, m.mnemonic));
                }
                if let Some(markers) = tab.and_then(|t| state.markers(t.id())) {
                    if let Some(change) = markers.change(*line) {
                        sticky_change.push((row.y, change.letter().chars().next().unwrap_or(' '), change.color(&palette)));
                    }
                    if markers.deleted_above(*line) > 0 {
                        sticky_change.push((row.y, 'D', code_core::markers::deleted_color(&palette)));
                    }
                }
            }
            // Rows under the sticky band are hidden by it; its own marks are painted over it.
            let band = sticky.iter().map(|(_, r)| r.y + 1.0).reduce(f32::max).unwrap_or(0.0);
            // Annotations of the rows below the band (a line's first row): row, label, age shade, of the hovered commit.
            let transient = tab.and_then(|tab| state.view(tab.id()).zip(state.presentation_view(tab.id())))
                .is_some_and(|(base, shown)| base.snapshot().key() != shown.snapshot().key());
            let hovered = self.ann_hovered().and_then(|(_, t, line)| Some(self.session.annotation(t, line)?.commit.sha.clone()));
            let ann: Vec<(f32, String, Option<f32>, bool)> = match tab {
                Some(t) if g.ann > 0.0 && !transient => rows.iter().filter(|r| r.number > 0 && r.y >= band).filter_map(|r| {
                    if let Some(label) = t.blame_status().and_then(code_core::blame::Status::label) {
                        return Some((r.y, label.into(), None, false));
                    }
                    let a = self.session.annotation(t.id(), r.number as usize - 1)?;
                    let hot = a.shade.is_some() && hovered.as_deref() == Some(a.commit.sha.as_str());
                    Some((r.y, a.label, a.shade, hot))
                }).collect(),
                _ => Vec::new(),
            };
            #[cfg(test)]
            self.painted_annotations.extend(ann.iter().map(|(_, label, _, _)| label.clone()));
            bookmarks.retain(|(y, _)| *y >= band);
            marks.retain(|&(i, ..)| i as f32 >= band);
            let sticky_parts = tab.map_or_else(Vec::new, |tab| sticky.iter().map(|(line, row)| {
                state.presentation_view(tab.id()).and_then(|view| match view.layout_row(*line, 0, state.wrap_cols(tab)) {
                    code_core::layout::Query::Ready(Some(logical)) => Some(self.source_parts(tab, &logical, row, window)),
                    _ => None,
                }).unwrap_or_default()
            }).collect());
            let sticky: Vec<_> = sticky.into_iter().map(|(line, row)| (row, tab.and_then(|t| state.presentation_view(t.id())).and_then(|v| v.fold_at(line)).map(|(_, c)| c))).collect();
            if g.pane == state.focused_pane() {
                marks_drawn = decor.clone();
                folds_drawn.extend(marks.iter().map(|&(i, c, _)| (i as f32, c)).chain(sticky.iter().filter_map(|(r, fold)| fold.map(|c| (r.y, c)))));
            }
            let row_parts = tab.filter(|_| csv.is_none()).and_then(|tab| state.presentation_view(tab.id()).map(|view| (tab, view))).map_or_else(Vec::new, |(tab, view)| {
                let logical = view.layout_rows(g.rows, state.wrap_cols(tab)).rows;
                rows.iter().map(|row| logical.get(row.y as usize).map(|r| self.source_parts(tab, r, row, window)).unwrap_or_default()).collect()
            });
            #[cfg(feature = "test-support")]
            painted_markers.extend(change_marks.iter().map(|&(_, glyph, color)| (glyph, color)));
            frames.push(PaneFrame { relative_caret: tab.and_then(|t| state.relative_number_origin(t.id())), inline: Vec::new(), csv, row_parts, sticky_parts, preview_button: self.preview_button(*g), find: self.find_box(g), geom: *g, focused: g.pane == focused && split.is_some(), tabs, rows, marks, bookmarks, sticky_bookmarks, comparison_marks, change_marks, sticky_change, link, decor, hscroll, picture, markdown, pending, unshowable, hint, start, sticky, sticky_decor, ann, scrollbar });
            if let Some(frame) = frames.last_mut() { frame.inline = self.inline_annotations(frame, window); }
        }
        #[cfg(feature = "test-support")]
        { self.painted_markers = painted_markers; self.painted_scrollbars = painted_scrollbars; self.painted_inline = frames.iter().flat_map(|frame| frame.inline.iter().cloned().map(|annotation| (frame.geom.pane, annotation))).collect(); }
        self.source_geometry = [0, 1].map(|pane| self.session.state().presentation_shown(pane).filter(|tab| self.session.state().csv(tab.id()).is_none()).and_then(|tab| self.session.state().presentation_view(tab.id())).and_then(|view| SourceGeometry::new(&view, self)));
        self.sticky_hits = sticky_hits;
        self.start_hits = start_hits;
        self.marks_drawn = marks_drawn;
        self.fold_marks_drawn = folds_drawn;
        self.uploads.keep(frames.iter().flat_map(|f| f.picture.iter().map(|(t, _)| t.clone()).chain(f.markdown.iter().flat_map(|m| m.2.iter().map(|(_, t)| t.clone())))).collect(), window);
        let navigation = self.navigation_frame();
        let form = self.form_layout();
        let search = self.search_layout();
        let modal = self.modal_layout();
        let theme_chooser = matches!(self.modal, Some(Modal::Themes { .. }));
        let menu_modal = matches!(self.modal, Some(Modal::AppMenu { .. } | Modal::CommandGroup { .. }));
        // The context menu: its box, and per row its top, label, key, whether it can run and is highlighted.
        let doc_frame = self.doc_frame(window);
        let completion_frame = self.completion_frame(window, cx);
        #[cfg(feature = "test-support")]
        { self.painted_completion_doc = completion_frame.as_ref().and_then(|f| f.2.as_ref()).and_then(|(_, layout, _)| layout.as_ref()).map(|layout| layout.preview.doc.text.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()); }
        let signature_frame = self.signature_frame(window);
        let code_actions_frame = self.code_actions_frame(window);
        self.clamp_context();
        let context = self.context_layout().map(|(b, rows)| {
            let at = self.modal.as_ref().map_or(0, Modal::at);
            let rows: Vec<_> = rows
                .into_iter()
                .enumerate()
                .map(|(i, (top, h, e))| match e {
                    MenuEntry::Command(id) => (top, h, Some((id.label(), self.key_hint(id), self.session.disabled(id).is_none(), i == at))),
                    MenuEntry::Separator => (top, h, None),
                })
                .collect();
            (b, rows)
        });
        let (font, char_w, split_at) = (self.font.clone(), self.char_w, self.split_at);
        let results_focus = self.results_focus;
        #[cfg(feature = "perf-probe")]
        if let Some(text) = probe::find()
            && frames.iter().any(|f| !f.rows.is_empty())
        {
            match probe::FIND_ASKED.get() {
                None => {
                    let _ = probe::FIND_ASKED.set(std::time::Instant::now());
                    self.session.dispatch(Command::Find(text));
                    window.request_animation_frame();
                }
                Some(_) => match self.session.state().active().and_then(|t| t.find()).filter(|f| !f.pending()) {
                    Some(f) if probe::found(f.matches().len()) => cx.quit(),
                    _ => window.request_animation_frame(),
                },
            }
        }
        #[cfg(feature = "perf-probe")]
        if probe::wrap() && !probe::WRAPPED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            self.session.dispatch(Command::SetSoftWrap(true));
        }
        #[cfg(feature = "perf-probe")]
        if let Some(seconds) = probe::scroll_seconds()
            && frames.iter().any(|f| !f.rows.is_empty())
            && (!probe::decor() || self.session.state().active().and_then(|t| self.session.state().presentation_view(t.id())).is_some_and(|v| v.inlay_hints().is_some_and(|h| !h.is_empty()) && v.tokens().is_some_and(|k| !k.is_empty())))
        {
            if probe::decor() && !probe::DECORATED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("lenz-code: decorated {:.1} ms", probe::START.get().map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0));
            }
            if probe::frame(seconds) {
                cx.quit();
            } else {
                if probe::fold() && self.session.state().active().and_then(|t| self.session.state().presentation_view(t.id())).is_some_and(|v| v.folds().is_some()) && !probe::FOLDED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    self.session.dispatch(Command::CollapseAll);
                }
                // One row further each frame (over collapsed regions), wrapping at the end.
                let at_end = self.session.state().active().is_none_or(|t| { let Some(view) = self.session.state().presentation_view(t.id()) else { return true }; let v = view.visible(); v.rows_between(t.top_line(), v.last()) < self.rows });
                self.session.dispatch(if at_end { Command::Scroll(0) } else { Command::ScrollRows(1) });
                window.request_animation_frame();
            }
        }
        #[cfg(feature = "perf-probe")]
        let probe = {
            let state = self.session.state();
            let wanted = probe::files();
            (frames.iter().any(|f| !f.rows.is_empty() || f.markdown.is_some() || f.picture.is_some()), !wanted.is_empty() && state.loading().is_none() && state.active().is_some_and(|t| state.presentation_view(t.id()).is_some()) && state.tabs().len() >= wanted.len())
        };
        #[cfg(feature = "perf-probe")]
        if render_started.elapsed().as_millis() >= 30 {
            eprintln!("lenz-code: slow render {:.1} ms", render_started.elapsed().as_secs_f64() * 1000.0);
        }
        let details_kind = match self.modal { Some(Modal::StatusDetails { kind, .. }) => Some(kind), _ => None };
        let details_modal = details_kind.is_some();
        let checkout_danger: Vec<_> = if details_kind == Some(Details::Checkout) { self.session.checkout_choices().into_iter().map(|(_, c)| matches!(c, code_core::branches::CheckoutChoice::Force | code_core::branches::CheckoutChoice::ConfirmForce)).collect() } else { Vec::new() };
        let details_buttons = match &self.modal {
            Some(Modal::StatusDetails { kind, .. }) => self.details_labels(*kind),
            _ => Vec::new(),
        };
        let details_disabled = match &self.modal {
            Some(Modal::StatusDetails { kind: Details::Install, .. }) => Some(self.session.server_install_actions().into_iter().map(|id| self.session.disabled(id).is_some()).chain(std::iter::once(false)).collect::<Vec<_>>()),
            Some(Modal::StatusDetails { kind: Details::External, .. }) => self.session.external_prompt()
                .map(|prompt| code_core::external::Choice::ALL.map(|choice| prompt.disabled(choice).is_some()).to_vec()),
            Some(Modal::StatusDetails { kind: Details::Reconcile, .. }) => self.reconcile_disabled(),
            Some(Modal::StatusDetails { kind: Details::Close, .. }) => self.session.close_prompt().map(|p| vec![p.save_disabled.is_some(), p.discard_disabled.is_some(), false]),
            Some(Modal::StatusDetails { kind: Details::Recovered, .. }) => self.session.recovery_prompt().map(|p| code_core::recovery::Choice::ALL.map(|c| p.disabled(c).is_some()).to_vec()),
            _ => None,
        };
        // The hovered annotation's commit, beside the pointer.
        let ann_tip = self.ann_hovered().zip(self.ann_hover).and_then(|((_, t, line), at)| {
            let a = self.session.annotation(t, line).filter(|a| a.shade.is_some())?;
            let c = a.commit;
            let lines = [format!("{} · {} · {}", c.sha.get(..7).unwrap_or(&c.sha), c.author, lenz_content::blame::date(c.author_time)), c.summary.clone()];
            let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32 * self.char_w + 16.0;
            let x = (f32::from(at.x) + 12.0).min(self.width - w - 4.0).max(0.0);
            Some((x, f32::from(at.y) + 16.0, w, lines))
        });
        let about_logo = match &self.modal {
            Some(Modal::StatusDetails { logo: Some(logo), .. }) => Some(logo.0.clone()),
            _ => None,
        };
        let nav_hover = self.nav_hover;
        let preview_hover = self.preview_hover;
        let app_menu_button = self.app_menu_button();
        let app_menu_active = self.app_menu_hover || matches!(self.modal, Some(Modal::AppMenu { .. } | Modal::CommandGroup { .. }));
        let commands_button = self.commands_button();
        let commands_hover = self.commands_hover;
        let activity_button = self.activity_button();
        let results_button = self.results_button();
        let activity = self.activity_layout();
        let activity_height = self.activity_height();
        let terminal_height = self.terminal_height();
        self.resize_terminal(cx);
        let terminal = self.terminal_paint();
        let terminal_drop = cx.weak_entity();
        let text_input = {
            self.input_state.registered.set(false);
            (!self.log_focus && (self.terminal_active() || self.draft_target().is_some() || self.csv_field().is_some()) && self.focus.is_focused(window)).then(|| (self.focus.clone(), cx.entity().clone(), self.input_state.registered.clone()))
        };
        #[cfg(feature = "test-support")]
        let painted_problem_bounds = { self.painted_problem_bounds.borrow_mut().clear(); self.painted_problem_bounds.clone() };
        div()
            .id("reader")
            .size_full()
            .bg(rgb(palette.bg))
            .track_focus(&self.focus)
            .on_drop(cx.listener(|v, paths: &gpui::ExternalPaths, window, cx| {
                if window.viewport_size() == gpui::size(px(v.width), px(v.height)) {
                    v.terminal_ui.external_paths = None;
                    v.terminal_drop_paths(paths.paths(), window.mouse_position(), cx);
                }
            }))
            .on_drag_move(cx.listener(|v,event: &gpui::DragMoveEvent<gpui::ExternalPaths>,_,cx| {
                v.terminal_ui.external_paths = v.modal.is_none().then(|| event.drag(cx).paths().to_vec());
            }))
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll(cx.listener(Self::on_wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_down(MouseButton::Middle, cx.listener(|v,ev,window,cx| { if v.modal.is_none() { v.terminal_click(ev,window,cx); } }))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_down))
            // The mouse's back and forward side buttons, as Alt+Left and Alt+Right (a dialog open keeps them).
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Back), cx.listener(|v, _: &MouseDownEvent, window, cx| v.side_button(CommandId::Back, window, cx)))
            .on_mouse_down(MouseButton::Navigate(gpui::NavigationDirection::Forward), cx.listener(|v, _: &MouseDownEvent, window, cx| v.side_button(CommandId::Forward, window, cx)))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_modifiers_changed(cx.listener(Self::on_modifiers))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let terminal_drop = terminal_drop.clone();
                        window.on_mouse_event(move |event: &gpui::MouseUpEvent,phase,window,cx| {
                            // GPUI's hover filter suppresses on_drop after keyboard input, including native file drops.
                            if phase == gpui::DispatchPhase::Bubble && window.last_input_was_keyboard() && cx.has_active_drag() {
                                let handled = terminal_drop.update(cx,|view,cx| {
                                    view.terminal_ui.external_paths.take().is_some_and(|paths| view.terminal_drop_paths(&paths,event.position,cx))
                                }).unwrap_or(false);
                                if handled {cx.stop_active_drag(window); cx.stop_propagation();}
                            }
                        });
                        if let Some((focus, entity, registered)) = &text_input {
                            window.handle_input(focus, gpui::ElementInputHandler::new(bounds, entity.clone()), cx);
                            registered.set(true);
                        }
                        #[cfg(feature = "perf-probe")]
                        let paint_started = std::time::Instant::now();
                        let m = metrics(&font);
                        let (x0, y0, w, h) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height));
                        if let Some(cursor) = cursor {
                            window.set_window_cursor_style(cursor);
                        }
                        lenz_gpui::fill(window, x0, y0, w, toolbar_h() + tabs_h, palette.header_bg);
                        lenz_gpui::fill(window, x0, y0 + toolbar_h() - 1.0, w, 1.0, palette.border);
                        let mx = x0 + f32::from(app_menu_button.origin.x);
                        let mw = f32::from(app_menu_button.size.width);
                        if app_menu_active { lenz_gpui::fill(window, mx, y0 + 3.0, mw, toolbar_h() - 6.0, palette.selected_bg); }
                        for offset in [-4.0, 0.0, 4.0] {
                            lenz_gpui::fill(window, mx + mw / 2.0 - 6.0, y0 + toolbar_h() / 2.0 + offset, 12.0, 1.0, palette.fg);
                        }
                        // The buttons: dim when they cannot run, raised under the pointer.
                        for (bx0, bx1, id, enabled, label) in nav.iter().chain(&explorer_buttons) {
                            let locate = *id == CommandId::LocateFile;
                            let (by, bh) = if locate { (y0 + toolbar_h(), tabs_h) } else { (y0, toolbar_h()) };
                            let drawn = matches!(id, CommandId::ToggleExplorer | CommandId::LocateFile);
                            if *enabled && nav_hover == Some(*id) {
                                window.paint_quad(gpui::fill(Bounds { origin: gpui::point(px(x0 + bx0), px(by + 3.0)), size: gpui::size(px(bx1 - bx0), px(bh - 6.0)) }, rgb(palette.selected_bg)).corner_radii(px(4.0)));
                            }
                            let color = if *enabled { palette.fg } else { palette.dim };
                            if drawn {
                                paint_symbol(window, *id, x0 + (bx0 + bx1) / 2.0, by + bh / 2.0, sidebar_w > 0.0, color);
                            } else {
                                let width = lenz_content::wrap::width(label) as f32 * char_w;
                                lenz_gpui::text(window, cx, m, x0 + bx0 + (bx1 - bx0 - width) / 2.0, by + (bh - line_h()) / 2.0, label, color);
                            }
                        }
                        // Each pane's tabs: the shown one raised (with an accent on the focused pane's while split), a
                        // preview tab dim, each with its close mark; never over the buttons.
                        for f in &frames {
                            let (bx0, bx1, by) = f.geom.bar;
                            let y = y0 + by;
                            if by > 0.0 {
                                lenz_gpui::fill(window, x0 + bx0, y, bx1 - bx0, tabs_h, palette.header_bg);
                            }
                            let tab_end = f.preview_button.as_ref().map_or(bx1, |(b, _)| f32::from(b.left()) - 4.0);
                            let bar = Bounds { origin: gpui::point(px(x0 + bx0), px(y)), size: gpui::size(px((tab_end - bx0).max(0.0)), px(tabs_h)) };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: bar }), |window| {
                                for (title, retained, shown, tx0, tx1, dirty, status, recovered) in &f.tabs {
                                    if *tx1 <= bx0 { continue; }
                                    if *tx0 >= tab_end { break; }
                                    if *shown {
                                        lenz_gpui::fill(window, x0 + tx0, y, tx1 - tx0, tabs_h, palette.bg);
                                        if f.focused {
                                            lenz_gpui::fill(window, x0 + tx0, y, tx1 - tx0, 2.0, palette.accent);
                                        }
                                    }
                                    let color = if *retained { palette.fg } else { palette.dim };
                                    if file_icons {
                                        paint_icon(window, cx, icon_font.as_ref(), &logos, bar, &palette, icons::file(title), x0 + tx0 + char_w, y);
                                    }
                                    let title_x = x0 + tx0 + char_w + icon_w;
                                    // A recovered draft says so beside its unsaved mark.
                                    if *recovered { lenz_gpui::text(window, cx, m, title_x, y, "⟲", palette.dim); }
                                    let title_x = title_x + if *recovered { 2.0 * char_w } else { 0.0 };
                                    if *dirty { lenz_gpui::text(window, cx, m, title_x, y, "●", palette.accent); }
                                    let name_x = title_x + if *dirty { 2.0 * char_w } else { 0.0 };
                                    lenz_gpui::text(window, cx, m, name_x, y, title, color);
                                    // The Git status letter after the name; its color is extra.
                                    if let Some(s) = status {
                                        let at = name_x + (lenz_content::wrap::width(title) + 1) as f32 * char_w;
                                        lenz_gpui::text(window, cx, m, at, y, s.letter(), s.color(&palette));
                                    }
                                    lenz_gpui::text(window, cx, m, x0 + tx1 - char_w * 2.0, y, "×", palette.dim);
                                    lenz_gpui::fill(window, x0 + tx1 - 1.0, y + 3.0, 1.0, tabs_h - 6.0, palette.border);
                                }
                                if let Some((_, dx)) = drop_x.filter(|(p, _)| *p == f.geom.pane) {
                                    lenz_gpui::fill(window, x0 + dx - 1.0, y + 2.0, 2.0, tabs_h - 4.0, palette.accent);
                                }
                            });
                            if let Some((b, action)) = &f.preview_button {
                                let x = x0 + f32::from(b.left());
                                let width = f32::from(b.size.width);
                                lenz_gpui::fill(window, x, y + 2.0, width, tabs_h - 4.0, if preview_hover == Some(f.geom.pane) { palette.selected_bg } else { palette.fold_bg });
                                let text_w = action.label.len() as f32 * char_w;
                                lenz_gpui::text(window, cx, m, x + (width - text_w) / 2.0, y, action.label, if action.disabled.is_none() { palette.fg } else { palette.dim });
                            }
                        }
                        // The status bar along the bottom, over the whole width.
                        if status_h > 0.0 {
                            let status_y = y0 + h - status_h;
                            lenz_gpui::fill(window, x0, status_y, w, status_h, palette.header_bg);
                            lenz_gpui::fill(window, x0, status_y, w, 1.0, palette.border);
                            for ((b, label), color, action) in [(&server_cell, server_color, Some(CommandId::LanguageServer)), (&sql_cell, palette.accent, Some(CommandId::SqlDialect)), (&markers_cell, palette.fg, Some(CommandId::StatusDetails)), (&message_cell, palette.fg, Some(CommandId::StatusDetails)), (&position_cell, palette.dim, None)] {
                                let clip = Bounds { origin: gpui::point(px(x0 + f32::from(b.origin.x)), px(status_y)), size: b.size };
                                window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                                    if action.is_some() && nav_hover == action { lenz_gpui::fill(window, f32::from(clip.origin.x), status_y + 2.0, f32::from(clip.size.width), status_h - 4.0, palette.selected_bg); }
                                    lenz_gpui::text(window, cx, m, f32::from(clip.origin.x), status_y + 2.0, label, color);
                                });
                            }
                            if let Some((b, label)) = &commands_button {
                                let bx = x0 + f32::from(b.origin.x);
                                let bw = f32::from(b.size.width);
                                lenz_gpui::fill(window, bx, status_y + 2.0, bw, status_h - 4.0, if commands_hover { palette.selected_bg } else { palette.fold_bg });
                                lenz_gpui::text(window, cx, m, bx + char_w, status_y + 2.0, label, palette.fg);
                            }
                        }
                        for (b, label) in activity_button.iter().chain(&results_button) {
                            lenz_gpui::text(window, cx, m, x0 + f32::from(b.left()), y0 + f32::from(b.top()) + 2.0, label, palette.accent);
                        }
                        if let Some(log) = &activity {
                            let top = y0 + f32::from(log.bounds.top());
                            lenz_gpui::fill(window, x0, top, w, activity_height, palette.sidebar_bg);
                            lenz_gpui::fill(window, x0, top, w, line_h(), palette.header_bg);
                            let title_width = log.buttons.last().map_or(w, |(b, _, _)| f32::from(b.left()));
                            let title = code_core::chrome::ellipsize("Activity Log", (title_width / char_w) as usize);
                            lenz_gpui::text(window, cx, m, x0 + 4.0, top, &title, palette.fg);
                            for (b, _, label) in &log.buttons { lenz_gpui::text(window, cx, m, x0 + f32::from(b.left()) + 3.0, top, label, palette.accent); }
                            for (i, row) in log.rows.iter().skip(log.start).take(log.visible).enumerate() {
                                lenz_gpui::text(window, cx, m, x0 + 6.0, top + (i + 1) as f32 * line_h(), row, palette.fg);
                            }
                        }
                        let body_h = (h - body_top - status_h - activity_height - terminal_height).max(0.0);
                        let body = Bounds { origin: gpui::point(px(x0), px(y0 + body_top)), size: gpui::size(px(sidebar_w), px(body_h)) };
                        if sidebar_w > 0.0 {
                            // The explorer: its own panel from the toolbar row down, titled with the folder's name.
                            let title_w = explorer_buttons.first().map_or(sidebar_w, |b| b.0) - 8.0;
                            let title = Bounds { origin: gpui::point(px(x0), px(y0 + toolbar_h())), size: gpui::size(px(title_w.max(0.0)), px(tabs_h)) };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: title }), |window| {
                                lenz_gpui::text(window, cx, m, x0 + 8.0, y0 + toolbar_h(), &explorer_title, palette.dim);
                            });
                            lenz_gpui::fill(window, x0, y0 + body_top, sidebar_w, body_h, palette.sidebar_bg);
                            window.with_content_mask(Some(gpui::ContentMask { bounds: body }), |window| {
                                for (i, line) in tree.iter().enumerate() {
                                    let y = y0 + body_top + i as f32 * line_h();
                                    if line.selected {
                                        lenz_gpui::fill(window, x0, y, sidebar_w, line_h(), if tree_focus { palette.selected_bg } else { palette.fold_bg });
                                    } else if line.marked {
                                        lenz_gpui::fill(window, x0, y, sidebar_w, line_h(), palette.fold_bg);
                                    }
                                    // Chevron, icon and name, indented by depth.
                                    let x = x0 + 6.0 + line.depth as f32 * 2.0 * char_w;
                                    if let Some(open) = line.expanded {
                                        lenz_gpui::text(window, cx, m, x, y, if open { "▾" } else { "▸" }, palette.dim);
                                    }
                                    if file_icons {
                                        paint_icon(window, cx, icon_font.as_ref(), &logos, body, &palette, line.icon, x + 1.5 * char_w, y);
                                    }
                                    let (match_bg, match_fg) = match_colors(&palette);
                                    let bg: Vec<_> = line.highlight.iter().map(|range| (range.clone(), match_bg)).collect();
                                    let fg: Vec<_> = line.highlight.iter().map(|range| (range.clone(), match_fg)).collect();
                                    lenz_gpui::colored_text(window, cx, m, if line.error { palette.error } else if line.hidden { palette.dim } else { palette.fg }, x + 1.5 * char_w + icon_w, y, &line.name, &fg, &bg);
                                    // At the row's end, right to left: the Compare Selected check, the Git status letter (its
                                    // color is extra) and the unsaved-draft mark, each over the name if it is long.
                                    let row_bg = if line.selected { if tree_focus { palette.selected_bg } else { palette.fold_bg } } else if line.marked { palette.fold_bg } else { palette.sidebar_bg };
                                    let ends = [line.marked.then_some(("✓", palette.fg)), line.status.map(|s| (s.letter(), s.color(&palette))), line.dirty.then_some(("●", palette.accent)), line.error.then_some(("•", palette.error))];
                                    let mut at = x0 + sidebar_w - 2.0 * char_w;
                                    for (symbol, color) in ends.into_iter().flatten() {
                                        lenz_gpui::fill(window, at - char_w * 0.5, y, char_w * 2.0, line_h(), row_bg);
                                        lenz_gpui::text(window, cx, m, at, y, symbol, color);
                                        at -= 2.0 * char_w;
                                    }
                                }
                            });
                            lenz_gpui::fill(window, x0 + sidebar_w - 1.0, y0 + toolbar_h(), 1.0, body_h + tabs_h, palette.border);
                        }
                        // Breadcrumb bars: crumbs and their `›`, the highlighted crumb on a selection box.
                        for c in &crumb_cells {
                            if let Some(bg) = c.bg {
                                lenz_gpui::fill(window, x0 + c.x, y0 + c.y, c.w, line_h(), bg);
                            }
                            if !c.text.is_empty() {
                                lenz_gpui::text(window, cx, m, x0 + c.x + 4.0, y0 + c.y, &c.text, c.color);
                            }
                        }
                        #[cfg(feature = "perf-probe")]
                        probe::painted(probe.0, probe.1);
                        for f in &frames {
                            let g = &f.geom;
                            let text = Bounds { origin: gpui::point(px(x0) + g.text.origin.x, px(y0) + g.text.origin.y), size: g.text.size };
                            let (tx, ty, tw) = (f32::from(text.origin.x), f32::from(text.origin.y), f32::from(text.size.width));
                            #[cfg(feature = "test-support")]
                            let mut record_problem = |bounds, color| painted_problem_bounds.borrow_mut().push((g.pane, bounds, color));
                            #[cfg(not(feature = "test-support"))]
                            let mut record_problem = |_: Bounds<Pixels>, _: u32| {};
                            if g.ann > 0.0 {
                                // The blame column: each line's date and author on its commit's age shade.
                                let (ax, aw) = (tx - g.ann, g.ann - 2.0);
                                lenz_gpui::fill(window, ax, ty, g.ann, f32::from(text.size.height), palette.gutter_bg);
                                let column = Bounds { origin: gpui::point(px(ax), px(ty)), size: gpui::size(px(aw), text.size.height) };
                                window.with_content_mask(Some(gpui::ContentMask { bounds: column }), |window| {
                                    for (y, label, shade, hot) in &f.ann {
                                        let bg = match (shade, hot) {
                                            (_, true) => palette.blame_new,
                                            (Some(s), false) => lenz_content::theme::mix(palette.gutter_bg, palette.blame_new, *s * 0.6),
                                            (None, false) => palette.gutter_bg,
                                        };
                                        lenz_gpui::fill(window, ax, ty + y * m.line_h, aw, m.line_h, bg);
                                        lenz_gpui::text(window, cx, m, ax + 6.0, ty + y * m.line_h, label, if *hot || shade.is_some() { palette.fg } else { palette.dim });
                                    }
                                });
                            }
                            if g.bar_strip.size.width > px(0.0) {
                                // The scrollbar: its track, the thumb, and over both each marked cell (the caret a 1 px line).
                                let (sx, sy, sw, sh) = (x0 + f32::from(g.bar_strip.origin.x), y0 + f32::from(g.bar_strip.origin.y), f32::from(g.bar_strip.size.width), f32::from(g.bar_strip.size.height));
                                lenz_gpui::fill(window, sx, sy, sw, sh, scrollbar::track_color(&palette));
                                if let Some((start, size)) = f.scrollbar.0 { lenz_gpui::fill(window, sx + 2.0, sy + start, sw - 4.0, size, scrollbar_thumb_color(&palette)); }
                                let cell_h = sh / track_cells(sh).max(1) as f32;
                                for &(cell, kind) in &f.scrollbar.1 {
                                    let (y, h) = if kind == MarkKind::Caret { (sy + cell as f32 * cell_h + 1.0, 1.0) } else { (sy + cell as f32 * cell_h, 3.0) };
                                    lenz_gpui::fill(window, sx + 1.0, y, sw - 2.0, h, scrollbar_mark_color(kind, &palette));
                                }
                            }
                            window.with_content_mask(Some(gpui::ContentMask { bounds: text }), |window| {
                                if let Some(csv) = &f.csv { csv.paint(window, cx, m, &palette); }
                                if let Some(hint) = f.hint {
                                    lenz_gpui::text(window, cx, m, tx + 24.0, ty + 24.0, hint, palette.dim);
                                }
                                for (row, label, target) in &f.start {
                                    lenz_gpui::text(window, cx, m, tx + 24.0, ty + 24.0 + row * m.line_h, label, if *target { palette.accent } else { palette.dim });
                                }
                                for (i, row) in f.rows.iter().enumerate() {
                                    paint_source_row(window, cx, m, &palette, text, tx, tw, f.hscroll, row, f.relative_caret, f.row_parts.get(i).map_or(&[], Vec::as_slice));
                                }
                                for &(row, collapsed, bright) in &f.marks {
                                    let color = if bright { palette.fg } else { palette.num_fg };
                                    lenz_gpui::text(window, cx, m, tx + num_w() - fold_w() + 1.0, ty + row as f32 * m.line_h, if collapsed { "▸" } else { "▾" }, color);
                                }
                                paint_bookmarks(window, cx, m, tx + 2.0, ty, &f.bookmarks, palette.accent);
                                paint_bookmarks(window, cx, m, tx, ty, &f.comparison_marks, palette.edge[2]);
                                paint_change_marks(window, cx, m, tx, ty, &f.change_marks);
                                if let Some((ry, shown)) = &f.link
                                    && let Some(row) = f.rows.iter().find(|r| r.y == *ry)
                                {
                                    let parts = f.rows.iter().position(|row| row.y == *ry).and_then(|i| f.row_parts.get(i)).map_or(&[][..], Vec::as_slice);
                                    let (a, b) = (native_x(window, m, row, parts, shown.start), native_x(window, m, row, parts, shown.end));
                                    let lx = lenz_gpui::text_x(m, tx, f.hscroll);
                                    window.with_content_mask(Some(lenz_gpui::clip(text, tx + m.num_w + 4.0, tw - m.num_w - 4.0)), |window| {
                                        lenz_gpui::fill(window, lx + a, ty + ry * m.line_h + m.line_h - 3.0, b - a, 1.0, palette.syntax.link);
                                    });
                                }
                                window.with_content_mask(Some(lenz_gpui::clip(text, tx + m.num_w + 4.0, tw - m.num_w - 4.0)), |window| {
                                    paint_decor(window, m, lenz_gpui::text_x(m, tx, f.hscroll), ty, |y| f.rows.iter().enumerate().find(|(_, r)| r.y == y).map(|(i, row)| (row, f.row_parts.get(i).map_or(&[][..], Vec::as_slice))), &f.decor, &mut record_problem);
                                });
                                inline_diagnostics::paint(window, cx, m, text, &f.inline, false);
                                if let Some((texture, (iw, ih))) = &f.picture {
                                    // Fitted to the view, never enlarged, centred, over a checkerboard.
                                    let (aw, ah) = ((tw - 16.0).max(1.0), (f32::from(text.size.height) - 16.0).max(1.0));
                                    let scale = (aw / *iw as f32).min(ah / *ih as f32).min(1.0);
                                    let (dw, dh) = (*iw as f32 * scale, *ih as f32 * scale);
                                    let (ix, iy) = (tx + (tw - dw) / 2.0, ty + (f32::from(text.size.height) - dh) / 2.0);
                                    lenz_gpui::image::checker(window, text, (ix, iy, dw, dh));
                                    let area = Bounds { origin: gpui::point(px(ix), px(iy)), size: gpui::size(px(dw), px(dh)) };
                                    let _ = window.paint_image(text, area, Default::default(), texture.clone(), 0, false);
                                }
                                if let Some((layout, top, textures, selection)) = &f.markdown {
                                    layout.paint(window, cx, text, *top, selection.clone(), |window, b, at| {
                                        if let Some((_, t)) = textures.iter().find(|(i, _)| *i == b) {
                                            let _ = window.paint_image(text, at, Default::default(), t.clone(), 0, false);
                                        }
                                    });
                                } else if f.pending {
                                    lenz_gpui::text(window, cx, m, tx + 24.0, ty + 24.0, "Rendering the preview…", palette.dim);
                                }
                                if let Some(why) = &f.unshowable {
                                    lenz_gpui::text(window, cx, m, tx + 24.0, ty + 24.0, why, palette.dim);
                                }
                                if let Some(n) = f.sticky.iter().map(|(r, _)| r.y + 1.0).reduce(f32::max) {
                                    lenz_gpui::fill(window, tx, ty, tw, n * m.line_h, palette.header_bg);
                                    for (i, (row, _)) in f.sticky.iter().enumerate() {
                                        paint_source_row(window, cx, m, &palette, text, tx, tw, f.hscroll, row, f.relative_caret, f.sticky_parts.get(i).map_or(&[], Vec::as_slice));
                                    }
                                    window.with_content_mask(Some(lenz_gpui::clip(text, tx + m.num_w + 4.0, tw - m.num_w - 4.0)), |window| {
                                        paint_decor(window, m, lenz_gpui::text_x(m, tx, f.hscroll), ty, |y| f.sticky.iter().enumerate().find(|(_, (r, _))| r.y == y).map(|(i, (row, _))| (row, f.sticky_parts.get(i).map_or(&[][..], Vec::as_slice))), &f.sticky_decor, &mut record_problem);
                                    });
                                    inline_diagnostics::paint(window, cx, m, text, &f.inline, true);
                                    // Their own fold marks, over the marks of the rows the band hides (those are not drawn).
                                    for (row, fold) in &f.sticky {
                                        if let Some(collapsed) = fold {
                                            lenz_gpui::text(window, cx, m, tx + num_w() - fold_w() + 1.0, ty + row.y * m.line_h, if *collapsed { "▸" } else { "▾" }, palette.num_fg);
                                        }
                                    }
                                    lenz_gpui::fill(window, tx, ty + n * m.line_h - 1.0, tw, 1.0, palette.border);
                                    paint_bookmarks(window, cx, m, tx + 2.0, ty, &f.sticky_bookmarks, palette.accent);
                                    paint_change_marks(window, cx, m, tx, ty, &f.sticky_change);
                                }
                                if let Some(b) = &f.find {
                                    b.paint(window, cx, m, &palette, gpui::point(px(x0), px(y0)), char_w);
                                }
                            });
                        }
                        if let Some((tx, ty, tw, lines)) = &ann_tip {
                            let (bg, border) = palette.raised();
                            let th = lines.len() as f32 * line_h() + 8.0;
                            lenz_gpui::fill(window, x0 + tx - 1.0, y0 + ty - 1.0, tw + 2.0, th + 2.0, border);
                            lenz_gpui::fill(window, x0 + tx, y0 + ty, *tw, th, bg);
                            for (i, l) in lines.iter().enumerate() {
                                lenz_gpui::text(window, cx, m, x0 + tx + 8.0, y0 + ty + 4.0 + i as f32 * line_h(), l, if i == 0 { palette.accent } else { palette.fg });
                            }
                        }
                        // The split's divider.
                        match split {
                            Some(Split::Columns) => lenz_gpui::fill(window, x0 + split_at, y0, 1.0, panel_top - tabs_h + tabs_h, palette.border),
                            Some(Split::Rows) if frames.len() > 1 => lenz_gpui::fill(window, x0 + sidebar_w, y0 + split_at, w - sidebar_w, 1.0, palette.border),
                            _ => {}
                        }
                        if let Some(status) = &status {
                            let (px0, pw) = (f32::from(bounds.origin.x), f32::from(bounds.size.width));
                            let top = y0 + panel_top;
                            lenz_gpui::fill(window, px0, top, pw, h - status_h - activity_height - terminal_height - panel_top, palette.sidebar_bg);
                            lenz_gpui::fill(window, px0, top, pw, 1.0, palette.border);
                            let panel = Bounds { origin: gpui::point(px(px0), px(top)), size: gpui::size(px(pw), px((h - status_h - activity_height - terminal_height - panel_top).max(0.0))) };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: panel }), |window| {
                                // What was asked in bold, then what was found; the keys at the right when they fit.
                                let bold = Font { weight: gpui::FontWeight::BOLD, ..m.font.clone() };
                                let title_x = px0 + 8.0 + lenz_content::wrap::width(&status.title) as f32 * char_w;
                                lenz_gpui::text(window, cx, Metrics { font: &bold, ..m }, px0 + 8.0, top, &status.title, if status.error { palette.error } else { palette.fg });
                                lenz_gpui::text(window, cx, m, title_x + 2.0 * char_w, top, &status.detail, palette.dim);
                                // The buttons at the right; the keys left of them when they fit.
                                let buttons_x = results_buttons.first().map_or(px0 + pw, |(b, _, _)| x0 + f32::from(b.left()));
                                // A narrow heading's text runs under the buttons, which stay readable over it.
                                lenz_gpui::fill(window, buttons_x, top + 1.0, (px0 + pw - buttons_x).max(0.0), line_h() - 1.0, palette.sidebar_bg);
                                for (b, _, label) in &results_buttons {
                                    lenz_gpui::text(window, cx, m, x0 + f32::from(b.left()) + char_w, top, label, palette.accent);
                                }
                                let hint_w = lenz_content::wrap::width(&status.hint) as f32 * char_w;
                                if title_x + (lenz_content::wrap::width(&status.detail) as f32 + 6.0) * char_w + hint_w < buttons_x {
                                    lenz_gpui::text(window, cx, m, buttons_x - 8.0 - hint_w, top, &status.hint, palette.dim);
                                }
                                let list_pw = pw.min(list_w);
                                let list_area = Bounds { origin: gpui::point(px(px0), px(top)), size: gpui::size(px(list_pw), panel.size.height) };
                                window.with_content_mask(Some(gpui::ContentMask { bounds: list_area }), |window| for (i, row) in list.iter().enumerate() {
                                    let y = top + (i + 1) as f32 * line_h();
                                    if row.selected {
                                        lenz_gpui::fill(window, px0, y, list_pw, line_h(), if results_focus { palette.selected_bg } else { palette.fold_bg });
                                    }
                                    let fg = if row.stale { palette.dim } else { palette.fg };
                                    let bg: Vec<_> = row.highlights.iter().map(|r| (r.clone(), match_colors(&palette).0)).collect();
                                    // Indented by depth; a folder or file with its chevron and icon.
                                    let mut x = px0 + 8.0 + row.depth as f32 * 2.0 * char_w;
                                    if let Some((icon, open)) = row.node {
                                        lenz_gpui::text(window, cx, m, x, y, if open { "▾" } else { "▸" }, palette.dim);
                                        if file_icons {
                                            paint_icon(window, cx, icon_font.as_ref(), &logos, panel, &palette, icon, x + 1.5 * char_w, y);
                                        }
                                        x += 1.5 * char_w + icon_w;
                                    }
                                    let row_fg: &[_] = if row.stale { &[] } else { &row.fg };
                                    lenz_gpui::colored_text(window, cx, m, fg, x, y, &row.label, row_fg, &bg);
                                });
                                // Beside the list in a wide window, else below it: a line, then the preview.
                                let (prev_x, prev_w, preview_top) = if list_w < pw {
                                    lenz_gpui::fill(window, px0 + list_w, top + line_h(), 1.0, f32::from(panel.size.height), palette.border);
                                    (px0 + list_w + 1.0, pw - list_w - 1.0, top + line_h())
                                } else {
                                    let preview_top = top + (list.len().max(1) + 1) as f32 * line_h();
                                    lenz_gpui::fill(window, px0, preview_top, pw, 1.0, palette.border);
                                    (px0, pw, preview_top)
                                };
                                let pane_h = (h - status_h - activity_height - terminal_height - (preview_top - y0)).max(0.0);
                                // The preview names its file and line; its rows start below that.
                                if let Some(title) = &preview_title && !preview.is_empty() {
                                    lenz_gpui::text(window, cx, m, prev_x + 8.0, preview_top, title, palette.dim);
                                }
                                let pane = Bounds { origin: gpui::point(px(prev_x), px(preview_top + line_h())), size: gpui::size(px(prev_w), px((pane_h - line_h()).max(0.0))) };
                                window.with_content_mask(Some(gpui::ContentMask { bounds: pane }), |window| for row in &preview {
                                    lenz_gpui::paint_row(window, cx, m, &palette, pane, prev_x, prev_w, 0.0, row);
                                });
                            });
                        }
                        if let Some(terminal) = &terminal { terminal.paint(window, cx, m, &palette, x0, y0); }
                        if let Some(frame) = &navigation {
                            frame.paint(window, cx, m, &palette, x0, y0, icon_font.as_ref(), &logos);
                        }
                        if let Some(dialog) = &search {
                            dialog.paint(window, cx, m, &palette, x0, y0);
                        }
                        if let Some(form) = &form { form.paint(window, cx, m, &palette, x0, y0); }
                        if let Some(l) = &modal {
                            let (mx, my) = (x0 + f32::from(l.bounds.origin.x), y0 + f32::from(l.bounds.origin.y));
                            let (mw, mh) = (f32::from(l.bounds.size.width), f32::from(l.bounds.size.height));
                            let (bg, border) = palette.raised();
                            lenz_gpui::fill(window, mx - 1.0, my - 1.0, mw + 2.0, mh + 2.0, border);
                            lenz_gpui::fill(window, mx, my, mw, mh, bg);
                            let menu = Bounds { origin: gpui::point(px(mx), px(my)), size: l.bounds.size };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: menu }), |window| {
                                if let Some(title) = &l.heading {
                                    let heading = Font { weight: gpui::FontWeight::BOLD, ..m.font.clone() };
                                    lenz_gpui::text(window, cx, Metrics { font: &heading, ..m }, mx + 12.0, my + 12.0, title, palette.fg);
                                    if let Some(query) = &l.query {
                                        let fy = my + line_h() + 20.0;
                                        let fw = (mw - 24.0).max(0.0);
                                        lenz_gpui::fill(window, mx + 12.0, fy, fw, line_h() + 8.0, palette.accent);
                                        lenz_gpui::fill(window, mx + 13.0, fy + 1.0, (fw - 2.0).max(0.0), line_h() + 6.0, palette.bg);
                                        let shown = navigation::tail(query, ((fw - 20.0) / char_w).max(0.0) as usize);
                                        lenz_gpui::text(window, cx, m, mx + 20.0, fy + 4.0, if query.is_empty() { "Filter branches…" } else { &shown }, if query.is_empty() { palette.dim } else { palette.fg });
                                        lenz_gpui::fill(window, mx + 20.0 + lenz_content::wrap::width(&shown) as f32 * char_w, fy + 4.0, 2.0, line_h(), palette.caret);
                                    }
                                }
                                if details_modal {
                                    for (index,(b, &label)) in l.details_buttons(char_w, &details_buttons).iter().zip(&details_buttons).enumerate() {
                                        let (x, y) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y));
                                        let clip = Bounds { origin: gpui::point(px(x), px(y)), size: b.size };
                                        window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                                            let disabled = details_disabled.as_ref().is_some_and(|d| d.get(index).copied().unwrap_or(false));
                                            let destructive = match details_kind {
                                                Some(Details::Close | Details::Recovered | Details::Change) => index == 1,
                                                Some(Details::Reconcile) => matches!(index, 1 | 2),
                                                Some(Details::Replace | Details::Terminal { .. }) => index == 0,
                                                Some(Details::Checkout) => checkout_danger.get(index).copied().unwrap_or(false),
                                                _ => false,
                                            };
                                            let primary = index == 0 && !destructive;
                                            let edge = if disabled { border } else if destructive { palette.error } else if primary { palette.accent } else { border };
                                            let hover = clip.contains(&window.mouse_position());
                                            lenz_gpui::fill(window, x, y, f32::from(b.size.width), f32::from(b.size.height), edge);
                                            lenz_gpui::fill(window, x + 1.0, y + 1.0, (f32::from(b.size.width) - 2.0).max(0.0), (f32::from(b.size.height) - 2.0).max(0.0), if !disabled && (primary || hover) { palette.selected_bg } else { bg });
                                            let shown = code_core::chrome::ellipsize(label, ((f32::from(b.size.width) - 16.0) / char_w).max(0.0) as usize);
                                            lenz_gpui::text(window, cx, m, x + 8.0, y + 6.0, &shown, if disabled { palette.dim } else if destructive { palette.readable_on(palette.error, bg) } else { palette.fg });
                                        });
                                    }
                                }
                                for (n, (row, line, disabled)) in l.lines.iter().enumerate().skip(l.first).take(l.visible) {
                                    let y = my + l.body_top + (n - l.first) as f32 * line_h();
                                    // A menu's separator: a line across, never highlighted.
                                    if menu_modal && line.is_empty() {
                                        lenz_gpui::fill(window, mx + 8.0, y + line_h() / 2.0, (mw - 16.0).max(0.0), 1.0, border);
                                        continue;
                                    }
                                    if !details_modal && *row != usize::MAX && *row == l.at {
                                        lenz_gpui::fill(window, mx, y, mw, line_h(), palette.selected_bg);
                                    }
                                    if (n == 0 || l.lines[n - 1].0 != *row) && let Some((_, key)) = l.shortcuts.iter().find(|(r, _)| r == row) {
                                        let kx = mx + mw - 12.0 - lenz_content::wrap::width(key) as f32 * char_w;
                                        if lenz_content::wrap::width(key) + 3 < ((mw - 24.0) / char_w).max(0.0) as usize / 2 { lenz_gpui::text(window, cx, m, kx, y, key, palette.dim); }
                                    }
                                    let section = details_modal && line.split_once(':').is_some_and(|(label, _)| label.len() <= 40 && !label.contains('/') && !label.contains('`'));
                                    let heading = Font { weight: gpui::FontWeight::BOLD, ..m.font.clone() };
                                    lenz_gpui::text(window, cx, if section { Metrics { font: &heading, ..m } } else { m }, mx + 12.0, y, line, if *row == usize::MAX { palette.readable_on(palette.dim, bg) } else if *disabled { palette.dim } else if theme_chooser && *row < THEME_HEADER_ROWS { palette.accent } else { palette.fg });
                                }
                                if !l.footer.is_empty() {
                                    let y = my + l.body_top + l.visible as f32 * line_h() + if l.heading.is_some() { 8.0 } else { 0.0 };
                                    lenz_gpui::fill(window, mx + 8.0, y, (mw - 16.0).max(0.0), 1.0, border);
                                    for (i, text) in l.footer.iter().enumerate() {
                                        lenz_gpui::text(window, cx, m, mx + 12.0, y + 1.0 + i as f32 * line_h(), text, palette.readable_on(palette.dim, bg));
                                    }
                                }
                                // About's logo, right of its short lines, when the box has room for it.
                                if let Some(logo) = about_logo.as_ref().filter(|_| mw >= 520.0 && mh >= 160.0) {
                                    let at = Bounds { origin: gpui::point(px(mx + mw - 112.0), px(my + 12.0)), size: gpui::size(px(96.0), px(96.0)) };
                                    let _ = window.paint_image(menu, at, Default::default(), logo.clone(), 0, false);
                                }
                            });
                        }
                        if let Some((b, layout, top)) = &doc_frame {
                            let (bx, by, bw, bh) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                            lenz_gpui::fill(window, bx - 1.0, by - 1.0, bw + 2.0, bh + 2.0, palette.border);
                            lenz_gpui::fill(window, bx, by, bw, bh, palette.sidebar_bg);
                            let area = Bounds { origin: gpui::point(px(bx), px(by)), size: b.size };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: area }), |window| layout.paint(window, cx, area, *top, None, |_, _, _| {}));
                        }
                        if let Some((b, lines)) = &signature_frame {
                            let (mx, my, mw, mh) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                            let (bg, border) = palette.raised();
                            lenz_gpui::fill(window, mx - 1.0, my - 1.0, mw + 2.0, mh + 2.0, border);
                            lenz_gpui::fill(window, mx, my, mw, mh, bg);
                            let clip = Bounds { origin: gpui::point(px(mx), px(my)), size: b.size };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                                for (i, (text, active, dim)) in lines.iter().enumerate() {
                                    let y = my + 2.0 + i as f32 * line_h();
                                    if let Some((before, param)) = active.as_ref().and_then(|r| Some((text.get(..r.start)?, text.get(r.clone())?))) {
                                        let start = before.chars().count() as f32;
                                        lenz_gpui::fill(window, mx + (start + 1.0) * char_w, y, param.chars().count() as f32 * char_w, line_h(), palette.selected_bg);
                                    }
                                    lenz_gpui::text(window, cx, m, mx + char_w, y, text, if *dim { palette.dim } else { palette.fg });
                                }
                            });
                        }
                        if let Some((b, rows, doc)) = &completion_frame {
                            let (mx, my, mw, mh) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                            let (bg, border) = palette.raised();
                            lenz_gpui::fill(window, mx - 1.0, my - 1.0, mw + 2.0, mh + 2.0, border);
                            lenz_gpui::fill(window, mx, my, mw, mh, bg);
                            let clip = Bounds { origin: gpui::point(px(mx), px(my)), size: b.size };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                                for (i, ((label, detail, selected, unavailable), matches, kind, group)) in rows.iter().enumerate() {
                                    let y = my + 2.0 + i as f32 * line_h();
                                    if *group { lenz_gpui::fill(window, mx, y, mw, 1.0, border); }
                                    if *selected {
                                        lenz_gpui::fill(window, mx, y, mw, line_h(), palette.selected_bg);
                                        lenz_gpui::fill(window, mx, y, 3.0, line_h(), palette.accent);
                                    }
                                    let colors: Vec<_> = matches.iter().cloned().map(|range| (range, palette.accent)).collect();
                                    let label_x = mx + char_w;
                                    let shaped = lenz_gpui::colored_text(window, cx, m, if *unavailable {palette.dim} else {palette.fg}, label_x, y, label, &colors, &[]);
                                    let end = label_x + shaped.map_or(label.chars().count() as f32 * char_w, |line| f32::from(line.width));
                                    let kind = kind.unwrap_or("");
                                    let kind_width = (kind.chars().count() as f32 + 2.0) * char_w;
                                    if !kind.is_empty() {
                                        let kind_x = (mx + mw - kind_width).max(mx + mw * 0.55);
                                        lenz_gpui::fill(window, kind_x, y, mw - (kind_x-mx), line_h(), if *selected {palette.selected_bg} else {bg});
                                        lenz_gpui::text(window, cx, m, kind_x + char_w, y, kind, palette.accent);
                                    }
                                    let right = mx + mw - kind_width;
                                    if !detail.is_empty() && end + 3.0 * char_w < right {
                                        let detail_clip = Bounds {origin:gpui::point(px(end+2.0*char_w),px(y)), size:gpui::size(px((right-end-2.0*char_w).max(0.0)),px(line_h()))};
                                        window.with_content_mask(Some(gpui::ContentMask{bounds:detail_clip}), |window| lenz_gpui::text(window,cx,m,end+2.0*char_w,y,detail,palette.dim));
                                    }
                                }
                            });
                            if let Some((d, layout, top)) = doc {
                                let (dx, dy, dw, dh) = (x0 + f32::from(d.origin.x), y0 + f32::from(d.origin.y), f32::from(d.size.width), f32::from(d.size.height));
                                lenz_gpui::fill(window, dx - 1.0, dy - 1.0, dw + 2.0, dh + 2.0, border);
                                lenz_gpui::fill(window, dx, dy, dw, dh, bg);
                                let clip = Bounds { origin: gpui::point(px(dx), px(dy)), size: d.size };
                                window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                                    if let Some(layout)=layout {layout.paint(window, cx, clip, *top, None, |_, _, _| {});}
                                    else {lenz_gpui::text(window,cx,m,dx+char_w,dy+2.0,"Loading documentation…",palette.dim);}
                                });
                            }
                        }
                        if let Some((b, rows, reason)) = &code_actions_frame {
                            let (mx, my, mw, mh) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                            let (bg, border) = palette.raised();
                            lenz_gpui::fill(window, mx - 1.0, my - 1.0, mw + 2.0, mh + 2.0, border);
                            lenz_gpui::fill(window, mx, my, mw, mh, bg);
                            let clip = Bounds { origin: gpui::point(px(mx), px(my)), size: b.size };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                                for (i, (label, _, selected, unavailable)) in rows.iter().enumerate() {
                                    let y = my + 2.0 + i as f32 * line_h();
                                    if *selected { lenz_gpui::fill(window, mx, y, mw, line_h(), palette.selected_bg); }
                                    lenz_gpui::text(window, cx, m, mx + char_w, y, label, if *unavailable { palette.dim } else { palette.fg });
                                }
                                if let Some(reason) = reason { lenz_gpui::text(window, cx, m, mx + char_w, my + 2.0 + rows.len() as f32 * line_h(), reason, palette.dim); }
                            });
                        }
                        if let Some((b, rows)) = &context {
                            let (mx, my, mw, mh) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                            let (bg, border) = palette.raised();
                            lenz_gpui::fill(window, mx - 1.0, my - 1.0, mw + 2.0, mh + 2.0, border);
                            lenz_gpui::fill(window, mx, my, mw, mh, bg);
                            let clip = Bounds { origin: gpui::point(px(mx), px(my)), size: b.size };
                            window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                                for (top, h, row) in rows {
                                    let y = y0 + top;
                                    match row {
                                        None => lenz_gpui::fill(window, mx + 6.0, y + h / 2.0, mw - 12.0, 1.0, palette.border),
                                        Some((label, key, enabled, highlighted)) => {
                                            if *highlighted { lenz_gpui::fill(window, mx, y, mw, *h, palette.selected_bg); }
                                            lenz_gpui::text(window, cx, m, mx + 2.0 * char_w, y, label, if *enabled { palette.fg } else { palette.dim });
                                            if let Some(key) = key {
                                                lenz_gpui::text(window, cx, m, mx + mw - (key.chars().count() as f32 + 2.0) * char_w, y, key, palette.dim);
                                            }
                                        }
                                    }
                                }
                            });
                        }
                        #[cfg(feature = "perf-probe")]
                        if paint_started.elapsed().as_millis() >= 30 {
                            eprintln!("lenz-code: slow paint {:.1} ms", paint_started.elapsed().as_secs_f64() * 1000.0);
                        }
                    },
                )
                .size_full(),
            )
    }
}

fn wanted_image(view: &code_core::session::View<'_>) -> Option<Arc<lenz_content::image::Pixels>> {
    let image = view.snapshot().image()?;
    let pixels = image.pixels.as_ref().ok()?;
    (image.format != lenz_content::image::Format::Svg || !view.tab().image_source()).then(|| pixels.clone())
}

/// Why `tab`'s raster image cannot be shown, when it cannot (an SVG that cannot be rendered shows its source
/// instead, with the reason in the status bar).
fn unshowable_image(view: &code_core::session::View<'_>) -> Option<String> {
    let image = view.snapshot().image()?;
    if image.format == lenz_content::image::Format::Svg {
        return None;
    }
    image.pixels.as_ref().err().map(|e| format!("This {} image cannot be shown: {e}", image.format.name()))
}

/// A Markdown preview's layout, its scroll and its visible images' textures.
type PreviewFrame = (Arc<preview::Layout>, f32, Vec<(usize, Arc<gpui::RenderImage>)>, Option<std::ops::Range<usize>>);
/// A tab in its bar: title, kept, shown, x range, unsaved draft marker.
/// And its Git status letter.
type TabCell = (String, bool, bool, f32, f32, bool, Option<lenz_content::branches::FileStatus>, bool);

/// One pane's frame to paint: where it is, its tabs (title, kept, shown, x range), and what it shows.
/// A piece of a breadcrumb bar to paint.
struct CrumbCell {
    x: f32,
    y: f32,
    w: f32,
    text: String,
    color: u32,
    bg: Option<u32>,
}

struct PaneFrame {
    relative_caret: Option<usize>,
    inline: Vec<inline_diagnostics::Annotation>,
    csv: Option<csv::Frame>,
    geom: PaneGeom,
    preview_button: Option<(Bounds<Pixels>, code_core::session::Action)>,
    /// The pane's find box, when its tab has a find.
    find: Option<FindBox>,
    /// The focused pane while split (its shown tab is marked).
    focused: bool,
    tabs: Vec<TabCell>,
    rows: Vec<TextRow>,
    /// Sticky lines over the top rows (syntax colours only), each with its fold mark (collapsed) when it starts one.
    sticky: Vec<(TextRow, Option<bool>)>,
    /// The sticky lines' underlines and strikethroughs: row, display range, colour, modifier bits.
    sticky_decor: Vec<(f32, std::ops::Range<usize>, u32, u8)>,
    /// Fold marks: row, collapsed, drawn bright.
    marks: Vec<(usize, bool, bool)>,
    /// Bookmarks on text rows and on sticky lines: row (in lines from the top) and key.
    bookmarks: Vec<(f32, Option<char>)>,
    sticky_bookmarks: Vec<(f32, Option<char>)>,
    comparison_marks: Vec<(f32, Option<char>)>,
    /// The draft's change markers on text rows: row, its letter, and the letter's colour.
    change_marks: Vec<(f32, char, u32)>,
    /// The same on the sticky lines pinned over them.
    sticky_change: Vec<(f32, char, u32)>,
    /// The linked identifier's row and display range.
    link: Option<(f32, std::ops::Range<usize>)>,
    /// Underlines (mutable) and strikethroughs (deprecated): row, display range, colour, modifier bits.
    decor: Vec<(f32, std::ops::Range<usize>, u32, u8)>,
    hscroll: f32,
    row_parts: Vec<Vec<(std::ops::Range<usize>, f32)>>,
    sticky_parts: Vec<Vec<(std::ops::Range<usize>, f32)>>,
    picture: Option<(Arc<gpui::RenderImage>, (u32, u32))>,
    markdown: Option<PreviewFrame>,
    pending: bool,
    unshowable: Option<String>,
    hint: Option<&'static str>,
    /// The start screen's rows: row, text, and whether it is a click target (not a heading).
    start: Vec<(f32, String, bool)>,
    /// Blame annotations: row, label, age shade (`None`: not committed), of the hovered commit.
    ann: Vec<(f32, String, Option<f32>, bool)>,
    /// The scrollbar's thumb (start, size) and each marked track cell with its mark's kind.
    scrollbar: DrawnBar,
}

/// A scrollbar as drawn: its thumb (start, size) and each marked track cell with its mark's kind.
type DrawnBar = (Option<(f32, f32)>, Vec<(usize, MarkKind)>);
/// For tests: a pane's scrollbar as painted: its pane, thumb (start, size) and mark kinds top to bottom.
#[cfg(feature = "test-support")]
pub type PaintedScrollbar = (u8, Option<(f32, f32)>, Vec<MarkKind>);

fn native_x(window: &Window, metrics: Metrics<'_>, row: &TextRow, parts: &[(std::ops::Range<usize>, f32)], at: usize) -> f32 {
    let Some((range, origin)) = parts.iter().rev().find(|(r, _)| r.start <= at && at <= r.end) else { return 0.0 };
    let Some(text) = row.text.get(range.clone()) else { return *origin };
    debug_assert!(text.len() <= 6144);
    let run = gpui::TextRun { len: text.len(), font: metrics.font.clone(), color: gpui::Hsla::default(), background_color: None, underline: None, strikethrough: None };
    origin + f32::from(window.text_system().shape_line(text.to_owned().into(), px(metrics.font_size), &[run], None).x_for_index(at - range.start))
}

fn display_number(number: u32, relative_caret: Option<usize>) -> u32 {
    relative_caret.filter(|&caret| number > 0 && number as usize != caret.saturating_add(1))
        .map_or(number, |caret| number.abs_diff(caret.saturating_add(1).min(u32::MAX as usize) as u32))
}

#[allow(clippy::too_many_arguments)]
fn paint_source_row(window: &mut Window, cx: &mut App, metrics: Metrics<'_>, palette: &lenz_content::theme::Palette, bounds: Bounds<Pixels>, x: f32, width: f32, scroll: f32, row: &TextRow, relative_caret: Option<usize>, parts: &[(std::ops::Range<usize>, f32)]) {
    for (i, (part, origin)) in parts.iter().enumerate() {
        let cut = |range: &std::ops::Range<usize>| {
            let (start, end) = (range.start.max(part.start), range.end.min(part.end));
            (start < end).then(|| start - part.start..end - part.start)
        };
        let fragment = TextRow {
            y: row.y, number: if i == 0 { display_number(row.number, relative_caret) } else { 0 },
            text: row.text.get(part.clone()).unwrap_or_default().to_owned(),
            fg: row.fg.iter().filter_map(|(r, c)| Some((cut(r)?, *c))).collect(),
            bg: row.bg.iter().filter_map(|(r, c)| Some((cut(r)?, *c))).collect(),
            selected: row.selected.as_ref().and_then(cut),
            caret: row.caret.filter(|at| part.start <= *at && (*at < part.end || i + 1 == parts.len() && *at == part.end)).map(|at| at - part.start),
            extra: row.extra.iter().filter_map(|(range, caret)| {
                let range = cut(range).unwrap_or(0..0);
                let caret = caret.filter(|at| part.start <= *at && (*at < part.end || i + 1 == parts.len() && *at == part.end)).map(|at| at - part.start);
                (caret.is_some() || !range.is_empty()).then_some((range, caret))
            }).collect(),
        };
        debug_assert!(fragment.text.len() <= 6144);
        lenz_gpui::paint_row(window, cx, metrics, palette, bounds, x, width, scroll - origin, &fragment);
    }
}

/// Bookmark marks at the gutter's left edge (line numbers are right-aligned): the key, or a dot when it has none.
fn paint_bookmarks(window: &mut Window, cx: &mut App, m: Metrics, x: f32, ty: f32, marks: &[(f32, Option<char>)], color: u32) {
    for &(row, key) in marks {
        let glyph = key.map_or_else(|| "●".to_string(), String::from);
        lenz_gpui::text(window, cx, m, x, ty + row * m.line_h, &glyph, color);
    }
}

/// The draft change markers in the gutter's left margin: the change's letter, then the deletion above it. Bookmarks
/// keep the third cell, so a changed bookmarked line shows both.
fn paint_change_marks(window: &mut Window, cx: &mut App, m: Metrics, tx: f32, ty: f32, marks: &[(f32, char, u32)]) {
    let cell = lenz_gpui::cell_w(m.font_size);
    for &(row, glyph, color) in marks {
        let x = tx + 2.0 + if glyph == 'D' { cell } else { 0.0 };
        lenz_gpui::text(window, cx, m, x, ty + row * m.line_h, &glyph.to_string(), color);
    }
}

/// Underlines (mutable) and strikethroughs (deprecated) on the rows `row_at` finds by their y, drawn from `lx` (the
/// text's left edge after scrolling) under the pane top `ty`: row, display range, colour and modifier bits each.
fn paint_decor<'a>(window: &mut Window, m: Metrics, lx: f32, ty: f32, row_at: impl Fn(f32) -> Option<(&'a TextRow, &'a [(std::ops::Range<usize>, f32)])>, decor: &[(f32, std::ops::Range<usize>, u32, u8)], mut record_problem: impl FnMut(Bounds<Pixels>, u32)) {
    for (ry, r, color, mods) in decor {
        let Some((row, parts)) = row_at(*ry) else { continue };
        let x = |b: usize| native_x(window, m, row, parts, b);
        let (a, w, y) = (x(r.start), x(r.end) - x(r.start), ty + ry * m.line_h);
        if mods & code_core::decor::MUTABLE != 0 {
            lenz_gpui::fill(window, lx + a, y + m.line_h - 2.0, w, 1.0, *color);
        }
        if mods & code_core::decor::DEPRECATED != 0 {
            lenz_gpui::fill(window, lx + a, y + m.line_h / 2.0, w, 1.0, *color);
        }
        if mods & code_core::decor::PROBLEM != 0 {
            lenz_gpui::fill(window, lx + a, y + m.line_h - 2.0, w.max(2.0), 2.0, *color);
            record_problem(Bounds { origin: gpui::point(px(lx + a), px(y + m.line_h - 2.0)), size: gpui::size(px(w.max(2.0)), px(2.0)) }, *color);
        }
    }
}

/// A toolbar button drawn rather than labelled, centred on `(cx, cy)`: the explorer's switch (a window with its
/// side panel, filled while `shown`) or the select-opened-file target.
fn paint_symbol(window: &mut Window, id: CommandId, cx: f32, cy: f32, shown: bool, color: u32) {
    let fill = |window: &mut Window, x: f32, y: f32, w: f32, h: f32| lenz_gpui::fill(window, (cx + x).round(), (cy + y).round(), w, h, color);
    match id {
        CommandId::ToggleExplorer => {
            for (x, y, w, h) in [(-7.0, -6.0, 14.0, 1.0), (-7.0, 5.0, 14.0, 1.0), (-7.0, -6.0, 1.0, 12.0), (6.0, -6.0, 1.0, 12.0), (-3.0, -6.0, 1.0, 12.0)] {
                fill(window, x, y, w, h);
            }
            if shown {
                fill(window, -6.0, -5.0, 3.0, 10.0);
            }
        }
        _ => {
            // A ring with four ticks and a centre dot.
            for (x, y, w, h) in [(-4.0, -4.0, 8.0, 1.0), (-4.0, 3.0, 8.0, 1.0), (-4.0, -4.0, 1.0, 8.0), (3.0, -4.0, 1.0, 8.0), (-7.0, -0.5, 3.0, 1.0), (4.0, -0.5, 3.0, 1.0), (-0.5, -7.0, 1.0, 3.0), (-0.5, 4.0, 1.0, 3.0), (-1.0, -1.0, 2.0, 2.0)] {
                fill(window, x, y, w, h);
            }
        }
    }
}

/// One visible line of the project tree.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeLine {
    pub depth: usize,
    /// `Some(expanded)` for a folder.
    pub expanded: Option<bool>,
    pub icon: icons::Icon,
    pub name: String,
    pub highlight: Vec<std::ops::Range<usize>>,
    pub error: bool,
    pub selected: bool,
    /// Marked for Compare Selected (Ctrl+click).
    pub marked: bool,
    /// A dotfile, or inside a dot-folder: its name is greyed out.
    pub hidden: bool,
    /// Its Git status against HEAD, shown as a letter.
    pub status: Option<lenz_content::branches::FileStatus>,
    /// Unsaved changes in a tab: a mark independent of the Git status.
    pub dirty: bool,
}

/// Space between an icon and the name after it.
const ICON_GAP: f32 = 6.0;

/// The width a file-type icon and its gap take before a name (none with File Icons off).
fn icon_w(file_icons: bool) -> f32 {
    if file_icons { icons::SIZE + ICON_GAP } else { 0.0 }
}

/// A file or folder icon in the [`icons::SIZE`] square at `x` on the row at `y`: its logo, else its Nerd Font glyph,
/// else a shape in its colour (a folder with a tab, a page for a file).
#[allow(clippy::too_many_arguments)]
fn paint_icon(window: &mut Window, cx: &mut App, nerd: Option<&gpui::Font>, logos: &std::collections::HashMap<&'static str, Arc<gpui::RenderImage>>, clip: Bounds<Pixels>, p: &lenz_content::theme::Palette, icon: icons::Icon, x: f32, y: f32) {
    let top = y + (line_h() - icons::SIZE) / 2.0;
    if let Some(texture) = icon.logo.and_then(|l| logos.get(l.name)) {
        let at = Bounds { origin: gpui::point(px(x), px(top)), size: gpui::size(px(icons::SIZE), px(icons::SIZE)) };
        let _ = window.paint_image(clip, at, Default::default(), texture.clone(), 0, false);
        return;
    }
    let color = icon.color(p);
    match nerd {
        Some(font) => lenz_gpui::text(window, cx, Metrics { font, font_size: icons::glyph_size(icon, &font.family), line_h: line_h(), num_w: num_w() }, x, y, &icon.glyph.to_string(), color),
        None if icon.folder => {
            lenz_gpui::fill(window, x, top + 2.0, icons::SIZE * 0.45, 3.0, color);
            lenz_gpui::fill(window, x, top + 4.0, icons::SIZE, icons::SIZE - 6.0, color);
        }
        None => lenz_gpui::fill(window, x + 2.0, top + 1.0, icons::SIZE - 4.0, icons::SIZE - 2.0, color),
    }
}

/// One visible row of the results panel: a result, or a folder or file of a project's matches.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    pub label: String,
    /// The matches within `label`.
    pub highlights: Vec<std::ops::Range<usize>>,
    pub selected: bool,
    /// Its file changed after the search read it.
    pub stale: bool,
    /// Text colours within `label` (the first covering one wins): the match, then syntax.
    pub fg: Vec<(std::ops::Range<usize>, u32)>,
    /// Tree depth.
    pub depth: usize,
    /// A folder or file: its icon and whether it is unfolded.
    pub node: Option<(icons::Icon, bool)>,
    /// The result it shows.
    pub hit: Option<usize>,
}

fn metrics(font: &Font) -> Metrics<'_> {
    Metrics { font, font_size: font_size(), line_h: line_h(), num_w: num_w() }
}

/// A context menu row: its top, height and entry.
type MenuRow = (f32, f32, MenuEntry);
fn context_row_height(entry: MenuEntry) -> f32 { if matches!(entry, MenuEntry::Separator) { line_h() / 2.0 } else { line_h() } }

/// The word (letters, digits, `_`) around byte `at` of `text`, as (start, end); the byte itself when none.
fn word_around(text: &str, at: usize) -> (usize, usize) {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let (before, after) = (text.get(..at).unwrap_or_default(), text.get(at..).unwrap_or_default());
    let start = before.char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(at, |(i, _)| i);
    let end = after.char_indices().take_while(|(_, c)| word(*c)).last().map_or(at, |(i, c)| at + i + c.len_utf8());
    (start, end)
}

fn menu_width(window_w: f32) -> f32 {
    (window_w - 32.0).clamp(0.0, 480.0)
}

/// `text` in lines of at most `cols` characters, breaking between words where it can.
pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let cols = cols.max(1);
    let (mut lines, mut line, mut n) = (Vec::new(), String::new(), 0);
    for word in text.split_inclusive(' ') {
        if n > 0 && n + lenz_content::wrap::width(word.trim_end()) > cols {
            lines.push(std::mem::take(&mut line));
            n = 0;
        }
        for c in word.chars() {
            let width = lenz_content::wrap::width(c.encode_utf8(&mut [0; 4]));
            if n > 0 && n + width > cols {
                lines.push(std::mem::take(&mut line));
                n = 0;
            }
            line.push(c);
            n += width;
        }
    }
    lines.push(line);
    lines
}

impl Modal {
    fn is_form(&self) -> bool {
        matches!(self, Self::Create { .. } | Self::SaveAs { .. } | Self::NewScratch { .. } | Self::RenameTerminal { .. } | Self::Relocate { .. } | Self::PasteEntry { .. } | Self::ConfirmDelete { .. } | Self::NewBranch { .. } | Self::Revision { .. } | Self::GoTo { .. } | Self::Root { .. } | Self::SqlMetadata { .. })
    }

    /// The highlighted row of [`ReaderView::modal_rows`].
    pub fn at(&self) -> usize {
        match self {
            Modal::Navigation { at } => *at,
            Modal::Commands { at } | Modal::AppMenu { at } | Modal::CommandGroup { at, .. } | Modal::LanguageServer { at } | Modal::Recent { at } => *at,
            Modal::ScratchFiles { at, .. } => *at + 1,
            Modal::Themes { at, .. } => *at + THEME_HEADER_ROWS,
            Modal::ServerRuntime { at, .. } | Modal::Siblings { at, .. } | Modal::Places { at, .. } => *at + FINDER_HEADER_ROWS,
            Modal::Search { in_files, .. } => *in_files as usize,
            Modal::StatusDetails { .. } | Modal::Declarations { .. } | Modal::Root { .. } | Modal::SqlMetadata { .. } | Modal::GoTo { .. } | Modal::Mnemonic | Modal::NewBranch { .. } | Modal::Revision { .. } | Modal::Create { .. } | Modal::PasteEntry { .. } | Modal::SaveAs { .. } | Modal::NewScratch { .. } | Modal::RenameTerminal { .. } | Modal::Relocate { .. } | Modal::ConfirmDelete { .. } => 0,
            Modal::Context { at, .. } => *at,
            Modal::Branches { at, .. } => *at + FINDER_HEADER_ROWS,
            Modal::BranchMenu { at, .. } => *at + 1,
            Modal::Keymap { at, .. } => *at + 1,
        }
    }
}

fn dark(window: &Window) -> bool {
    matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark)
}

#[cfg(all(test, feature = "test-support"))]
mod tests;

impl Drop for ReaderView {
    fn drop(&mut self) { self.remember_explorer(); }
}

/// A list's first row after a wheel event: whole rows move it, the fraction waits for the next event (a trackpad's
/// small movements add up, as in the text) and is dropped at an edge so reversing responds at once.
fn list_wheel(fraction: &mut f32, delta: ScrollDelta, first: usize, max: usize) -> usize {
    if matches!(delta, ScrollDelta::Lines(_)) {
        *fraction = 0.0;
    }
    let rows = lenz_gpui::scroll::steps(fraction, lenz_gpui::scroll::pixels(delta, line_h(), 1.0).y / line_h());
    let first = first.saturating_add_signed(rows).min(max);
    lenz_gpui::scroll::clamp(fraction, first == 0, first == max);
    first
}
