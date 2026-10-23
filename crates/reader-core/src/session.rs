//! The reader session: every semantic state change goes through [`Session::dispatch`], whether it comes from
//! the user ([`Command`]) or from background work ([`Completion`]). Adapters read [`Session::state`] and
//! [`Session::actions`]; nothing hands out mutable state.

pub use crate::breadcrumbs::{Breadcrumbs, Crumb, Sibling, SiblingTarget};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use lenz_content::{
    highlight::{self, FoldKind, Folds, Language, Parsed},
    theme::{Palette, ThemeCommand, ThemeEntry, ThemeOverrides, ThemePaths, ThemeService, ThemeUpdate},
};

use crate::{
    cache::{INACTIVE_BYTES, InactiveCache, Stamp},
    decor::Decor,
    document::Document,
    jobs::{Jobs, Priority},
    project::{self, Finder, Project},
    index::{self, DeclIndex, FileIndex},
    search::{self, Results, ResultsKind, Scope},
    semantics::{
        self, ProviderView, Semantics,
        hierarchy::{self, Call, Direction, Hierarchy},
    },
    tabs::{History, Location, Tab, TabId},
    watch::{Change, Watcher},
};

/// Reads retried while a file changes under them before the reader gives up for now.
pub const READ_ATTEMPTS: u32 = 5;
/// Later rechecks of a file that kept changing during every read, at growing intervals.
pub const UNSTABLE_RETRIES: u32 = 8;

/// Background results waiting for the UI at most; producers block beyond this.
pub const COMPLETIONS: usize = 64;

/// Recent browsing roots kept.
pub const RECENT_ROOTS: usize = 10;
/// How often idle language servers are checked and their memory sampled.
pub const TICK: std::time::Duration = std::time::Duration::from_secs(5);
/// How long a symbol search waits for the slowest server it asked.
pub const SYMBOLS_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A selection as source byte offsets on character boundaries; `anchor == head` is a caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn range(&self) -> std::ops::Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    PrepareProjection(crate::projection::Request),
    PrepareLayout { pane: u8, tab: TabId, work: crate::layout::Work },
    CancelProjection { pane: u8, request: u64 },
    SearchEverywhere { scope: crate::navigation::Scope, query: String },
    CloseSearchEverywhere,
    /// Forget one recent-file result only if its displayed identity is still current.
    ForgetNavigation { revision: u64, index: usize },
    /// Add an anonymous bookmark on the caret's line, or remove the one there.
    ToggleBookmark,
    /// Key `key` (0–9, A–Z) on the caret line's bookmark (adding it); its own key removes it, a key used elsewhere
    /// moves here.
    SetMnemonic(char),
    /// Go to the bookmark with key `key` (a history step; its fold opens).
    JumpMnemonic(char),
    /// The next (or previous) bookmark by path then line, wrapping.
    StepBookmark { forward: bool },
    /// Go to a listed place (a history step; its fold opens).
    GoToPlace(Place),
    /// Remove a listed place from its list.
    ForgetPlace(Place),
    /// Find which listed files no longer exist (off the UI thread), for the lists' "missing" marks.
    CheckPlaces,
    /// Open a source in the reusable preview tab (or activate the tab already showing it).
    Open(PathBuf),
    /// Open a source in a retained tab, as a double-click or pin does.
    OpenRetained(PathBuf),
    ActivateTab(TabId),
    CloseTab(TabId),
    /// Retain a tab: the next preview opens elsewhere.
    Pin(TabId),
    Back,
    Forward,
    /// The active tab's horizontal position, in display columns.
    ScrollColumn(usize),
    /// Clamp a pane using its adapter's measured extent, rejecting obsolete source or width identities.
    ClampColumn { pane: u8, tab: TabId, identity: crate::projection::Cursor, cols: usize, max: usize },
    /// Check every open source against the disk (focus regained, or notifications unavailable).
    Recheck,
    /// Search the project or the active document for literal text, case-insensitively.
    Search { query: String, scope: SearchIn },
    /// Find this text in the focused tab's document with the session's find options (the newest query wins);
    /// opens the tab's find and selects the first match at or after the caret.
    Find(String),
    /// The next (or previous) match of the focused tab's find, wrapping at the ends.
    FindStep { forward: bool },
    /// Close the focused tab's find and drop its matches.
    CloseFind,
    /// Flip one find option (case, whole word, regular expression); an open find runs again with it.
    ToggleFindOption(lenz_content::find::FindOption),
    /// Put the caret at a 1-based line and column of the active document (clamped to it), its fold open; a step in
    /// history (Back returns).
    GoToLine(lenz_content::goto::LineColumn),
    /// Open `path` (as [`Command::Open`]) and go to `at` once it is shown.
    OpenAt { path: PathBuf, at: lenz_content::goto::LineColumn },
    /// Wrap long lines at the pane's width (remembered).
    SetSoftWrap(bool),
    /// Show a project file or folder (project-relative) in the explorer, opening the folders above it.
    RevealInTree(Arc<str>),
    /// Pin the enclosing declarations' first lines at the top of each pane (remembered).
    SetStickyScroll(bool),
    /// Show the path and the declarations at the caret above each pane's text (remembered).
    SetBreadcrumbs(bool),
    /// Show the language server's type hints (remembered).
    SetInlayTypes(bool),
    /// Show the language server's parameter-name hints (remembered).
    SetInlayParameters(bool),
    /// Colour names by the language server's semantic tokens (remembered).
    SetSemanticHighlighting(bool),
    /// A pane's text area in character columns and rows (sent by the adapter when it changes).
    SetTextSize { pane: u8, cols: usize, rows: usize },
    /// The focused tab's find query, every match listed in the results panel.
    ListMatches,
    /// Fold or unfold a folder or file of a project's matches.
    ToggleResultGroup(search::Group),
    /// Unfold the folder and file holding a result, so its row shows.
    RevealResult(usize),
    /// Colour these results' lines (those shown), off the UI thread; each line once.
    ColourResults(Vec<usize>),
    RerunSearch,
    CloseResults,
    /// The active document's declarations in the results panel, in source order.
    Outline,
    /// Declarations across the project whose names match `query` (fuzzy), from the declaration index.
    FindDeclarations(String),
    /// Select a result for preview in the panel; the main reading area does not move.
    SelectResult(usize),
    /// Open a result in the main area, after checking it still matches (a committed navigation).
    OpenResult(usize),
    /// Browse a folder, replacing the current root.
    OpenRoot(PathBuf),
    /// Expand or collapse a project-tree directory (its path relative to the root); a folder whose only entry is a
    /// folder opens that one too, down to a folder with more.
    ToggleDir(String),
    /// Activate a current explorer row by stable identity.
    ActivateTree { path: Arc<str>, retain: bool },
    /// Open the tree's folders above the active file, so its row shows.
    Locate,
    /// List each folder's folders before its files (else everything by name); remembered.
    SetFoldersFirst(bool),
    /// Split the file panel (or turn the split; remembered): the active tab shows in the new pane too, which takes
    /// focus.
    Split(Split),
    /// One pane again: the other pane's tabs join the focused one's (a source open in both keeps one tab).
    Unsplit,
    /// Keys and commands go to pane `0` or `1`; each keeps its tabs and what it shows.
    FocusPane(u8),
    /// Move a tab to position `index` among pane `pane`'s tabs (past the end: last). Into the other pane it is shown
    /// there and takes focus; a source that pane has already keeps that pane's tab.
    MoveTab { tab: TabId, pane: u8, index: usize },
    /// Panel sizes chosen in the adapter; remembered.
    SetLayout(Layout),
    /// Explorer presentation recorded by either adapter, using paths rather than row indices.
    SetExplorer(crate::workspace::Explorer),
    /// Checkpoint the current directory and flush its pending metadata off the UI thread.
    SaveWorkspace,
    /// Show the file finder with this query (empty shows it with no results).
    FindFile(String),
    CloseFinder,
    Reload,
    /// Close the active tab.
    Close,
    Select(Selection),
    /// A click's selection at `at`: one click the caret, two the word, three the word's syntax group (Extend
    /// Selection's next step; the line without a grammar), which Extend Selection then goes on from.
    ClickSelect { at: usize, clicks: u8 },
    /// Annotate the active tab with Git blame, or stop.
    SetAnnotate(bool),
    /// Open the Git Branches popup (its lists load in the background), or close it.
    OpenBranches,
    /// The popup as a pick list for Compare with Branch, Tag or Revision….
    OpenRevisions,
    /// Ctrl+click on explorer entry `path`: mark or unmark it for Compare Selected (`selected`, the highlighted entry,
    /// is marked first).
    MarkTreeRow { path: Arc<str>, selected: Option<Arc<str>> },
    ClearTreeMarks,
    /// The popup with only the other worktrees, for Compare with Worktree….
    OpenWorktrees,
    CloseBranches,
    /// Check out a branch, remote branch, tag or revision (local changes go along when git allows).
    Checkout(lenz_content::branches::Target),
    /// The checkout-problem dialog's answer.
    CheckoutChoice(crate::branches::CheckoutChoice),
    /// Make branch `name` at `from` (HEAD when `None`), and check it out when `checkout`.
    NewBranch { name: String, from: Option<String>, checkout: bool },
    /// Open another worktree of the repository as the folder.
    OpenWorktree(PathBuf),
    /// The commit that last changed line `line` of `tab`: the caret goes to the line and its details open.
    ShowCommitAt { tab: TabId, line: usize },
    CloseCommit,
    /// Copy the shown or the caret line's commit id.
    CopyRevision,
    SelectAll,
    Copy,
    /// Copy text the adapter selected (a Markdown preview's rendered text).
    CopyText(String),
    /// Grow the selection to the next enclosing syntax step (words, lines and paragraphs without a grammar).
    ExtendSelection,
    /// Walk Extend Selection back one step.
    ShrinkSelection,
    /// Move the selection's head by `Motion`, keeping its anchor (a caret becomes a selection).
    Extend(Motion),
    /// Move a collapsed text caret; Shift uses Extend instead.
    MoveCaret(Motion),
    /// The first visible line (one inside a collapsed region shows that region's first line).
    Scroll(usize),
    /// Scroll by this many shown rows (collapsed lines are skipped).
    ScrollRows(isize),
    /// Collapse the innermost open fold region holding the caret's line.
    Collapse,
    /// Unfold the collapsed region on the caret's line.
    Expand,
    CollapseAll,
    ExpandAll,
    /// Collapse every run of comments and of imports.
    CollapseCommentsImports,
    /// Fold or unfold the region starting on this line of the active tab (its gutter mark or placeholder).
    ToggleFold(usize),
    Theme(ThemeCommand),
    /// Preview a chooser highlight without selecting or saving; None restores the current theme.
    PreviewTheme(Option<String>),
    /// Follow the definition of the name at the caret: one target opens (a committed navigation), several are
    /// listed as candidates. `peek` shows the target in the results panel without moving.
    Definition { peek: bool },
    /// Go to the implementations of the trait, interface or abstract member at the caret (several are listed).
    Implementation,
    /// Go to the definition of the type of the name at the caret.
    TypeDefinition,
    /// The callers (or callees) of the function at the caret, as a tree in the panel.
    CallHierarchy(Direction),
    /// The subtypes (or supertypes) of the type at the caret, as a tree in the panel.
    TypeHierarchy(Direction),
    /// Show the call hierarchy in the panel the other way, from the same function (else ask at the caret).
    HierarchyDirection(Direction),
    /// Unfold (asking for its level the first time) or fold a call-hierarchy result.
    ToggleCall(usize),
    /// The language server's documentation of the name at the caret, in a popup.
    Documentation,
    /// The documentation of the name at byte `offset` of `tab` (the pointer rests there); the caret stays.
    DocumentationAt { tab: TabId, offset: usize },
    CloseDocumentation,
    /// Whether resting the pointer on a name shows its documentation; remembered.
    SetDocumentationOnHover(bool),
    /// Whether files show their file-type icons (Nerd Font glyphs in a terminal); remembered.
    SetFileIcons(bool),
    /// Copy the active file's path (relative to the open folder), with `:line` of the caret when `line`.
    CopyPath { line: bool },
    /// A message from the adapter (a launch it could not start), shown with the core's own.
    Notice(String),
    /// Mark the occurrences of the name at the caret (asked in the background; never cancels another request).
    HighlightAt,
    /// Stop waiting for a semantic answer; reading continues undisturbed.
    CancelSemantic,
    /// Use this workspace root for the active file's provider under the browsing root (`None` restores the
    /// provider's own discovery); remembered outside the checkout.
    SetWorkspaceRoot(Option<PathBuf>),
    /// List every usage of the name at the caret (semantic references, its declaration excluded).
    Usages,
    /// Workspace symbols matching a query, from the folder's running language servers.
    Symbols(String),
    /// The active document's symbols from its language server (a semantic outline).
    FileSymbols,
    /// Restart the active file's language server (also after it failed or was missing).
    RestartServer,
    /// Turn the active file's language server off (or on) for the browsing root, remembered.
    SetServerEnabled(bool),
    OpenServerRuntime,
    CloseServerRuntime,
    SetServerRuntime(Option<PathBuf>),
    ExcludeRubyTooling,
    SetRubyToolingExclusions(bool),
    ResetProjectServerSettings,
    SetPersistentIndex(bool),
    RebuildIndex,
    /// An SVG: switch between its rendering and its source text.
    ToggleImageSource,
    /// A Markdown file: switch between its preview and its source, at the same place.
    ToggleMarkdownSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchIn {
    Project,
    /// The project's files this pattern picks (see [`search::select_files`]).
    Matching(String),
    ActiveFile,
}

/// How [`Command::Extend`] moves the head: by character, by line (keeping the byte column where the line
/// allows), or to the line's ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
}

/// The user-facing commands, as menus list them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    Open,
    OpenFolder,
    OpenRecent,
    FindFile,
    SearchProject,
    /// Open the find box over the focused tab's text.
    FindInFile,
    NextMatch,
    PreviousMatch,
    CloseFind,
    /// Find options, shared by the find box and project search; remembered while the reader runs.
    MatchCase,
    WholeWord,
    RegularExpression,
    /// The find query's matches listed in the results panel.
    ListMatches,
    /// Go to a line (or line:column) of the active document.
    GoToLine,
    /// Wrap long lines at the pane's width; remembered.
    SoftWrap,
    /// Pin the enclosing declarations at the top of each pane; remembered.
    StickyScroll,
    /// The path and declarations bar above each pane's text; remembered.
    Breadcrumbs,
    /// Give the keys to the focused pane's breadcrumb bar (Ctrl+Shift+.).
    FocusBreadcrumbs,
    Outline,
    FindDeclarations,
    RerunSearch,
    CloseResults,
    Back,
    Forward,
    ToggleBookmark,
    SetBookmarkMnemonic,
    ShowBookmarks,
    NextBookmark,
    PreviousBookmark,
    RecentFiles,
    RecentLocations,
    PinTab,
    NextTab,
    PreviousTab,
    Reload,
    Close,
    SelectAll,
    Copy,
    /// Fold regions: the innermost at the caret, the one on the caret's line, all of them, comments and imports.
    Collapse,
    Expand,
    CollapseAll,
    ExpandAll,
    CollapseCommentsImports,
    ExtendSelection,
    ShrinkSelection,
    ChooseTheme,
    /// The application's name, version, platform, licence and project page (the adapter shows it).
    About,
    /// A separate window with its own session (the adapter creates it).
    NewWindow,
    Definition,
    PeekDefinition,
    TypeDefinition,
    Implementation,
    Usages,
    /// The callers of the function at the caret, as a tree (the panel switches to callees).
    CallHierarchy,
    CallHierarchyCallers,
    CallHierarchyCallees,
    /// The subtypes of the type at the caret, as a tree (the panel switches to supertypes).
    TypeHierarchy,
    TypeHierarchySupertypes,
    TypeHierarchySubtypes,
    /// The language server's documentation of the name at the caret, in a popup.
    Documentation,
    /// Whether resting the pointer on a name shows its documentation; remembered.
    DocumentationOnHover,
    /// File-type icons beside file names; remembered.
    FileIcons,
    /// Inlay hints naming types; remembered.
    InlayTypes,
    /// Inlay hints naming parameters; remembered.
    InlayParameters,
    /// Language-server token colours; remembered.
    SemanticHighlighting,
    /// The active file's path (relative to the open folder), and with the caret's line.
    CopyPath,
    CopyPathLine,
    /// The active file in the user's editor at the caret, or with the system's default app.
    OpenInEditor,
    OpenWithDefaultApp,
    /// The active file, or the explorer's folder or file, against another in Lenz Diff.
    CompareWith,
    /// Lenz Diff on the two entries marked in the explorer.
    CompareSelected,
    Symbols,
    FileSymbols,
    WorkspaceRoot,
    LanguageServer,
    ServerDetails,
    CopyServerDetails,
    ServerRuntime,
    RestartServer,
    ExcludeRubyTooling,
    ResetProjectServerSettings,
    PersistentIndex,
    RebuildIndex,
    DisableServer,
    EnableServer,
    ImageSource,
    MarkdownSource,
    /// Show or hide the explorer (the adapter's panel).
    ToggleExplorer,
    LocateFile,
    /// The tree's order: each folder's folders before its files, or everything by name.
    FoldersFirst,
    AllByName,
    GitBranches,
    NewBranch,
    CheckoutRevision,
    /// Lenz Diff on the working tree as it is against HEAD.
    ShowLocalChanges,
    CompareWithRevision,
    /// Lenz Diff on the working tree against the base branch's tip.
    CompareWithBase,
    /// Lenz Diff on everything since the current branch split from its base.
    ShowBranchChanges,
    CompareWithWorktree,
    ResolveConflicts,
    AnnotateBlame,
    CloseAnnotations,
    ShowCommit,
    ShowCommitDiff,
    CopyRevision,
    SplitRight,
    SplitDown,
    Unsplit,
    FocusOtherPane,
    MoveTabToOtherPane,
    SearchEverywhere,
    FindSymbol,
    FindAction,
    StatusDetails,
}

/// How the file panel is split: side by side, or one pane above the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Split {
    Columns,
    Rows,
}

/// Panel sizes the adapter's dividers were dragged to (`None`: its default), remembered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Layout {
    /// The explorer's width, in pixels.
    pub explorer_width: Option<u32>,
    /// The results panel's share of the height below the toolbar, in thousandths.
    pub results_permille: Option<u32>,
    /// The first pane's share of the split file panel, in thousandths.
    pub split_permille: Option<u32>,
}

impl CommandId {
    pub const ALL: [CommandId; 114] = [
        Self::Open,
        Self::OpenFolder,
        Self::OpenRecent,
        Self::FindFile,
        Self::SearchProject,
        Self::FindInFile,
        Self::NextMatch,
        Self::PreviousMatch,
        Self::CloseFind,
        Self::MatchCase,
        Self::WholeWord,
        Self::RegularExpression,
        Self::ListMatches,
        Self::GoToLine,
        Self::SoftWrap,
        Self::StickyScroll,
        Self::Breadcrumbs,
        Self::FocusBreadcrumbs,
        Self::Outline,
        Self::FindDeclarations,
        Self::RerunSearch,
        Self::CloseResults,
        Self::Back,
        Self::Forward,
        Self::ToggleBookmark,
        Self::SetBookmarkMnemonic,
        Self::ShowBookmarks,
        Self::NextBookmark,
        Self::PreviousBookmark,
        Self::RecentFiles,
        Self::RecentLocations,
        Self::PinTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::Reload,
        Self::Close,
        Self::SelectAll,
        Self::Copy,
        Self::CopyPath,
        Self::CopyPathLine,
        Self::OpenInEditor,
        Self::OpenWithDefaultApp,
        Self::CompareWith,
        Self::CompareSelected,
        Self::Collapse,
        Self::Expand,
        Self::CollapseAll,
        Self::ExpandAll,
        Self::CollapseCommentsImports,
        Self::ExtendSelection,
        Self::ShrinkSelection,
        Self::ChooseTheme,
        Self::About,
        Self::NewWindow,
        Self::Definition,
        Self::PeekDefinition,
        Self::TypeDefinition,
        Self::Implementation,
        Self::Usages,
        Self::CallHierarchy,
        Self::CallHierarchyCallers,
        Self::CallHierarchyCallees,
        Self::TypeHierarchy,
        Self::TypeHierarchySupertypes,
        Self::TypeHierarchySubtypes,
        Self::Documentation,
        Self::DocumentationOnHover,
        Self::FileIcons,
        Self::InlayTypes,
        Self::InlayParameters,
        Self::SemanticHighlighting,
        Self::Symbols,
        Self::FileSymbols,
        Self::WorkspaceRoot,
        Self::LanguageServer,
        Self::ServerDetails,
        Self::CopyServerDetails,
        Self::ServerRuntime,
        Self::RestartServer,
        Self::ExcludeRubyTooling,
        Self::ResetProjectServerSettings,
        Self::PersistentIndex,
        Self::RebuildIndex,
        Self::DisableServer,
        Self::EnableServer,
        Self::ImageSource,
        Self::MarkdownSource,
        Self::ToggleExplorer,
        Self::LocateFile,
        Self::FoldersFirst,
        Self::AllByName,
        Self::GitBranches,
        Self::NewBranch,
        Self::CheckoutRevision,
        Self::ShowLocalChanges,
        Self::CompareWithRevision,
        Self::CompareWithBase,
        Self::ShowBranchChanges,
        Self::CompareWithWorktree,
        Self::ResolveConflicts,
        Self::AnnotateBlame,
        Self::CloseAnnotations,
        Self::ShowCommit,
        Self::ShowCommitDiff,
        Self::CopyRevision,
        Self::SplitRight,
        Self::SplitDown,
        Self::Unsplit,
        Self::FocusOtherPane,
        Self::MoveTabToOtherPane,
        Self::SearchEverywhere,
        Self::FindSymbol,
        Self::FindAction,
        Self::StatusDetails,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SearchEverywhere => "Search Everywhere…",
            Self::FindSymbol => "Find Symbol…",
            Self::FindAction => "Find Action…",
            Self::StatusDetails => "Status details…",
            Self::Open => "Open File…",
            Self::OpenFolder => "Open Folder…",
            Self::OpenRecent => "Open Recent Folder…",
            Self::FindFile => "Find File…",
            Self::SearchProject => "Search in Project…",
            Self::FindInFile => "Find in File…",
            Self::NextMatch => "Next Match",
            Self::PreviousMatch => "Previous Match",
            Self::CloseFind => "Close Find",
            Self::MatchCase => "Find: Match Case",
            Self::WholeWord => "Find: Whole Word",
            Self::RegularExpression => "Find: Regular Expression",
            Self::ListMatches => "List All Matches",
            Self::GoToLine => "Go to Line…",
            Self::SoftWrap => "Soft Wrap",
            Self::StickyScroll => "Sticky Scroll",
            Self::Breadcrumbs => "Breadcrumbs",
            Self::FocusBreadcrumbs => "Focus Breadcrumbs",
            Self::Outline => "File Outline",
            Self::FindDeclarations => "Find Declaration…",
            Self::RerunSearch => "Rerun Search",
            Self::CloseResults => "Close Results",
            Self::Back => "Back",
            Self::Forward => "Forward",
            Self::ToggleBookmark => "Toggle Bookmark",
            Self::SetBookmarkMnemonic => "Set Bookmark Mnemonic…",
            Self::ShowBookmarks => "Show Bookmarks…",
            Self::NextBookmark => "Next Bookmark",
            Self::PreviousBookmark => "Previous Bookmark",
            Self::RecentFiles => "Recent Files…",
            Self::RecentLocations => "Recent Locations…",
            Self::PinTab => "Keep Tab Open",
            Self::NextTab => "Next Tab",
            Self::PreviousTab => "Previous Tab",
            Self::Reload => "Reload",
            Self::Close => "Close Tab",
            Self::SelectAll => "Select All",
            Self::Copy => "Copy",
            Self::Collapse => "Collapse",
            Self::Expand => "Expand",
            Self::CollapseAll => "Collapse All",
            Self::ExpandAll => "Expand All",
            Self::CollapseCommentsImports => "Collapse Comments and Imports",
            Self::ExtendSelection => "Extend Selection",
            Self::ShrinkSelection => "Shrink Selection",
            Self::ChooseTheme => "Change theme",
            Self::About => crate::chrome::ABOUT_TITLE,
            Self::NewWindow => "New Window",
            Self::Definition => "Go to Definition",
            Self::PeekDefinition => "Peek Definition",
            Self::TypeDefinition => "Go to Type Definition",
            Self::Implementation => "Go to Implementation",
            Self::CallHierarchy => "Call Hierarchy",
            Self::CallHierarchyCallers => "Call Hierarchy: Callers",
            Self::CallHierarchyCallees => "Call Hierarchy: Callees",
            Self::TypeHierarchy => "Type Hierarchy",
            Self::TypeHierarchySupertypes => "Type Hierarchy: Supertypes",
            Self::TypeHierarchySubtypes => "Type Hierarchy: Subtypes",
            Self::Documentation => "Quick Documentation",
            Self::DocumentationOnHover => "Documentation on Hover",
            Self::GitBranches => "Git Branches…",
            Self::NewBranch => "New Branch…",
            Self::CheckoutRevision => "Checkout Tag or Revision…",
            Self::ShowLocalChanges => "Show Local Changes",
            Self::CompareWithRevision => "Compare with Branch, Tag or Revision…",
            Self::CompareWithBase => "Compare with Base Branch",
            Self::ShowBranchChanges => "Show Changes Since Branch Start",
            Self::CompareWithWorktree => "Compare with Worktree…",
            Self::ResolveConflicts => "Resolve Conflicts in Lenz Diff",
            Self::AnnotateBlame => "Annotate with Git Blame",
            Self::CloseAnnotations => "Close Annotations",
            Self::ShowCommit => "Show Commit…",
            Self::ShowCommitDiff => "Show Diff",
            Self::CopyRevision => "Copy Revision",
            Self::FileIcons => "File Icons",
            Self::InlayTypes => "Inlay Hints: Types",
            Self::InlayParameters => "Inlay Hints: Parameter Names",
            Self::SemanticHighlighting => "Semantic Highlighting",
            Self::CopyPath => "Copy Path",
            Self::CopyPathLine => "Copy Path:Line",
            Self::OpenInEditor => "Open in Editor",
            Self::OpenWithDefaultApp => "Open with Default App",
            Self::CompareWith => "Compare With…",
            Self::CompareSelected => "Compare Selected",
            Self::WorkspaceRoot => "Set Workspace Root…",
            Self::Usages => "Find Usages",
            Self::Symbols => "Find Symbol (language server)…",
            Self::FileSymbols => "File Symbols (language server)",
            Self::LanguageServer => "Language Server Status…",
            Self::ServerDetails => "Server Details…",
            Self::CopyServerDetails => "Copy Language Server Details",
            Self::ServerRuntime => "Choose Language Server Runtime…",
            Self::RestartServer => "Restart Language Server",
            Self::ExcludeRubyTooling => "Exclude Ruby Tooling Gems for This Project",
            Self::ResetProjectServerSettings => "Reset LSP Settings for This Project",
            Self::PersistentIndex => "Reuse Saved Declaration Index",
            Self::RebuildIndex => "Rebuild Project Index",
            Self::DisableServer => "Disable Language Server for This Folder",
            Self::EnableServer => "Enable Language Server for This Folder",
            Self::ImageSource => "Image Source / Rendered",
            Self::MarkdownSource => "Markdown Source / Preview",
            Self::ToggleExplorer => "Show / Hide Explorer",
            Self::LocateFile => "Select Opened File in Explorer",
            Self::FoldersFirst => "Explorer: Folders First",
            Self::AllByName => "Explorer: All by Name",
            Self::SplitRight => "Split Right",
            Self::SplitDown => "Split Down",
            Self::Unsplit => "Close Split",
            Self::FocusOtherPane => "Focus Other Pane",
            Self::MoveTabToOtherPane => "Move Tab to Other Pane",
        }
    }
}

/// A listed place: what Enter goes to and Delete forgets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Bookmark(PathBuf, usize),
    File(PathBuf),
    Location(PathBuf, usize),
}

/// The lists of places the adapter shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceList {
    Bookmarks,
    RecentFiles,
    RecentLocations,
}

/// A row of a list of places: where it is and its line's text; `missing` when its file was not found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceRow {
    pub place: Place,
    pub label: String,
    pub detail: String,
    pub missing: bool,
}

/// How a Markdown file is shown.
#[derive(Debug)]
pub enum Rendering<'a> {
    Source,
    /// Its preview is being made.
    Pending,
    Preview(&'a Arc<crate::markdown::Preview>),
}

/// A row of the text's context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Command(CommandId),
    Separator,
}

/// A command as a menu shows it: its label and, when it cannot run now, why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: CommandId,
    pub label: &'static str,
    pub disabled: Option<&'static str>,
    /// A setting's state (`None`: not a setting).
    pub on: Option<bool>,
}

/// Results of background work, applied through [`Session::dispatch`].
pub enum Completion {
    Projected(crate::projection::Finished),
    LaidOut(Box<crate::layout::Finished>),
    LibraryRead { generation: u64, read: crate::libraries::Read },
    Navigation { request: u64, files: usize, indexed: usize, found: Option<(Vec<crate::navigation::Row>, usize)> },
    ServerRuntimes { request: u64, paths: Vec<PathBuf> },
    ServerRuntimeSaved { request: u64, kind: semantics::runtime::Kind, result: Result<Option<PathBuf>, String> },
    ProviderSettingsSaved { ok: bool },
    ProjectProviderSaved { provider: usize, root: PathBuf, reset: bool, result: Result<(), String> },
    /// The project's saved places read from `from` (or why not), for saving to `file` (a different file when `from`
    /// is its name before names were canonical).
    PlacesRead { generation: u64, file: PathBuf, from: PathBuf, result: Result<crate::places::Places, crate::places::ReadError> },
    /// A save of sequence `seq` was written merged with another window's changes: the places written.
    PlacesMerged { generation: u64, seq: u64, places: crate::places::Places },
    /// Which listed files no longer exist, checked for the project of generation `generation`.
    PlacesChecked { generation: u64, missing: std::collections::HashSet<PathBuf> },
    /// A kept snapshot's file was checked: reuse it if `stamp` still matches.
    Checked { request: u64, tab: TabId, path: PathBuf, stamp: Option<Stamp> },
    /// A read and decoded source, with its resolved identity where the platform has one.
    Loaded { request: u64, tab: TabId, path: PathBuf, result: Result<Loaded, String> },
    /// A version's syntax spans, fold regions and (while asked for) declarations, from one parse.
    Highlighted { version: u64, parsed: Option<Parsed>, folds: Option<Folds>, declarations: Option<highlight::Declarations> },
    /// A version's decoded inlay hints or semantic tokens, made under its kind's setting generation `generation`.
    Decorated { version: u64, generation: u64, request: i64, data: crate::decor::Decorated },
    /// A tab's find matches for one request and document version (or why the query cannot be matched).
    Matched { tab: TabId, request: u64, version: u64, result: crate::find::Found },
    /// The syntax steps a selection grows through (`None`: no tree; words, lines and paragraphs instead).
    SelectionSteps { request: u64, steps: Option<Vec<std::ops::Range<usize>>> },
    /// A Markdown version's preview (or why it cannot be shown), then its local images.
    Rendered { version: u64, preview: Result<crate::markdown::Preview, String> },
    PreviewImages { version: u64, doc: Arc<lenz_content::markdown::MarkdownDocument>, images: crate::markdown::Images },
    /// A batch of the root's files; `done` ends the walk.
    Scanned { generation: u64, batch: project::Batch },
    /// Finder results for query `seq`, ranked over `searched` files: the best hits and how many matched (`None`:
    /// superseded before it finished).
    Ranked { generation: u64, seq: u64, searched: usize, hits: Option<(Vec<Arc<str>>, usize)> },
    Theme(Box<ThemeUpdate>),
    /// Filesystem changes, coalesced.
    Changed(Change),
    /// A tab's source was checked against the disk.
    Refreshed { tab: TabId, token: u64, result: Refresh },
    /// Results for search `id`.
    Found { id: u64, found: search::Found },
    /// Line colours for results of search `id`.
    ResultSpans { id: u64, spans: Vec<(usize, search::LineSpans)> },
    /// Syntax colours of the previewed snapshot `version`.
    PreviewHighlighted { request: u64, version: u64, parsed: Option<Parsed> },
    /// The document for previewing result `index` of search `id`.
    Previewed { id: u64, index: usize, doc: Result<(Stamp, Arc<Document>), String> },
    /// Result `index` of search `id`, for open request `open`, checked against the file as it is now: its current
    /// snapshot (and stamp, when read from disk), or why not.
    Validated { id: u64, open: u64, index: usize, result: Result<(Option<Stamp>, Arc<Document>), String> },
    /// Declaration records for files of root generation `generation`.
    Indexed { generation: u64, revision: u64, files: Vec<(Arc<str>, FileIndex)> },
    IndexReset { generation: u64, revision: u64, result: Result<(), String> },
    /// A tab's source was renamed, confirmed by file identity.
    Moved { tab: TabId, token: u64, to: PathBuf, stamp: Option<Stamp>, canonical: Option<PathBuf> },
    /// A semantic provider's event.
    Semantic(semantics::Event),
    /// The folders to watch for this inventory and these tabs, built off the UI thread.
    Watch { key: WatchKey, folders: Arc<std::collections::BTreeSet<PathBuf>> },
    /// The text of a virtual source, for request `request`.
    Retrieved { request: u64, target: semantics::VirtualTarget, doc: Arc<Document> },
    /// The located targets of definition request `request`.
    Targets { request: u64, session: u64, provider: &'static str, origin: String, indexing: bool, targets: semantics::Targets },
    /// Documentation made into a preview (or why it could not be).
    DocumentationReady { request: u64, preview: Result<crate::markdown::Preview, String> },
    /// A call-hierarchy level, located: each function with its call site.
    /// `limited`: the usages it was read from stopped at the limit (a syntax-built level may miss children).
    /// `indexing`: the server was still loading or indexing when it answered.
    HierarchyLevel { request: u64, level: Vec<(Call, semantics::Target)>, limited: bool, indexing: bool },
    /// The kind of results `id`'s hierarchy root, read from its file's declarations.
    HierarchyRootKind { id: u64, kind: highlight::DeclKind },
    /// A checkout (or new branch, named `label`; `made` when it was only created) in the repository at `root` ended.
    CheckedOut { request: u64, root: PathBuf, label: String, made: bool, result: Result<lenz_content::branches::Outcome, lenz_content::branches::CheckoutError> },
    /// HEAD of the repository at `root`, read again because its signal changed.
    Head { root: PathBuf, head: Option<lenz_content::branches::Head> },
    /// The repository holding `anchor` (or why there is none).
    Repo { anchor: PathBuf, result: Result<crate::branches::Repo, String> },
    /// The Git Branches popup's lists for the repository at `root`.
    Branches { request: u64, root: PathBuf, result: Result<(lenz_content::branches::Refs, Vec<lenz_content::branches::Worktree>), String> },
    /// The details of the commit popup's commit, for request `request`.
    CommitLoaded { request: u64, result: Result<lenz_content::blame::CommitDetails, String> },
    /// Blame of `tab`'s request `request`: the repository root first, then runs of lines, then `done`.
    Blamed { tab: TabId, request: u64, root: Option<PathBuf>, hunks: Vec<(std::ops::Range<usize>, Arc<lenz_content::blame::Commit>)>, done: Option<Result<(), String>> },
}

/// A loaded source: its stamp, the decoded snapshot and its resolved path.
pub type Loaded = (Option<Stamp>, Arc<Document>, Option<PathBuf>);

/// What a refresh found.
pub enum Refresh {
    /// The same file and content: only its stamp is updated.
    Same(Stamp),
    /// New content, as a new version.
    Changed(Stamp, Arc<Document>),
    /// Changed on disk while the tab's content is released: reloaded on activation.
    Released,
    /// The file is gone (or unreadable): the tab keeps its last content.
    Gone(String),
    /// The file kept changing while it was read: try again shortly.
    Unstable,
}

pub enum Input {
    Command(Command),
    Completion(Completion),
}

impl Input {
    /// What it is, for diagnostics: a command's or a completion's kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Input::Command(_) => "command",
            Input::Completion(c) => match c {
                Completion::Projected(_) => "source projection",
                Completion::LaidOut(_) => "source layout",
                Completion::PlacesRead { .. } => "places read",
                Completion::PlacesMerged { .. } => "places merged",
                Completion::PlacesChecked { .. } => "places checked",
                Completion::Checked { .. } => "checked",
                Completion::Loaded { .. } => "loaded",
                Completion::Highlighted { .. } => "highlighted",
                Completion::Decorated { .. } => "decorated",
                Completion::Matched { .. } => "matched",
                Completion::SelectionSteps { .. } => "selection steps",
                Completion::Rendered { .. } => "rendered",
                Completion::PreviewImages { .. } => "preview images",
                Completion::Scanned { .. } => "scanned",
                Completion::LibraryRead { .. } => "library read",
                Completion::Ranked { .. } => "ranked",
                Completion::Navigation { .. } => "navigation",
                Completion::Theme(_) => "theme",
                Completion::Changed(_) => "changed",
                Completion::Refreshed { .. } => "refreshed",
                Completion::Found { .. } => "found",
                Completion::Previewed { .. } => "previewed",
                Completion::PreviewHighlighted { .. } => "preview highlighted",
                Completion::ResultSpans { .. } => "result spans",
                Completion::Validated { .. } => "validated",
                Completion::Indexed { .. } => "indexed",
                Completion::IndexReset { .. } => "index reset",
                Completion::Moved { .. } => "moved",
                Completion::Semantic(_) => "semantic",
                Completion::ServerRuntimes { .. } => "Server runtimes",
                Completion::ServerRuntimeSaved { .. } => "Server runtime saved",
                Completion::ProviderSettingsSaved { .. } => "provider settings saved",
                Completion::ProjectProviderSaved { .. } => "project provider settings saved",
                Completion::Watch { .. } => "watch",
                Completion::Retrieved { .. } => "retrieved",
                Completion::Targets { .. } => "targets",
                Completion::HierarchyLevel { .. } => "hierarchy level",
                Completion::HierarchyRootKind { .. } => "hierarchy root kind",
                Completion::Blamed { .. } => "blamed",
                Completion::CommitLoaded { .. } => "commit loaded",
                Completion::Repo { .. } => "repository",
                Completion::CheckedOut { .. } => "checked out",
                Completion::Head { .. } => "head",
                Completion::Branches { .. } => "branches",
                Completion::DocumentationReady { .. } => "documentation",
            },
        }
    }
}

impl From<Command> for Input {
    fn from(c: Command) -> Self {
        Input::Command(c)
    }
}

impl From<Completion> for Input {
    fn from(c: Completion) -> Self {
        Input::Completion(c)
    }
}

/// What the adapter does after a transition.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Effect {
    pub repaint: bool,
    /// Text for the clipboard.
    pub copy: Option<String>,
}

/// The state adapters present.
pub struct State {
    sources: Sources,
    tabs: Vec<Tab>,
    /// The focused pane's shown tab; commands act on it.
    active: Option<TabId>,
    /// While split: the other pane's shown tab, kept in memory and coloured like the active one.
    other: Option<TabId>,
    /// The focused pane (0, or 1 while split).
    focused: u8,
    history: History,
    loading: Option<PathBuf>,
    /// Messages about failed actions and settings, newest last.
    messages: Vec<String>,
    palette: Arc<Palette>,
    project: Option<Project>,
    libraries: crate::libraries::Libraries,
    tree_revision: u64,
    explorer: crate::workspace::Explorer,
    workspace_revision: u64,
    recent: Vec<PathBuf>,
    /// The project's bookmarks, recent files and locations (the session's own without a project).
    places: Arc<crate::places::Places>,
    /// Listed files found missing when a list last opened.
    missing: std::collections::HashSet<PathBuf>,
    settings: Settings,
    /// Each pane's text area, in columns and rows, as the adapter last reported it (0: not yet).
    text_size: [(usize, usize); 2],
    results: Option<Results>,
    navigation: Option<crate::navigation::Search>,
    index: DeclIndex,
    semantics: Semantics,
    server_runtime: Option<semantics::runtime::Runtime>,
    runtime_saving: Option<u64>,
    provider_saves: usize,
    /// The semantic request being answered.
    asking: Option<Asking>,
    /// The occurrences of the name last asked about.
    occurrences: Option<Occurrences>,
    /// The documentation shown in a popup.
    documentation: Option<Documentation>,
    /// The commit an annotation opened, while shown.
    commit: Option<crate::blame::CommitPopup>,
    /// The repository of what is shown, the path it was looked up for, and the last path answered.
    pub(crate) repo: Option<crate::branches::Repo>,
    pub(crate) repo_anchor: Option<PathBuf>,
    pub(crate) repo_answered: Option<PathBuf>,
    /// The Git Branches popup's lists, while it is open.
    pub(crate) branches: Option<crate::branches::BranchesView>,
    /// Explorer entries marked for Compare Selected (Ctrl+click), oldest first; at most two.
    pub(crate) tree_marks: Vec<Arc<str>>,
    /// The running checkout's request, the refused one's dialog, and the files a Smart Checkout left in conflict.
    pub(crate) checkout_running: Option<u64>,
    pub(crate) checkout_problem: Option<crate::branches::CheckoutProblem>,
    pub(crate) stash_conflicts: Option<Vec<String>>,
}

/// What asked for documentation: a key or menu (about the caret), or the pointer resting on a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocTrigger {
    Caret,
    Pointer,
}

/// A language server's documentation of a name in a tab's document version, ready to show.
#[derive(Clone, Debug)]
pub struct Documentation {
    pub tab: TabId,
    pub version: u64,
    /// The name, as a range of the document.
    pub at: std::ops::Range<usize>,
    pub trigger: DocTrigger,
    pub preview: Arc<crate::markdown::Preview>,
}

/// The occurrences in a document version of the name at a caret: each range, and whether it is written there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrences {
    pub path: PathBuf,
    pub version: u64,
    pub ranges: Vec<(std::ops::Range<usize>, bool)>,
}

/// How long the caret rests on a name before its occurrences are asked for (the adapter's timer).
pub const HIGHLIGHT_REST: std::time::Duration = std::time::Duration::from_millis(250);

/// A semantic request in flight: what was asked, about which snapshot (or query), in which project, and how
/// many servers still owe an answer.
struct Asking {
    request: u64,
    ask: Ask,
    /// The document, offset and version asked about (not for symbols).
    at: Option<(PathBuf, usize, u64)>,
    project: u64,
    remaining: usize,
    /// When it was asked (the session's clock).
    since: std::time::Instant,
    /// The virtual target whose text is being retrieved (after a definition named one).
    retrieving: Option<semantics::VirtualTarget>,
    /// A call hierarchy's root, while its place is read.
    root: Option<Call>,
}

/// What the user asked a language server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Definition,
    Peek,
    Implementation,
    TypeDefinition,
    CallHierarchy(Direction),
    TypeHierarchy(Direction),
    Usages,
    Symbols,
    FileSymbols,
}

impl Ask {
    fn question(self) -> semantics::Question {
        match self {
            Ask::Definition | Ask::Peek => semantics::Question::Definition,
            Ask::Implementation => semantics::Question::Implementation,
            Ask::TypeDefinition => semantics::Question::TypeDefinition,
            Ask::CallHierarchy(_) => semantics::Question::PrepareCallHierarchy,
            Ask::TypeHierarchy(_) => semantics::Question::PrepareTypeHierarchy,
            Ask::Usages => semantics::Question::Usages,
            Ask::Symbols => semantics::Question::Symbols,
            Ask::FileSymbols => semantics::Question::FileSymbols,
        }
    }

    /// The prefix of its messages.
    fn label(self) -> &'static str {
        match self {
            Ask::Definition | Ask::Peek => "Definition",
            Ask::Implementation => "Implementations",
            Ask::TypeDefinition => "Type definitions",
            Ask::CallHierarchy(_) => "Call hierarchy",
            Ask::TypeHierarchy(_) => "Type hierarchy",
            Ask::Usages => "Usages",
            Ask::Symbols => "Symbols",
            Ask::FileSymbols => "File symbols",
        }
    }

    /// The question that produced results of `kind`, if a language server answered it.
    fn of(kind: ResultsKind) -> Option<Ask> {
        [Ask::Definition, Ask::Peek, Ask::Implementation, Ask::TypeDefinition, Ask::CallHierarchy(Direction::Callers), Ask::TypeHierarchy(Direction::Subtypes), Ask::Usages, Ask::Symbols, Ask::FileSymbols].into_iter().find(|a| a.results() == kind)
    }

    fn results(self) -> ResultsKind {
        match self {
            Ask::Definition => ResultsKind::Definitions,
            Ask::Peek => ResultsKind::Peek,
            Ask::Implementation => ResultsKind::Implementations,
            Ask::TypeDefinition => ResultsKind::TypeDefinitions,
            Ask::CallHierarchy(_) | Ask::TypeHierarchy(_) => ResultsKind::Hierarchy,
            Ask::Usages => ResultsKind::Usages,
            Ask::Symbols => ResultsKind::Symbols,
            Ask::FileSymbols => ResultsKind::FileSymbols,
        }
    }
}

/// Session-owned identities outlive release of reloadable content. Only the registry owns current
/// text; tabs retain identity and reading positions, and the bounded cache keeps clean text reusable.
#[derive(Default)]
struct Sources {
    next: u64,
    aliases: std::collections::HashMap<PathBuf, crate::text::BufferId>,
    buffers: std::collections::HashMap<crate::text::BufferId, CurrentSource>,
}
struct CurrentSource {
    document: std::sync::Weak<Document>,
    snapshot: Option<Arc<crate::text::TextSnapshot>>,
    stamp: Option<Stamp>,
}
impl Sources {
    fn admit(&mut self, source: &Path, previous: Option<crate::text::BufferId>, doc: &Arc<Document>, stamp: Option<Stamp>) -> crate::text::BufferId {
        let id = *self.aliases.entry(source.to_path_buf()).or_insert_with(|| previous.unwrap_or_else(|| {
            self.next += 1;
            crate::text::BufferId(self.next)
        }));
        let entry = self.buffers.entry(id).or_insert_with(|| CurrentSource { document: Default::default(), snapshot: None, stamp });
        if entry.snapshot.as_ref().is_some_and(|snapshot| snapshot.key().version >= doc.version()) {
            return id;
        }
        entry.snapshot = Some(Arc::new(crate::text::TextSnapshot::from_document(id, doc)));
        entry.document = Arc::downgrade(doc);
        entry.stamp = stamp;
        id
    }
}

/// Current source text paired with one tab's independent reading state.
pub struct View<'a> {
    tab: &'a Tab,
    snapshot: &'a Arc<crate::text::TextSnapshot>,
}
impl<'a> View<'a> {
    pub fn tab(&self) -> &'a Tab { self.tab }
    pub fn snapshot(&self) -> &'a Arc<crate::text::TextSnapshot> { self.snapshot }
    pub fn dirty(&self) -> bool { false }
    pub fn edit_disabled(&self) -> Option<&str> { Some("Editing is not available yet") }
    /// Reader source rows include the logical final empty line.
    pub fn line_count(&self) -> usize { self.snapshot.line_count() }
    pub fn visible(&self) -> Arc<crate::folds::Visible> { self.tab.visible_for(self.snapshot.line_count()) }
    /// Compatibility layout over the retained immutable Document; use line_window for bounded reads.
    pub fn shown_line(&self, line: usize) -> crate::shown::Shown { self.tab.shown_line(line) }
    /// Compatibility layout until both adapters consume bounded snapshot windows.
    pub fn line_rows(&self, line: usize, cols: Option<usize>) -> Arc<[crate::wrap::Row]> { self.tab.line_rows(line, cols) }
    /// Compatibility hit testing for the existing whole-line display map.
    pub fn caret_row(&self, line: usize, byte: usize, cols: Option<usize>) -> usize { self.tab.caret_row(line, byte, cols) }
    /// Selection coordinates can be queried without allocating its text.
    pub fn selected_range(&self) -> Option<std::ops::Range<usize>> {
        let range = self.tab.selection()?.range();
        (!range.is_empty() && range.end <= self.snapshot.len_bytes()
            && self.snapshot.floor_char_boundary(range.start) == range.start
            && self.snapshot.floor_char_boundary(range.end) == range.end).then_some(range)
    }
    pub fn line_window(&self, line: usize, from: usize, max_bytes: usize) -> Option<crate::text::TextWindow<'_>> {
        self.snapshot.line_window(line, from, max_bytes)
    }
    pub fn copy_selection(&self) -> Option<String> { self.snapshot.copy_range(self.selected_range()?) }

}

impl View<'_> {
    /// A line-relative hover identifier; names clipped by the 4 KiB inspection window are omitted.
    pub fn linkable(&self, line: usize, byte: usize) -> Option<std::ops::Range<usize>> {
        let source = self.snapshot.line_range(line)?;
        let at = source.start.checked_add(byte)?;
        if at >= source.end || self.snapshot.floor_char_boundary(at) != at { return None; }
        let from = self.snapshot.floor_char_boundary(at.saturating_sub(2048).max(source.start));
        let end = self.snapshot.floor_char_boundary(from.saturating_add(4096).min(source.end));
        let text = self.snapshot.copy_range(from..end)?;
        let word = crate::document::identifier(&text, at - from)?;
        if (word.start == 0 && from > source.start) || (word.end == text.len() && end < source.end) { return None; }
        let word = from + word.start - source.start..from + word.end - source.start;
        use lenz_content::highlight::Capture as C;
        let spans = self.tab.highlights().map(|p| p.spans(line..line + 1)).unwrap_or_default();
        let quiet = spans.first().is_some_and(|s| s.iter().any(|(r, c)| r.contains(&word.start)
            && matches!(c, C::Keyword | C::String | C::Escape | C::Comment | C::DocComment | C::Number | C::ConstantBuiltin)));
        (!quiet).then_some(word)
    }
}

impl View<'_> {
    fn layout_query<T>(&self, cols: Option<usize>, mut query: impl FnMut(&mut crate::layout::Cache) -> crate::layout::Query<T>) -> crate::layout::Query<T> {
        use crate::layout::Query;
        let mut cache = self.tab.layout.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.configure(self.snapshot, cols, self.inlay_hints());
        let mut budget = crate::projection::WINDOW_WORK;
        for _ in 0..64 {
            match query(&mut cache) {
                Query::Pending(work) => {
                    if budget < crate::projection::MIN_WORK { return Query::Pending(work); }
                    let Some(delta) = work.clone().run(self.snapshot, budget, || true) else { return Query::Pending(work) };
                    budget = budget.saturating_sub(delta.work().max(crate::projection::MIN_WORK));
                    cache.apply(delta);
                }
                ready => return ready,
            }
        }
        query(&mut cache)
    }

    pub fn layout_row(&self, line: usize, index: usize, cols: Option<usize>) -> crate::layout::Query<Option<crate::layout::Row>> {
        self.layout_query(cols, |cache| cache.row(self.snapshot, line, index, 32))
    }

    pub fn layout_last(&self, line: usize, cols: Option<usize>) -> crate::layout::Query<Option<crate::layout::Row>> {
        self.layout_query(cols, |cache| cache.last(self.snapshot, line))
    }

    pub fn layout_position(&self, source: usize, cols: Option<usize>) -> crate::layout::Query<Option<crate::layout::Position>> {
        self.layout_query(cols, |cache| cache.position(self.snapshot, self.snapshot.line_of(source), source))
    }

    /// A projection cursor at or before the global display byte, within one bounded raw window.
    pub fn layout_cursor(&self, line: usize, display: usize, cols: Option<usize>) -> crate::layout::Query<Option<crate::projection::Cursor>> {
        self.layout_query(cols, |cache| cache.cursor(self.snapshot, line, display))
    }

    pub fn layout_byte_at(&self, row: &crate::layout::Row, column: usize, cols: Option<usize>) -> crate::layout::Query<Option<usize>> {
        self.layout_query(cols, |cache| cache.byte_at(self.snapshot, row, column))
    }

    pub fn layout_rows(&self, count: usize, cols: Option<usize>) -> crate::layout::Rows {
        use crate::layout::{Query, Rows};
        let visible = self.visible();
        let count = count.min(4096);
        let mut out = Vec::with_capacity(count);
        let query = self.layout_query(cols, |cache| {
            out.clear();
            let (mut line, mut index) = (self.tab.top_line(), if cols.is_some() { self.tab.top_row() } else { 0 });
            while out.len() < count {
                match cache.row(self.snapshot, line, index, count - out.len()) {
                    Query::Ready(Some(row)) => {
                        let last = row.last;
                        out.push(row);
                        if !last { index += 1; continue; }
                    }
                    Query::Ready(None) => {}
                    Query::Pending(work) => {
                        if let Some(prefix) = cache.prefix(self.snapshot, line, index) { out.push(prefix); }
                        return Query::Pending(work);
                    }
                }
                let next = visible.step(line, 1);
                if next == line { break; }
                line = next; index = 0;
            }
            Query::Ready(())
        });
        Rows { rows: out, pending: match query { Query::Pending(work) => Some(work), Query::Ready(()) => None } }
    }

    pub fn layout_sticky(&self, max: usize, cols: Option<usize>) -> crate::layout::Query<Arc<[crate::tabs::StickyLine]>> {
        use crate::layout::Query;
        let Some(declarations) = self.tab.declarations.as_ref().filter(|_| max > 0) else { return Query::Ready(Arc::from([])) };
        let shown = self.layout_rows(max.saturating_add(1), cols);
        if let Some(work) = shown.pending { return Query::Pending(work); }
        let row_of = |line: u32| shown.rows.iter().position(|row| row.line as u32 >= line).map_or(u32::MAX, |at| at as u32);
        Query::Ready(declarations.sticky_rows(u32::try_from(self.tab.top_line()).unwrap_or(u32::MAX), self.tab.top_row() > 0, max, row_of)
            .into_iter().map(|line| crate::tabs::StickyLine { line: line.line as usize, row: line.row as usize })
            .filter(|line| line.line < self.line_count()).collect())
    }

    pub fn layout_step(&self, from: (usize, usize), delta: isize, cols: usize) -> crate::layout::Query<(usize, usize)> {
        let visible = self.visible();
        self.layout_query(Some(cols), |cache| cache.step(self.snapshot, &visible, from, delta))
    }

    pub fn inlay_hints(&self) -> Option<&Arc<crate::decor::Hints>> {
        let (version, hints) = self.tab.hints.as_ref()?;
        (*version == self.snapshot.key().version).then_some(hints)
    }

    pub fn display_start(&self, line: usize) -> Option<crate::projection::Cursor> {
        crate::projection::Cursor::start(self.snapshot, line, self.inlay_hints().cloned())
    }

    pub fn display_window(&self, cursor: &mut crate::projection::Cursor, work_bytes: usize) -> Option<crate::projection::Window> {
        if !cursor.matches(self.snapshot, self.inlay_hints()) { return None; }
        cursor.window(self.snapshot, work_bytes)
    }
}

impl State {
    pub fn view(&self, id: TabId) -> Option<View<'_>> {
        let tab = self.tabs.iter().find(|tab| tab.id == id)?;
        let buffer = tab.buffer?;
        let snapshot = self.sources.buffers.get(&buffer)?.snapshot.as_ref()?;
        tab.doc.as_ref()?;
        Some(View { snapshot, tab })
    }

    fn admit_current(&mut self, id: TabId) -> Vec<TabId> {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else { return Vec::new() };
        let Some(doc) = &tab.doc else { return Vec::new() };
        let buffer = self.sources.admit(tab.canonical.as_deref().unwrap_or(&tab.source), tab.buffer, doc, tab.stamp);
        tab.buffer = Some(buffer);
        let Some(entry) = self.sources.buffers.get(&buffer) else { return Vec::new() };
        let Some(current) = entry.document.upgrade() else { return Vec::new() };
        let mut changed = Vec::new();
        for tab in &mut self.tabs {
            if tab.buffer != Some(buffer) { continue; }
            let Some(previous) = &tab.doc else { continue };
            if Arc::ptr_eq(previous, &current) { continue; }
            let at = tab.location();
            tab.doc = Some(current.clone());
            tab.stamp = entry.stamp;
            tab.new_content();
            tab.restore_to(&at);
            changed.push(tab.id);
        }
        changed
    }

    fn current(&self) -> Option<Arc<crate::text::TextSnapshot>> {
        Some(self.view(self.active?)?.snapshot().clone())
    }

    pub fn navigation(&self) -> Option<&crate::navigation::Search> { self.navigation.as_ref() }
    /// How much of the project the declaration index covers.
    pub fn index_coverage(&self) -> index::Coverage {
        self.index.coverage()
    }
    /// The bottom panel's results, when a search ran.
    pub fn results(&self) -> Option<&Results> {
        self.results.as_ref()
    }
    /// The browsing root, its discovery coverage, tree and finder.
    pub fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }
    /// Recently opened roots, most recent first.
    /// The project's bookmarks, recent files and recent locations.
    pub fn places(&self) -> &crate::places::Places {
        &self.places
    }
    /// List `list`'s places matching `query` (fuzzy on the path, or a plain match in the line's text), in the list's
    /// order. Recent files lead with the file left last, the open one second.
    pub fn place_rows(&self, list: PlaceList, query: &str) -> Vec<PlaceRow> {
        let rel = |p: &Path| self.project.as_ref().and_then(|pr| pr.relative(p)).map_or_else(|| p.display().to_string(), |r| r.to_string());
        let mark = |m: &crate::places::Mark, place: Place| {
            let key = m.mnemonic.map_or_else(|| "·".to_string(), String::from);
            let label = match list {
                PlaceList::Bookmarks => format!("{key}  {}:{}", rel(&m.source), m.line + 1),
                _ => format!("{}:{}", rel(&m.source), m.line + 1),
            };
            let moved = if m.moved { "  (moved?)" } else { "" };
            PlaceRow { place, label, detail: format!("{}{moved}", m.anchor.trim()), missing: self.missing.contains(&*m.source) }
        };
        let rows: Vec<PlaceRow> = match list {
            PlaceList::Bookmarks => self.places.bookmarks().iter().map(|m| mark(m, Place::Bookmark(m.source.to_path_buf(), m.line))).collect(),
            PlaceList::RecentLocations => self.places.recent_locations().iter().map(|m| mark(m, Place::Location(m.source.to_path_buf(), m.line))).collect(),
            PlaceList::RecentFiles => {
                let current = self.active().filter(|t| t.retrieved.is_none()).map(|t| t.source.clone());
                let mut files: Vec<PathBuf> = self.places.recent_files().iter().filter(|p| Some(*p) != current.as_ref()).cloned().collect();
                if let Some(c) = current {
                    files.insert(files.len().min(1), c);
                }
                files
                    .into_iter()
                    .map(|p| {
                        let r = rel(&p);
                        let (dir, name) = r.rsplit_once('/').map_or((String::new(), r.clone()), |(d, n)| (d.to_string(), n.to_string()));
                        PlaceRow { missing: self.missing.contains(&p), place: Place::File(p), label: name, detail: dir }
                    })
                    .collect()
            }
        };
        if query.is_empty() {
            return rows;
        }
        let q = query.to_lowercase();
        rows.into_iter().filter(|r| lenz_content::fuzzy::score(query, &format!("{}/{}", r.detail, r.label)).is_some() || r.label.to_lowercase().contains(&q) || r.detail.to_lowercase().contains(&q)).collect()
    }
    pub fn recent(&self) -> &[PathBuf] {
        &self.recent
    }
    /// The tree lists each folder's folders before its files.
    pub fn folders_first(&self) -> bool {
        self.settings.folders_first
    }
    /// Cached project and library rows, with stable identities shared by both explorers.
    pub fn tree_rows(&self) -> impl DoubleEndedIterator<Item = &project::TreeRow> + Clone {
        self.project.as_ref().map_or(&[][..], |p| p.rows()).iter().chain(self.libraries.rows())
    }
    pub fn has_tree(&self) -> bool { self.project.is_some() || !self.libraries.rows().is_empty() }
    /// Changes when navigation asks the adapters to reveal the active library file.
    pub fn tree_revision(&self) -> u64 { self.tree_revision }
    pub fn active_in_tree(&self) -> Option<Arc<str>> {
        let t = self.active()?;
        if self.libraries.contains(&t.source) { return Some(t.source.to_string_lossy().as_ref().into()); }
        let p = self.project.as_ref()?;
        p.relative(&t.source).filter(|rel| p.has_file(rel))
    }
    /// Every tab, of both panes, in order.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }
    /// Pane `pane`'s tabs, in order.
    pub fn pane_tabs(&self, pane: u8) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(move |t| t.pane == pane)
    }
    /// The focused pane's shown tab.
    pub fn active(&self) -> Option<&Tab> {
        let id = self.active?;
        self.tabs.iter().find(|t| t.id == id)
    }
    /// The tab pane `pane` shows.
    pub fn shown(&self, pane: u8) -> Option<&Tab> {
        let id = if pane == self.focused { self.active } else { self.other }?;
        self.tabs.iter().find(|t| t.id == id)
    }
    pub fn focused_pane(&self) -> u8 {
        self.focused
    }
    /// How the file panel is split, if it is.
    pub fn split(&self) -> Option<Split> {
        self.settings.split
    }
    pub fn explorer(&self) -> &crate::workspace::Explorer { &self.explorer }
    pub fn workspace_revision(&self) -> u64 { self.workspace_revision }

    pub fn layout(&self) -> Layout {
        self.settings.layout
    }
    /// The active tab's document.
    pub fn document(&self) -> Option<&Arc<Document>> {
        self.active()?.document()
    }
    /// Syntax spans for the active document's version, when ready.
    pub fn highlights(&self) -> Option<&Parsed> {
        self.active()?.highlights()
    }
    /// How the active Markdown file is shown (`None`: not Markdown). A preview that cannot be made shows the
    /// source, and a notice says why.
    pub fn rendering(&self) -> Option<Rendering<'_>> {
        self.active()?.rendering()
    }
    pub fn loading(&self) -> Option<&Path> {
        self.loading.as_deref()
    }
    /// The active tab's selected source text (`None` for a caret or no selection).
    pub fn selected_text(&self) -> Option<&str> {
        let (doc, s) = (self.document()?, self.selection()?);
        doc.text().get(s.range()).filter(|t| !t.is_empty())
    }

    pub fn selection(&self) -> Option<Selection> {
        self.active()?.selection
    }
    pub fn top_line(&self) -> usize {
        self.active().map_or(0, |t| t.top_line)
    }
    pub fn column(&self) -> usize {
        self.active().map_or(0, |t| t.column)
    }
    pub fn can_back(&self) -> bool {
        self.history.can_back()
    }
    pub fn can_forward(&self) -> bool {
        self.history.can_forward()
    }
    /// What to say now: the active document's encoding, loss and large-file notices, its availability, then
    /// messages.
    pub fn notices(&self) -> Vec<String> {
        let tab = self.active();
        let doc = self.current().map(|snapshot| snapshot.notices()).unwrap_or_default();
        let gone = tab.and_then(|t| Some((t.unavailable.as_ref()?, t.doc.is_some()))).map(|(e, shown)| match shown {
            true => format!("Unavailable: {e}; showing the last loaded content"),
            false => format!("Unavailable: {e}"),
        });
        let approximate = tab.filter(|t| t.approximate).map(|_| "Approximate position: the surrounding text changed".to_string());
        let changing = tab.filter(|t| t.changing > 0).map(|_| "Changing on disk: showing the last confirmed content".to_string());
        // A file outside the browsing root (a definition in a dependency): say where it comes from.
        // Source a language server returned: from where, and whether that server session still runs.
        let retrieved = tab.and_then(|t| t.retrieved.as_ref()).map(|v| match self.semantics.has_session(v.session) {
            true => format!("Read-only source retrieved from {} ({})", v.provider, v.uri),
            false => format!("Read-only source retrieved from {}; its server session has ended ({})", v.provider, v.uri),
        });
        // Outside the browsing root, or a dependency inside it (node_modules, a project's virtualenv): its origin.
        let origin = tab.filter(|t| t.retrieved.is_none()).zip(self.project.as_ref()).and_then(|(t, p)| {
            let language = t.doc.as_ref().map_or(t.language.flatten(), |d| d.language());
            let origin = semantics::provider::origin(&t.source, language);
            let inside = t.source.starts_with(p.root());
            (!inside || origin != semantics::provider::OUTSIDE).then(|| format!("Read-only source from {origin}"))
        });
        let preview = tab.and_then(|t| t.preview.as_ref()?.as_ref().err()).filter(|_| self.rendering().is_some()).map(|e| format!("Markdown preview unavailable: {e}; showing the source"));
        let memory = semantics::memory_notice(self.semantics.memory());
        doc.into_iter().chain(gone).chain(changing).chain(approximate).chain(retrieved).chain(origin).chain(preview).chain(memory).chain(self.messages.iter().cloned()).collect()
    }
    pub fn palette(&self) -> &Arc<Palette> {
        &self.palette
    }
    /// The semantic provider for the active document and what it is doing; `None` when its language has none.
    pub fn provider(&self) -> Option<ProviderView> {
        // Source a language server returned is not a document any server holds.
        if self.active()?.retrieved.is_some() {
            return None;
        }
        self.semantics.view(self.document()?)
    }
    fn project_provider(&self) -> Option<(usize, PathBuf)> {
        let (provider, Some(Some(root))) = self.semantics.resolved(self.document()?)? else { return None };
        Some((provider, root))
    }
    /// Compact status shown before paths and other notices so setup failures remain visible.
    pub fn language_server_label(&self) -> String {
        self.provider().map_or_else(|| if self.document().is_some() { "LSP unsupported" } else { "LSP no file" }.to_owned(), |v| v.status_label())
    }
    pub fn server_runtime(&self) -> Option<&semantics::runtime::Runtime> { self.server_runtime.as_ref() }
    /// Every language server of this session and its state.
    pub fn servers(&self) -> Vec<ProviderView> {
        self.semantics.views()
    }
    /// What the reader is asking a provider, while it waits (Escape cancels).
    /// The occurrences of the name at the caret, while they are about the active document as it is and the caret
    /// is on one of them.
    pub fn occurrences(&self) -> Option<&Occurrences> {
        let (o, t, caret) = (self.occurrences.as_ref()?, self.active()?, self.selection()?.head);
        let current = t.source == o.path && t.doc.as_ref().is_some_and(|d| d.version() == o.version);
        (current && o.ranges.iter().any(|(r, _)| r.start <= caret && caret <= r.end)).then_some(o)
    }

    /// The commit an annotation opened, while its tab is open.
    pub fn commit_popup(&self) -> Option<&crate::blame::CommitPopup> {
        self.commit.as_ref().filter(|c| self.tabs.iter().any(|t| t.id == c.tab))
    }

    /// The documentation to show, while it is about the active tab's document as it is.
    pub fn documentation(&self) -> Option<&Documentation> {
        let d = self.documentation.as_ref()?;
        let shown = [self.active, self.other].contains(&Some(d.tab));
        let t = self.tabs.iter().find(|t| t.id == d.tab)?;
        (shown && t.doc.as_ref().is_some_and(|doc| doc.version() == d.version)).then_some(d)
    }

    /// Whether resting the pointer on a name shows its documentation.
    /// Whether long lines wrap (the setting; see [`State::wrap_cols`] for a tab).
    pub fn soft_wrap(&self) -> bool {
        self.settings.soft_wrap
    }
    /// Whether each pane pins the enclosing declarations at its top (the setting).
    pub fn sticky_scroll(&self) -> bool {
        self.settings.sticky_scroll
    }
    /// Whether each pane shows its breadcrumb bar (the setting).
    pub fn breadcrumbs(&self) -> bool {
        self.settings.breadcrumbs
    }
    /// Setting `id`'s state, for the menus (`None`: `id` is not one of these settings).
    pub(crate) fn setting(&self, id: CommandId) -> Option<bool> {
        match id {
            CommandId::PersistentIndex => Some(self.settings.persistent_index),
            CommandId::ExcludeRubyTooling => self.project_provider().map(|(_, root)| self.semantics.settings().ruby_tooling_excluded(&root)),
            CommandId::InlayTypes => Some(self.settings.inlay_types),
            CommandId::InlayParameters => Some(self.settings.inlay_parameters),
            CommandId::SemanticHighlighting => Some(self.settings.semantic_highlighting),
            CommandId::SoftWrap => Some(self.settings.soft_wrap),
            CommandId::StickyScroll => Some(self.settings.sticky_scroll),
            CommandId::Breadcrumbs => Some(self.settings.breadcrumbs),
            CommandId::DocumentationOnHover => Some(self.settings.documentation_on_hover),
            CommandId::FileIcons => Some(self.settings.file_icons),
            CommandId::AnnotateBlame => Some(self.active().is_some_and(|t| t.blame.is_some())),
            _ => None,
        }
    }
    /// Whether type hints are shown (the setting).
    pub fn inlay_types(&self) -> bool {
        self.settings.inlay_types
    }
    /// Whether parameter-name hints are shown (the setting).
    pub fn inlay_parameters(&self) -> bool {
        self.settings.inlay_parameters
    }
    /// Whether semantic tokens colour names (the setting).
    pub fn semantic_highlighting(&self) -> bool {
        self.settings.semantic_highlighting
    }
    /// Pane `pane`'s breadcrumbs (`None` while they are off or the pane shows nothing).
    pub fn breadcrumbs_for(&self, pane: u8) -> Option<Breadcrumbs> {
        let tab = self.shown(pane).filter(|_| self.settings.breadcrumbs)?;
        Some(crate::breadcrumbs::build(tab, self.project.as_ref()))
    }
    /// What is inside crumb `crumb` of pane `pane`'s bar.
    pub fn siblings(&self, pane: u8, crumb: usize) -> Vec<Sibling> {
        let (Some(bar), Some(tab)) = (self.breadcrumbs_for(pane), self.shown(pane)) else { return Vec::new() };
        crate::breadcrumbs::inside(&bar, crumb, tab, self.project.as_ref())
    }
    /// Whether parses should find declarations (sticky scroll or breadcrumbs is on).
    fn wants_declarations(&self) -> bool {
        self.settings.sticky_scroll || self.settings.breadcrumbs
    }

    /// The adapter's current text viewport, in its own columns and rows.
    pub fn text_size(&self, pane: u8) -> Option<(usize, usize)> { self.text_size.get(pane as usize).copied() }

    /// The columns `tab`'s lines wrap at: `None` when wrap is off, the width is not known yet, or the tab shows a
    /// large file, an image or a Markdown preview.
    pub fn wrap_cols(&self, tab: &Tab) -> Option<usize> {
        let cols = self.text_size.get(tab.pane as usize).map_or(0, |s| s.0);
        let doc = tab.doc.as_ref()?;
        let text = !doc.is_large() && (doc.image().is_none() || tab.image_source) && !matches!(tab.rendering(), Some(Rendering::Preview(_) | Rendering::Pending));
        (self.settings.soft_wrap && cols > 0 && text).then_some(cols)
    }

    pub fn documentation_on_hover(&self) -> bool {
        self.settings.documentation_on_hover
    }

    /// Whether files show their file-type icons (the setting; a terminal may override it at startup).
    pub fn file_icons(&self) -> bool {
        self.settings.file_icons
    }

    pub fn asking(&self) -> Option<String> {
        let a = self.asking.as_ref()?;
        let what = match a.ask {
            Ask::Definition => "Finding the definition",
            Ask::Peek => "Peeking at the definition",
            Ask::Implementation => "Finding implementations",
            Ask::TypeDefinition => "Finding the type definition",
            Ask::CallHierarchy(_) => "Finding the call hierarchy",
            Ask::TypeHierarchy(_) => "Finding the type hierarchy",
            Ask::Usages => "Finding usages",
            Ask::Symbols => "Finding symbols",
            Ask::FileSymbols => "Listing the file's symbols",
        };
        Some(match self.semantics.waiting() {
            Some(r) if r == a.request => format!("{what}: waiting for the language server to start…"),
            _ => format!("{what}…"),
        })
    }
}

pub struct Config {
    pub themes: ThemePaths,
    /// Where recent roots are kept, outside any checkout.
    pub recent: PathBuf,
    /// The reader's own settings (the tree's order), outside any checkout.
    pub settings: PathBuf,
    /// Explicit semantic provider settings, outside any checkout.
    pub providers: PathBuf,
    /// The reader's cache, outside any checkout (language servers' workspace data).
    pub cache: PathBuf,
    /// How long an unused language server stays warm.
    pub idle: std::time::Duration,
    /// The time, for idle expiry (tests drive it).
    pub clock: semantics::Clock,
    pub overrides: ThemeOverrides,
    /// The platform appearance at start.
    pub dark: bool,
}

impl Config {
    /// Shared Lenz appearance; Reader settings, providers, cache and recents stay independent.
    pub fn from_env(dark: bool) -> Self {
        Self {
            themes: ThemePaths::shared(),
            recent: xdg_state().join("lenz-reader/recent-roots.json"),
            settings: xdg_config().join("lenz-reader/settings.json"),
            providers: xdg_config().join("lenz-reader/providers.json"),
            cache: xdg_cache().join("lenz-reader"),
            idle: semantics::IDLE,
            clock: Arc::new(std::time::Instant::now),
            overrides: ThemeOverrides::shared_from_env(),
            dark,
        }
    }
}

/// A project inventory's identity: its root, walk, size, and whether that walk is complete (a rescan keeps the
/// old inventory until its walk ends; a walk's folders merge in as it goes).
type Inventory = (PathBuf, u64, usize, bool);
/// What a watched-folder set was built from: the inventory, and the tabs' folders outside it.
pub type WatchKey = (Option<Inventory>, std::collections::BTreeSet<PathBuf>);

/// Where a source opened by [`Command::OpenAt`] or a place jump goes once shown.
#[derive(Clone, Debug)]
enum Goto {
    At(lenz_content::goto::LineColumn),
    /// A mark, found again in the text as it is when shown.
    Mark(crate::places::Mark),
}

#[derive(Clone)]
enum LayoutPurpose { Command(Command), Selection, Reveal { line: usize, byte: usize }, Band { line: usize, row: usize } }

struct LayoutIntent {
    request: crate::layout::Request, purpose: LayoutPurpose, selection: Option<Selection>, top: (usize, usize), visible: Arc<crate::folds::Visible>,
}

pub struct Session {
    state: State,
    cache: InactiveCache,
    index_cache_dir: PathBuf,
    index_cache: Option<Arc<std::sync::Mutex<crate::index_cache::Cache>>>,
    index_revision: Arc<AtomicU64>,
    index_rebuilding: bool,
    projections: crate::projection_jobs::Scheduler,
    layouts: crate::layout_jobs::Scheduler,
    layout_intents: [Option<LayoutIntent>; 2],
    jobs: Jobs,
    themes: ThemeService,
    theme_preview: Option<String>,
    request: u64,
    /// `request`, for jobs to skip work a later open or reload superseded.
    latest: Arc<AtomicU64>,
    /// The current document's version, read by syntax work to stop when superseded; and the other pane's.
    current: Arc<AtomicU64>,
    other_current: Arc<AtomicU64>,
    /// Bumped when a preview is no longer wanted (its tab shows the source): its work stops.
    renders: Arc<AtomicU64>,
    versions: Arc<AtomicU64>,
    /// The tab the pending load or check is for, and whether it reloads shown content.
    pending: Option<(TabId, bool)>,
    next_tab: u64,
    /// The current root's generation, for scans and rankings to stop when superseded.
    root_generation: Arc<AtomicU64>,
    /// The current project's generation (unchanged by rescans), for indexing to stop when the root is replaced.
    project_generation: Arc<AtomicU64>,
    /// The current finder query's sequence number.
    query_latest: Arc<AtomicU64>,
    navigation_latest: Arc<AtomicU64>,
    recent_file: PathBuf,
    /// Where each project's places file is kept (`projects/` beside the recent roots).
    places_dir: PathBuf,
    places_saver: crate::places::Saver,
    /// Where the project's places are saved, once its file was read (or an unreadable one set aside): until then a
    /// save would replace them with what this session saw so far, so none is made.
    places_file: Option<PathBuf>,
    /// Bumped at each change of the places: a merge returned for an older save is not taken.
    places_seq: u64,
    /// Recent-root saves are numbered; a save older than the last one written is skipped.
    recent_saves: Arc<std::sync::Mutex<u64>>,
    recent_seq: u64,
    settings_file: PathBuf,
    settings_saves: Arc<std::sync::Mutex<u64>>,
    settings_seq: u64,
    runtime_request: u64,
    restore_workspace: bool,
    watcher: Watcher,
    /// The project's folders to watch, for its root, walk and size (built once per inventory), and the tabs' folders
    /// outside them that were last added.
    watching: Option<WatchKey>,
    /// Markdown sources last switched to their preview: they open as previews again (others as source).
    previewed: std::collections::HashSet<PathBuf>,
    /// The inventory's folders as last built (off the UI thread), for tab-only changes to reuse.
    inventory_folders: Option<(Option<Inventory>, Arc<std::collections::BTreeSet<PathBuf>>)>,
    /// A project rescan is running; another one is due when it ends.
    rescanning: bool,
    rescan_again: bool,
    /// Coarse watch events stay pending through overlapping scans, so servers see the final inventory.
    rescan_changes: Change,
    /// Pre-overflow content, bounded by the declaration index; fresh indexing reads resolve these reports.
    overflow_fingerprints: std::collections::HashMap<Arc<str>, [u8; 32]>,
    refresh_seq: u64,
    /// The current search, for its job to stop when superseded.
    search_latest: Arc<AtomicU64>,
    /// Case, whole word, regular expression: the find box's and project search's options, while the reader runs.
    find_options: lenz_content::find::FindOptions,
    find_seq: u64,
    /// Where to go once a source opened by [`Command::OpenAt`] or a place jump is shown.
    pending_goto: Option<(PathBuf, Goto)>,
    search_seq: u64,
    semantic_seq: u64,
    /// The documentation request: its id, and the tab, version, name and trigger it is about.
    pending_doc: Option<(u64, TabId, u64, std::ops::Range<usize>, DocTrigger)>,
    /// The occurrences request: its id, and the file, version and caret it is about.
    pending_highlight: Option<(u64, PathBuf, u64, usize)>,
    /// The tabs' document versions when language servers were last synchronized (0: synchronize next time).
    synced: u64,
    /// The latest hierarchy level asked for: a syntax-built level for another stops reading relations.
    hierarchy_latest: Arc<AtomicU64>,
    /// Per decoration kind: bumped when its setting changes, so decodes started before are dropped.
    decor_generation: [u64; 2],
    /// A memory sample is being taken.
    sampling: bool,
    /// The latest result-open or preview request; any other navigation supersedes a pending open.
    open_seq: u64,
    preview_latest: Arc<AtomicU64>,
    /// The latest preview colouring request; a parse stops once another replaces it.
    preview_parse: Arc<AtomicU64>,
    /// The snapshot being coloured for the preview (its version) and its request.
    preview_parsing: Option<(u64, u64)>,
    /// The latest selection-steps request; a parse stops once another replaces it.
    expand_latest: Arc<AtomicU64>,
    /// Steps being parsed for a tab's selection: the tab, its version, the selection and the request.
    /// Extend Selection waiting for its steps: tab, version, selection, request, and whether it scrolls to the result
    /// (not for a triple click).
    expanding: Option<(TabId, u64, Selection, u64, bool)>,
    /// The next Extend Selection comes from a triple click: it leaves the view where it was clicked.
    click_extend: bool,
    /// The last blame request made (each tab keeps the one it wants).
    blame_requests: u64,
    commit_requests: u64,
    checkout_requests: u64,
    /// The last lookup found no repository though a `.git` was there: the tick does not ask again.
    repo_git_seen: bool,
    /// What the running checkout does, to run again from its dialog when git refuses.
    pending_checkout: Option<crate::branches::Retry>,
    branches_requests: u64,
    tx: async_channel::Sender<Completion>,
    rx: async_channel::Receiver<Completion>,
}

/// Why Compare Selected cannot run yet, in words for both frontends.
const MARK_TWO: &str = "Mark two files or folders in the explorer (Ctrl+click, or Space)";

/// Lenz Diff beside `exe` (the terminal app when `terminal`), or why not, for the command `what`.
pub(crate) fn lenz_diff(exe: &Path, terminal: bool, what: &str) -> Result<String, String> {
    let program = exe.with_file_name(if terminal { "lenz-diff-tui" } else { "lenz-diff" });
    if !program.is_file() {
        return Err(format!("{what}: Lenz Diff is not beside Lenz Code ({}); reinstall Lenz", program.display()));
    }
    Ok(program.to_string_lossy().into_owned())
}

impl Session {
    pub fn projection(&self, pane: u8) -> Option<&Arc<crate::projection::Batch>> {
        self.projections.ready(pane).filter(|b| projection_current(&self.state, &b.request))
    }
    pub fn new(config: Config) -> Self {
        let recent = read_recent(&config.recent);
        let settings = read_settings(&config.settings);
        let mut themes = ThemeService::new_deferred(config.themes, config.overrides, config.dark);
        let messages = themes.take_notices();
        // Bounded: a scan that outruns the UI waits instead of queueing the whole inventory.
        let (tx, rx) = async_channel::bounded(COMPLETIONS);
        let changes = tx.clone();
        let watcher = Watcher::new(move |c| changes.send_blocking(Completion::Changed(c)).is_ok());
        // Time passing: idle language servers stop, their memory is sampled. Ends with the session's channel.
        let ticks = tx.clone();
        let _ = std::thread::Builder::new().name("reader-tick".into()).spawn(move || {
            while !ticks.is_closed() {
                std::thread::sleep(TICK);
                if ticks.send_blocking(Completion::Semantic(semantics::Event::Tick)).is_err() {
                    return;
                }
            }
        });
        let merged = tx.clone();
        Self {
            state: State {
                sources: Sources::default(),
                tabs: Vec::new(),
                active: None,
                other: None,
                focused: 0,
                history: History::default(),
                loading: None,
                messages,
                palette: themes.snapshot().palette,
                project: None,
            libraries: Default::default(),
            tree_revision: 0,
                explorer: Default::default(), workspace_revision: 0,
                recent,
                places: Default::default(),
                missing: Default::default(),
                settings,
                text_size: [(0, 0); 2],
                server_runtime: None, runtime_saving: None, provider_saves: 0,
                results: None,
                navigation: None,
                index: DeclIndex::new(index::INDEX_BYTES),
                semantics: Semantics::new(config.providers, config.cache.clone(), config.idle, config.clock),
                asking: None,
                occurrences: None,
                documentation: None,
                commit: None,
                repo: None,
                repo_anchor: None,
                repo_answered: None,
                branches: None,
                tree_marks: Vec::new(),
                checkout_running: None,
                checkout_problem: None,
                stash_conflicts: None,
            },
            cache: InactiveCache::new(INACTIVE_BYTES),
            index_cache_dir: config.cache,
            index_cache: None,
            index_revision: Arc::new(AtomicU64::new(0)),
            index_rebuilding: false,
            jobs: Jobs::new(),
            projections: Default::default(),
            layouts: Default::default(),
            layout_intents: Default::default(),
            themes,
            theme_preview: None,
            request: 0,
            latest: Arc::new(AtomicU64::new(0)),
            current: Arc::new(AtomicU64::new(0)),
            other_current: Arc::new(AtomicU64::new(0)),
            renders: Arc::new(AtomicU64::new(0)),
            versions: Arc::new(AtomicU64::new(0)),
            pending: None,
            next_tab: 0,
            root_generation: Arc::new(AtomicU64::new(0)),
            project_generation: Arc::new(AtomicU64::new(0)),
            query_latest: Arc::new(AtomicU64::new(0)),
            navigation_latest: Arc::new(AtomicU64::new(0)),
            places_dir: config.recent.parent().map_or_else(PathBuf::new, |d| d.join("projects")),
            places_saver: crate::places::Saver::new(crate::places::DEBOUNCE, move |generation, seq, places| {
                let _ = merged.send_blocking(Completion::PlacesMerged { generation, seq, places });
            }),
            places_file: None,
            places_seq: 0,
            recent_file: config.recent,
            recent_saves: Arc::default(),
            recent_seq: 0,
            settings_file: config.settings,
            settings_saves: Arc::default(),
            settings_seq: 0,
            runtime_request: 0,
            restore_workspace: false,
            watcher,
            watching: None,
            inventory_folders: None,
            previewed: Default::default(),
            rescanning: false,
            rescan_again: false,
            rescan_changes: Change::default(),
            overflow_fingerprints: Default::default(),
            refresh_seq: 0,
            search_latest: Arc::new(AtomicU64::new(0)),
            find_options: Default::default(),
            find_seq: 0,
            pending_goto: None,
            search_seq: 0,
            semantic_seq: 0,
            pending_highlight: None,
            pending_doc: None,
            synced: 0,
            hierarchy_latest: Arc::default(),
            decor_generation: [0, 0],
            sampling: false,
            open_seq: 0,
            preview_latest: Arc::new(AtomicU64::new(0)),
            preview_parse: Arc::new(AtomicU64::new(0)),
            preview_parsing: None,
            expand_latest: Arc::new(AtomicU64::new(0)),
            expanding: None,
            click_extend: false,
            blame_requests: 0,
            commit_requests: 0,
            checkout_requests: 0,
            repo_git_seen: false,
            pending_checkout: None,
            branches_requests: 0,
            tx,
            rx,
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Background results; the adapter feeds each one back through [`Session::dispatch`].
    pub fn completions(&self) -> async_channel::Receiver<Completion> {
        self.rx.clone()
    }

    /// Theme service updates; the adapter feeds each one back as [`Completion::Theme`].
    pub fn theme_updates(&self) -> async_channel::Receiver<ThemeUpdate> {
        self.themes.updates()
    }

    /// Start theme I/O after the first paint.
    pub fn start_themes(&mut self) {
        self.themes.start();
    }

    pub fn theme_entries(&self) -> &[ThemeEntry] {
        self.themes.entries()
    }

    pub fn theme_id(&self) -> String {
        self.themes.snapshot().selection.id().to_owned()
    }

    pub fn theme_source(&self) -> String {
        self.themes.snapshot().source
    }

    /// A background quit task can wait until metadata already submitted by SaveWorkspace is durable.
    pub fn workspace_saved(&self) -> std::sync::mpsc::Receiver<()> { self.places_saver.flush() }

    pub fn inactive_bytes(&self) -> usize {
        self.cache.used()
    }

    #[cfg(test)]
    fn cache_entries(&self) -> Vec<Arc<Document>> {
        self.cache.documents()
    }

    /// Why folding is unavailable in the active tab now.
    fn fold_unavailable(&self) -> Option<&'static str> {
        let Some(t) = self.state.active() else { return Some("No file is open") };
        let Some(doc) = &t.doc else { return Some("No file is open") };
        if doc.image().is_some() && !t.image_source {
            return Some("Images have no folds");
        }
        if matches!(t.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) {
            return Some("The Markdown preview has no folds");
        }
        if doc.is_large() {
            return Some("Folding is off for large files");
        }
        if doc.language().is_none() {
            return Some("No folds for this file type");
        }
        t.folds.is_none().then_some("Folds are not ready yet")
    }

    pub fn actions(&self) -> Vec<Action> {
        CommandId::ALL.iter().map(|&id| Action { id, label: id.label(), disabled: self.disabled(id), on: self.on(id) }).collect()
    }

    /// Toggle `id`'s state for the menus (`None`: not a toggle): a setting, or a find option (the session's, which the
    /// active tab's find uses).
    fn on(&self, id: CommandId) -> Option<bool> {
        let o = self.find_options;
        match id {
            CommandId::MatchCase => Some(o.case),
            CommandId::WholeWord => Some(o.word),
            CommandId::RegularExpression => Some(o.regex),
            _ => self.state.setting(id),
        }
    }

    pub fn disabled(&self, id: CommandId) -> Option<&'static str> {
        if (self.state.runtime_saving.is_some() || self.state.provider_saves > 0) && matches!(id,
            CommandId::ServerRuntime | CommandId::EnableServer | CommandId::DisableServer | CommandId::WorkspaceRoot
            | CommandId::RestartServer | CommandId::RebuildIndex | CommandId::ExcludeRubyTooling | CommandId::ResetProjectServerSettings) {
            return Some("Waiting for language server settings to finish saving");
        }

        let current = self.state.current();
        let open = current.is_some();
        let tabs = self.state.pane_tabs(self.state.focused).count();
        match id {
            CommandId::StatusDetails | CommandId::FindFile | CommandId::SearchEverywhere | CommandId::FindSymbol | CommandId::FindAction | CommandId::Open | CommandId::OpenFolder | CommandId::ChooseTheme | CommandId::About | CommandId::NewWindow => None,
            CommandId::SplitRight => (self.state.settings.split == Some(Split::Columns)).then_some("Already split side by side"),
            CommandId::SplitDown => (self.state.settings.split == Some(Split::Rows)).then_some("Already split one above the other"),
            CommandId::Unsplit | CommandId::FocusOtherPane => self.state.settings.split.is_none().then_some("The file panel is not split"),
            CommandId::MoveTabToOtherPane => match (self.state.settings.split, self.state.active) {
                (None, _) => Some("The file panel is not split"),
                (_, None) => Some("No tab is open"),
                _ => None,
            },
            CommandId::FoldersFirst | CommandId::AllByName => ((id == CommandId::FoldersFirst) == self.state.settings.folders_first).then_some("Already sorted this way"),
            CommandId::ToggleExplorer => (!self.state.has_tree()).then_some("No folder or library is open"),
            CommandId::LocateFile => match self.state.active() {
                None => Some("No file is open"),
                _ if self.state.active_in_tree().is_none() => Some("The file is not in the explorer"),
                _ => None,
            },
            CommandId::Back => (!self.state.can_back()).then_some("Nothing to go back to"),
            CommandId::Forward => (!self.state.can_forward()).then_some("Nothing to go forward to"),
            CommandId::ToggleBookmark | CommandId::SetBookmarkMnemonic => match self.state.active() {
                Some(t) if self.state.view(t.id).is_some() && !shows_text(t) => Some("Bookmarks need a text view"),
                Some(t) if self.state.view(t.id).is_some() => None,
                _ => Some("No file is open"),
            },
            CommandId::ShowBookmarks | CommandId::NextBookmark | CommandId::PreviousBookmark => self.state.places.bookmarks().is_empty().then_some("No bookmarks"),
            CommandId::RecentFiles => self.state.place_rows(PlaceList::RecentFiles, "").is_empty().then_some("No recent files"),
            CommandId::RecentLocations => self.state.places.recent_locations().is_empty().then_some("No recent locations"),
            CommandId::PinTab => match self.state.active() {
                None => Some("No tab is open"),
                Some(t) if t.retained => Some("This tab is already kept open"),
                Some(_) => None,
            },
            CommandId::NextTab | CommandId::PreviousTab => (tabs < 2).then_some("Only one tab is open"),
            CommandId::Close => (tabs == 0).then_some("No tab is open"),
            CommandId::OpenRecent => self.state.recent.is_empty().then_some("No recent folders"),
            CommandId::SearchProject => self.state.project.is_none().then_some("No folder is open"),
            CommandId::FindInFile => match self.state.active() {
                _ if !open => Some("No file is open"),
                Some(t) if matches!(t.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) => Some("Find works on the source; Ctrl+Shift+V shows it"),
                Some(t) if self.state.view(t.id).map(|view| view.snapshot()).is_some_and(|d| d.image().is_some()) && !t.image_source => Some("An image has no text to find"),
                _ => None,
            },
            CommandId::NextMatch | CommandId::PreviousMatch | CommandId::CloseFind | CommandId::ListMatches => self.state.active().and_then(|t| t.find.as_ref()).is_none().then_some("No find is open"),
            CommandId::MatchCase | CommandId::WholeWord | CommandId::RegularExpression => None,
            CommandId::SoftWrap => match self.state.active().and_then(|t| Some((t, self.state.view(t.id).map(|view| view.snapshot())?))) {
                Some((_, d)) if d.is_large() => Some("Too large to wrap"),
                Some((t, d)) if d.image().is_some() && !t.image_source => Some("An image has no lines"),
                Some((t, _)) if matches!(t.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) => Some("The preview wraps already; Ctrl+Shift+V shows the source"),
                _ => None,
            },
            CommandId::StickyScroll | CommandId::Breadcrumbs | CommandId::PersistentIndex => None,
            CommandId::RebuildIndex => if self.state.project.is_none() { Some("Open a project folder first") } else if self.index_rebuilding { Some("The project index is being cleared") } else { None },
            CommandId::FocusBreadcrumbs if !self.state.settings.breadcrumbs => Some("Breadcrumbs are off"),
            CommandId::FocusBreadcrumbs => self.state.active().is_none().then_some("No file is open"),
            CommandId::GoToLine => match self.state.active() {
                _ if !open => Some("No file is open"),
                Some(t) if matches!(t.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) => Some("Lines are in the source; Ctrl+Shift+V shows it"),
                Some(t) if self.state.view(t.id).map(|view| view.snapshot()).is_some_and(|d| d.image().is_some()) && !t.image_source => Some("An image has no lines"),
                _ => None,
            },
            CommandId::Outline => match current.as_ref() {
                None => Some("No file is open"),
                Some(d) if d.is_large() => Some("Large file: no outline"),
                Some(d) if !d.language().is_some_and(lenz_content::highlight::has_declarations) => Some("No outline for this language"),
                Some(_) => None,
            },
            CommandId::FindDeclarations => self.state.project.is_none().then_some("No folder is open"),
            CommandId::RerunSearch | CommandId::CloseResults => self.state.results.is_none().then_some("No search results"),
            CommandId::Reload if self.state.active().is_some_and(|t| t.retrieved.is_some()) => Some("Source a language server returned cannot be reloaded; ask for the definition again"),
            CommandId::Reload | CommandId::SelectAll => (!open).then_some("No file is open"),
            CommandId::Collapse | CommandId::Expand | CommandId::CollapseAll | CommandId::ExpandAll | CommandId::CollapseCommentsImports => self.fold_unavailable(),
            CommandId::ExtendSelection => match self.state.active() {
                Some(t) if self.state.view(t.id).map(|view| view.snapshot()).is_some_and(|d| d.image().is_some()) && !t.image_source => Some("Only text has a selection to extend"),
                Some(t) if matches!(t.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) => Some("Only text has a selection to extend"),
                Some(t) if self.state.view(t.id).is_some() => None,
                _ => Some("No file is open"),
            },
            CommandId::ShrinkSelection => match self.state.active() {
                Some(t) if t.expansion.as_ref().is_some_and(|e| e.at > 0 && Some(e.current()) == t.selection) => None,
                _ => Some("Nothing to shrink"),
            },
            // Each arrives with its own change.
            // The shown hierarchy turns around from its own root.
            CommandId::CallHierarchyCallers | CommandId::CallHierarchyCallees if self.state.results.as_ref().and_then(|r| r.hierarchy.as_ref()).is_some_and(|h| !h.direction.is_type()) => None,
            CommandId::TypeHierarchySupertypes | CommandId::TypeHierarchySubtypes if self.state.results.as_ref().and_then(|r| r.hierarchy.as_ref()).is_some_and(|h| h.direction.is_type()) => None,
            CommandId::DocumentationOnHover | CommandId::FileIcons => None,
            CommandId::InlayTypes | CommandId::InlayParameters | CommandId::SemanticHighlighting => None,
            CommandId::CopyPath => self.state.active().is_none().then_some("No file is open"),
            CommandId::OpenInEditor | CommandId::OpenWithDefaultApp => self.external_tab().err(),
            CommandId::ShowCommit | CommandId::ShowCommitDiff | CommandId::CopyRevision => self.commit_target().err(),
            CommandId::GitBranches | CommandId::NewBranch | CommandId::CheckoutRevision => match &self.state.repo {
                None => Some("Not in a Git repository"),
                Some(_) if self.state.checkout_running.is_some() => Some("A checkout is running"),
                Some(_) => None,
            },
            // Whether there is a base, a commit or another worktree is said when one runs (no git here).
            CommandId::ShowLocalChanges | CommandId::CompareWithRevision | CommandId::CompareWithBase | CommandId::ShowBranchChanges | CommandId::CompareWithWorktree => {
                self.state.repo.is_none().then_some("Not in a Git repository")
            }
            CommandId::ResolveConflicts => self.state.stash_conflicts.is_none().then_some("Smart Checkout left no conflicts"),
            CommandId::AnnotateBlame | CommandId::CloseAnnotations => match self.state.active() {
                None => Some("No file is open"),
                // Turning annotations off always works, also once the file has gone.
                Some(t) if t.blame.is_some() => None,
                Some(t) if t.retrieved.is_some() => Some("Source from a language server has no Git history"),
                Some(t) if t.unavailable.is_some() => Some("The file is unavailable"),
                Some(t) if t.doc.is_none() && self.state.loading().is_some() => Some("The file is still loading"),
                // An SVG shown as its source is text.
                Some(t) if self.state.view(t.id).map(|view| view.snapshot()).is_none_or(|d| d.image().is_some() && !t.image_source) => Some("Only text can be annotated"),
                Some(t) if id == CommandId::CloseAnnotations && t.blame.is_none() => Some("Annotations are off"),
                Some(_) => None,
            },
            // The explorer's selection, which only the adapter knows, is checked when it asks.
            CommandId::CompareWith => (self.state.active().is_none() && self.state.project.is_none()).then_some("No file or folder is open"),
            CommandId::CompareSelected => (self.state.tree_marks.len() < 2).then_some(MARK_TWO),
            CommandId::CopyPathLine => match self.state.active() {
                None => Some("No file is open"),
                Some(t) if t.doc.is_none() => Some("No text to point at"),
                Some(_) => None,
            },
            CommandId::Copy => match self.state.selection() {
                _ if !open => Some("No file is open"),
                // The adapter holds a preview's selection.
                _ if matches!(self.state.rendering(), Some(Rendering::Preview(_))) => None,
                // A caret copies its line.
                Some(_) => None,
                None => Some("Nothing is selected"),
            },
            CommandId::MarkdownSource => self.preview_control(self.state.focused_pane())
                .map_or(Some("Not a Markdown file"), |control| control.disabled),
            CommandId::ImageSource => match current.as_ref().and_then(|d| d.image()) {
                Some(i) if i.format == lenz_content::image::Format::Svg && i.pixels.is_ok() => None,
                Some(i) if i.format == lenz_content::image::Format::Svg => Some("This SVG cannot be rendered"),
                Some(_) => Some("A raster image has no source text"),
                None => Some("Not an image"),
            },
            CommandId::Symbols => match &self.state.project {
                None => Some("No folder is open"),
                Some(p) => self.state.semantics.symbols_unavailable(Some(p.root())),
            },
            CommandId::LanguageServer | CommandId::ServerDetails | CommandId::CopyServerDetails => None,
            CommandId::ExcludeRubyTooling | CommandId::ResetProjectServerSettings => match self.state.project_provider() {
                None => Some("Open a source file and wait for its language server workspace"),
                Some((p, _)) if id == CommandId::ExcludeRubyTooling && semantics::provider::PROVIDERS[p].id != "ruby-lsp" => Some("Open a Ruby source file to configure Ruby indexing"),
                Some((_, root)) if id == CommandId::ExcludeRubyTooling && root.join(".index.yml").exists() => Some("Ruby indexing is configured by this project’s .index.yml"),
                Some((p, root)) if id == CommandId::ResetProjectServerSettings && !self.state.semantics.settings().has_project(semantics::provider::PROVIDERS[p].id, &root) => Some("This project already inherits global language server settings"),
                _ => None,
            },
            CommandId::ServerRuntime => if self.state.provider_saves > 0 { Some("Waiting for provider settings to finish saving") }
                else if self.runtime_kind().is_none() { Some("Open a Java or Python source file to choose its server runtime") } else { None },
            CommandId::RestartServer => match self.state.provider() {
                None => Some("No language server for this file"),
                Some(v) if v.status == semantics::Status::Disabled => Some("Disabled for this folder"),
                Some(v) if v.root.is_none() || v.status == semantics::Status::Queued => Some("The language server has not started"),
                Some(_) => None,
            },
            CommandId::DisableServer | CommandId::EnableServer => match (self.state.provider(), self.state.project.is_some()) {
                (None, _) => Some("No language server for this file"),
                (_, false) => Some("No folder is open"),
                (Some(v), _) if (v.status == semantics::Status::Disabled) == (id == CommandId::DisableServer) => {
                    Some(if id == CommandId::DisableServer { "Already disabled for this folder" } else { "Not disabled" })
                }
                _ => None,
            },
            CommandId::Definition | CommandId::PeekDefinition | CommandId::TypeDefinition | CommandId::Implementation | CommandId::Documentation | CommandId::CallHierarchy | CommandId::CallHierarchyCallers | CommandId::CallHierarchyCallees | CommandId::TypeHierarchy | CommandId::TypeHierarchySupertypes | CommandId::TypeHierarchySubtypes | CommandId::Usages | CommandId::FileSymbols => match (current.as_ref(), self.state.provider()) {
                (None, _) => Some("No file is open"),
                _ if self.state.active().is_some_and(|t| t.retrieved.is_some()) => Some("Not available in source a language server returned"),
                (Some(d), _) if d.is_large() => Some("Large file: no language server"),
                (_, None) => Some("No language server for this language"),
                (_, Some(v)) => match v.status {
                    semantics::Status::Missing(_) => Some("The language server is unavailable (see the status bar)"),
                    semantics::Status::Failed(_) => Some("The language server stopped (see the status bar)"),
                    semantics::Status::Disabled => Some("The language server is disabled for this folder"),
                    _ if self.state.selection().is_none() && id != CommandId::FileSymbols => Some("Place the caret on a name first"),
                    // Known once it has started; until then the request waits for it, as a definition does.
                    _ if !v.supports.is_empty() && id == CommandId::Implementation && !v.offers(semantics::Question::Implementation) => Some("The language server does not offer implementations"),
                    _ if !v.supports.is_empty() && id == CommandId::TypeDefinition && !v.offers(semantics::Question::TypeDefinition) => Some("The language server does not offer type definitions"),
                    _ if !v.supports.is_empty() && id == CommandId::Documentation && !v.offers(semantics::Question::Hover) => Some("The language server does not offer documentation"),
                    _ if !v.supports.is_empty() && matches!(id, CommandId::CallHierarchy | CommandId::CallHierarchyCallers | CommandId::CallHierarchyCallees) && !v.offers(semantics::Question::PrepareCallHierarchy) => Some("The language server does not offer call hierarchy"),
                    _ if !v.supports.is_empty() && matches!(id, CommandId::TypeHierarchy | CommandId::TypeHierarchySupertypes | CommandId::TypeHierarchySubtypes) && !v.offers(semantics::Question::PrepareTypeHierarchy) && !v.offers(semantics::Question::Usages) => Some(no_type_hierarchy(v.name)),
                    _ => None,
                },
            },
            CommandId::WorkspaceRoot => match (current.as_ref(), self.state.project.is_some()) {
                (None, _) => Some("No file is open"),
                (_, false) => Some("No folder is open"),
                (Some(d), _) if d.language().and_then(semantics::provider::for_language).is_none() => Some("No language server for this language"),
                _ => None,
            },
        }
    }

    fn runtime_kind(&self) -> Option<semantics::runtime::Kind> {
        match self.state.current()?.language()? {
            highlight::Language::Java => Some(semantics::runtime::Kind::Java),
            highlight::Language::Python => Some(semantics::runtime::Kind::PyrightNode),
            _ => None,
        }
    }

    /// Stable management actions shared by the application menu and server status menu.
    pub fn language_server_rows(&self) -> Vec<(String, Option<CommandId>)> {
        let mut rows = Vec::new();
        let view = self.state.provider();
        // Recovery rows keep their identity while asynchronous status text changes or wraps.
        for id in [
            CommandId::ServerDetails,
            CommandId::RestartServer,
            if view
                .as_ref()
                .is_some_and(|v| v.status == semantics::Status::Disabled)
            {
                CommandId::EnableServer
            } else {
                CommandId::DisableServer
            },
            CommandId::WorkspaceRoot,
            CommandId::ServerRuntime,
            CommandId::ExcludeRubyTooling,
            CommandId::ResetProjectServerSettings,
            CommandId::PersistentIndex,
            CommandId::RebuildIndex,
        ] {
            let label = self.on(id).map_or_else(|| id.label().to_owned(), |on| format!("{}: {}", id.label(), if on { "On" } else { "Off" }));
            rows.push((label, Some(id)));
        }
        rows
    }

    /// Full, unwrapped diagnostic snapshot for display and copying in either adapter.
    pub fn language_server_details(&self) -> String {
        let mut rows = Vec::new();
        let view = self.state.provider();
        let summary = match &view {
            Some(v) => v.summary(),
            None if self.state.document().is_some() => "No language server supports this document. Syntax highlighting and text search remain available.".into(),
            None => "Open a source file to start its language server automatically.".into(),
        };
        rows.push(summary);
        // Right below its status: the reason it may not be working.
        if let Some(error) = view.as_ref().and_then(|v| v.error.as_ref()) {
            rows.push(format!("Server error: {error}\nThe server may keep running with reduced results."));
        }
        if let Some((p, root)) = self.state.project_provider() {
            rows.push(format!("Project settings for {}: {}", root.display(), if self.state.semantics.settings().has_project(semantics::provider::PROVIDERS[p].id, &root) { "project overrides global settings" } else { "inheriting global settings" }));
            if semantics::provider::PROVIDERS[p].id == "ruby-lsp" {
                rows.push(format!("Ruby tooling gem exclusions: {}. Exclusions reduce navigation into tooling gems; project files keep their normal coverage. A project .index.yml takes precedence over editor indexing settings.", if self.state.semantics.settings().ruby_tooling_excluded(&root) { "enabled" } else { "not applied" }));
            }
        }
        if let Some(v) = view {
            let has_root = v.root.is_some();
            if let Some(root) = v.root {
                rows.push(format!("Workspace: {}", root.display()));
            }
            if let Some(p) = semantics::provider::PROVIDERS
                .iter()
                .find(|p| p.name == v.name)
            {
                let launch = self.state.semantics.settings().launch(p);
                rows.push(format!("Executable: {}", launch.executable.display()));
                if !launch.args.is_empty() { rows.push(format!("Arguments: {}", launch.args.join(" "))); }
                rows.push(format!("Server version: {}", v.version.as_deref().unwrap_or("unknown (the server has not reported a version)")));
                if let Some(tested) = p.tested {
                    rows.push(format!("Tested with Lenz: {} {tested}. Other versions may work; compatibility is unverified.", p.name));
                }
                if matches!(v.status, semantics::Status::Missing(_) | semantics::Status::Failed(_)) {
                    rows.push(format!("Setup: {}", p.setup));
                    if self.runtime_kind().is_some() {
                        rows.push("Choose Language Server Runtime to select an existing installation, validate it and restart the server. The failure may be a runtime selection or version problem; an exit code alone does not prove incompatibility.".into());
                    }
                }
                match v.status {
                    semantics::Status::Disabled => rows.push("Disabled by your folder settings. Choose Enable Language Server for This Folder to activate it.".into()),
                    semantics::Status::Missing(_) if self.state.document().is_some_and(|d| d.is_large()) => rows.push("Large files do not start a language server. Open a smaller source file to use semantic navigation.".into()),
                    semantics::Status::Missing(_) if !has_root => rows.push("No workspace was found. Choose Set Workspace Root, or open a file inside a supported project.".into()),
                    semantics::Status::Missing(_) | semantics::Status::Failed(_) => {
                        rows.push("Check the error above, the executable and its runtime/toolchain in the same environment as Lenz. Check executable, args, env and workspace root in your provider settings. After fixing setup, choose Restart Language Server; if no workspace was found, set its root first. Lenz never installs servers automatically.".into());
                    }
                    semantics::Status::Queued | semantics::Status::Starting => rows.push("Starts automatically when you open a supported source file. Startup is limited to 120 seconds; a failure will be shown here.".into()),
                    semantics::Status::Indexing(_) => rows.push("The server is still loading the project. Results may be incomplete until indexing finishes.".into()),
                    semantics::Status::Ready => rows.push("The server is running. If navigation fails, check the workspace root and server capabilities, or restart it.".into()),
                }
                rows.push(format!("Provider limits: {}", p.limits));
            }
        }
        rows.push(
            format!(
                "Provider settings: {}",
                self.state.semantics.settings_file().display()
            ),
        );
        rows.extend(self.state.notices());
        rows.join("\n\n")
    }

    /// The context menu for the active tab: navigation and copying for text; for an image or a Markdown preview
    /// only its path and its place in the explorer. Then opening elsewhere, comparing and splitting.
    pub fn context_menu(&self) -> Vec<MenuEntry> {
        use MenuEntry::{Command as C, Separator};
        let tab = self.state.active();
        let text = tab.is_some_and(|t| t.doc.is_some()) && !matches!(self.state.rendering(), Some(Rendering::Preview(_) | Rendering::Pending));
        let mut entries = Vec::new();
        if text {
            entries.extend([CommandId::Definition, CommandId::PeekDefinition, CommandId::TypeDefinition, CommandId::Implementation, CommandId::Usages, CommandId::CallHierarchy, CommandId::TypeHierarchy, CommandId::Documentation].map(C));
            entries.extend([Separator, C(CommandId::Collapse), C(CommandId::Expand), C(CommandId::CollapseAll), C(CommandId::ExpandAll)]);
            entries.extend([Separator, C(CommandId::Copy), C(CommandId::CopyPath), C(CommandId::CopyPathLine)]);
        } else {
            if matches!(self.state.rendering(), Some(Rendering::Preview(_))) {
                entries.push(C(CommandId::Copy));
            }
            entries.push(C(CommandId::CopyPath));
            if tab.is_some_and(|t| t.doc.is_some()) {
                entries.push(C(CommandId::CopyPathLine));
            }
        }
        entries.push(C(CommandId::LocateFile));
        if text {
            match tab.is_some_and(|t| t.blame.is_some()) {
                true => entries.extend([Separator, C(CommandId::ShowCommit), C(CommandId::ShowCommitDiff), C(CommandId::CopyRevision), C(CommandId::CloseAnnotations)]),
                false => entries.extend([Separator, C(CommandId::AnnotateBlame)]),
            }
        }
        entries.extend([Separator, C(CommandId::OpenInEditor), C(CommandId::OpenWithDefaultApp), C(CommandId::CompareWith)]);
        entries.extend([Separator, C(CommandId::SplitRight), C(CommandId::SplitDown)]);
        if self.state.settings.split.is_some() {
            entries.push(C(CommandId::Unsplit));
        }
        entries
    }

    /// Line `line`'s annotation in tab `tab`, while its annotations are on and git has reached it.
    pub fn annotation(&self, tab: TabId, line: usize) -> Option<crate::blame::Annotation<'_>> {
        crate::blame::annotation(self.state.tabs.iter().find(|t| t.id == tab)?.blame.as_ref()?, line)
    }

    /// A new request for the checkout about to run (an older one's answer is then dropped).
    fn begin_checkout(&mut self) -> u64 {
        self.checkout_requests += 1;
        self.state.checkout_running = Some(self.checkout_requests);
        self.state.checkout_problem = None;
        self.checkout_requests
    }

    /// The repository a checkout may write in: none outside git, and none while another checkout runs (said so).
    fn writable_repo(&mut self) -> Option<PathBuf> {
        let root = self.state.repo.as_ref()?.root.clone();
        if self.state.checkout_running.is_some() {
            self.state.messages.push("A checkout is running; wait for it to finish".into());
            return None;
        }
        Some(root)
    }

    /// Runs a checkout or a new branch (checked out) in `mode`; a refusal opens the dialog to run it again.
    fn retry(&mut self, retry: crate::branches::Retry, mode: lenz_content::branches::Mode) -> Effect {
        let Some(root) = self.writable_repo() else { return Effect { repaint: true, copy: None } };
        let request = self.begin_checkout();
        let label = retry.label().to_string();
        self.state.messages.push(match &retry {
            crate::branches::Retry::Checkout(_) => format!("Checking out {label}…"),
            crate::branches::Retry::NewBranch { .. } => format!("Creating branch {label}…"),
        });
        self.pending_checkout = Some(retry.clone());
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Visible, move || {
            let result = lenz_content::branches::attempt(&root, &retry, mode);
            let _ = tx.send_blocking(Completion::CheckedOut { request, root, label, made: false, result });
        });
        Effect { repaint: true, copy: None }
    }

    /// After a checkout: every tab checked against the disk, the folder rescanned, annotations blamed again (the
    /// history moved even where the bytes did not).
    fn checked_out(&mut self) {
        for t in &mut self.state.tabs {
            if let Some(b) = t.blame.as_mut() {
                b.stamp = None;
            }
        }
        self.changed(crate::watch::Change { overflow: true, ..Default::default() });
        for id in [self.state.active, self.state.other].into_iter().flatten() {
            self.annotate(id);
        }
    }

    /// Looks up the repository when what it is looked up for changed: the folder, else the active file's folder.
    fn sync_repo(&mut self) {
        let anchor = self.state.project.as_ref().map(|p| p.root().to_path_buf())
            .or_else(|| self.state.active().filter(|t| t.retrieved.is_none()).and_then(|t| t.source.parent().map(Path::to_path_buf)));
        if anchor == self.state.repo_anchor {
            return;
        }
        self.state.repo_anchor = anchor.clone();
        self.state.repo = None;
        self.state.branches = None;
        self.state.checkout_problem = None;
        self.state.stash_conflicts = None;
        let Some(anchor) = anchor else { return };
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Background, move || {
            let result = lenz_content::branches::locate(&anchor).and_then(|(root, git_dir)| {
                let head = lenz_content::branches::head(&root, &git_dir).ok_or("HEAD is unreadable")?;
                let signal = lenz_content::branches::head_signal(&git_dir);
                Ok(crate::branches::Repo { root, git_dir, head, signal })
            });
            let _ = tx.send_blocking(Completion::Repo { anchor, result });
        });
    }

    /// The commit the commit commands act on: the popup's, else the caret line's. (Its id and repository root.)
    fn commit_target(&self) -> Result<(String, PathBuf), &'static str> {
        if let Some(c) = self.state.commit_popup() {
            return Ok((c.sha.clone(), c.root.clone()));
        }
        let t = self.state.active().ok_or("No file is open")?;
        let b = t.blame.as_ref().ok_or("Annotations are off")?;
        let line = t.doc.as_ref().zip(t.selection).map(|(d, s)| d.line_of(s.head)).ok_or("No line is selected")?;
        match (b.lines.get(line), &b.root) {
            (Some(Some(c)), _) if !c.is_committed() => Err("This line is not committed"),
            (Some(Some(c)), Some(root)) => Ok((c.sha.clone(), root.clone())),
            _ => Err("This line is not annotated yet"),
        }
    }

    /// The caret line's annotation in the active tab (the status line's commit).
    pub fn caret_annotation(&self) -> Option<crate::blame::Annotation<'_>> {
        let t = self.state.active()?;
        let b = t.blame.as_ref()?; // nothing more while annotations are off
        let line = t.doc.as_ref().zip(t.selection).map(|(d, s)| d.line_of(s.head))?;
        crate::blame::annotation(b, line)
    }

    /// The commit popup's text, the same in both frontends.
    pub fn commit_text(&self) -> Option<String> {
        self.state.commit_popup().map(crate::blame::CommitPopup::text)
    }

    /// Lenz Diff on the commit the commit commands act on, against its first parent (or the empty tree for a root
    /// commit), started from `exe`'s folder: the window app, or the terminal app when `terminal`.
    pub fn commit_diff_launch(&self, exe: &Path, terminal: bool) -> Result<lenz_content::open::Launch, String> {
        let (sha, root) = self.commit_target().map_err(|e| format!("Show Diff: {e}"))?;
        let program = lenz_diff(exe, terminal, "Show Diff")?;
        // ponytail: one quick `git rev-parse` on the UI thread
        let base = lenz_content::blame::parent_or_empty_tree(&root, &sha).map_err(|e| format!("Show Diff: {e}"))?;
        Ok(lenz_content::open::Launch { program, args: vec![base, sha], cwd: Some(root) })
    }


    /// The active tab, when its file is on disk for another app to open.
    fn external_tab(&self) -> Result<&Tab, &'static str> {
        match self.state.active() {
            None => Err("No file is open"),
            Some(t) if t.retrieved.is_some() => Err("Source from a language server has no file to open"),
            Some(t) if t.unavailable.is_some() => Err("The file is unavailable"),
            Some(t) => Ok(t),
        }
    }

    /// Open in Editor (`default_app` false: the user's editor at the caret, from the environment, else `configured`,
    /// the settings file's) or Open with Default App; the adapter spawns the launch and shows its toast.
    pub fn editor_plan(&self, host: &lenz_content::open::Host, configured: Option<&str>, default_app: bool) -> Result<lenz_content::open::Plan, &'static str> {
        let t = self.external_tab()?;
        let at = t.doc.as_ref().zip(t.selection).map(|(d, s)| {
            let line = d.line_of(s.head);
            let before = d.text().get(d.line_range(line).start..s.head).unwrap_or_default();
            (line + 1, before.chars().count() + 1)
        });
        let target = lenz_content::open::Target {
            file: &t.source,
            // Images and other files without text go to the system.
            binary: t.doc.is_none(),
            line: Some(at.map_or(1, |a| a.0)),
            column: at.map(|a| a.1),
            cwd: self.state.project.as_ref().map(|p| p.root()),
        };
        Ok(lenz_content::open::plan_with(host, &target, configured, default_app))
    }

    /// The left side of Compare With: the explorer row `tree_row` (its stable path) when the explorer has focus,
    /// else the active file.
    pub fn compare_source(&self, tree_row: Option<&str>) -> Result<PathBuf, &'static str> {
        let Some(row) = tree_row else { return self.external_tab().map(|t| t.source.clone()) };
        if row.contains("://") {
            return Err("This entry has no file on disk");
        }
        match (Path::new(row).is_absolute(), self.state.project.as_ref()) {
            (true, _) => Ok(PathBuf::from(row)),
            (false, Some(p)) => Ok(p.root().join(row)),
            (false, None) => Err("No folder is open"),
        }
    }

    /// Lenz Diff comparing `left` with `right` (two files or two folders), started from `exe`'s folder: the window
    /// app, or the terminal app when `terminal`.
    pub fn compare_launch(&self, left: &Path, right: &Path, exe: &Path, terminal: bool) -> Result<lenz_content::open::Launch, String> {
        let right = std::path::absolute(right).map_err(|e| format!("Compare With: {e}"))?;
        let metadata = |p: &Path| std::fs::metadata(p).map_err(|_| format!("Compare With: no such file or folder: {}", p.display()));
        let (l, r) = (metadata(left)?, metadata(&right)?);
        if l.is_dir() != r.is_dir() {
            return Err("Compare With: pick a file to compare with a file, or a folder with a folder".into());
        }
        if std::fs::canonicalize(left).ok() == std::fs::canonicalize(&right).ok() {
            return Err("Compare With: pick a different file or folder".into());
        }
        let program = lenz_diff(exe, terminal, "Compare With")?;
        // ponytail: non-UTF-8 paths go lossy, as in lenz_content::open
        let args = ["--no-index".into(), left.to_string_lossy().into_owned(), right.to_string_lossy().into_owned()];
        let cwd = self.state.project.as_ref().map(|p| p.root().to_path_buf());
        Ok(lenz_content::open::Launch { program, args: args.to_vec(), cwd })
    }

    /// Compare Selected (Ctrl+D): Lenz Diff on the two explorer entries marked, while both are still listed.
    pub fn compare_selected_launch(&self, exe: &Path, terminal: bool) -> Result<lenz_content::open::Launch, String> {
        // Checked now: a marked entry may have been collapsed away, deleted or renamed since.
        let listed = |m: &Arc<str>| self.state.tree_rows().any(|r| r.path == *m);
        let [a, b] = self.state.tree_marks.as_slice() else { return Err(format!("Compare Selected: {MARK_TWO}")) };
        if !listed(a) || !listed(b) {
            return Err(format!("Compare Selected: {MARK_TWO}; one marked entry is no longer listed"));
        }
        let source = |row: &str| self.compare_source(Some(row)).map_err(|e| format!("Compare Selected: {e}"));
        let (left, right) = (source(a)?, source(b)?);
        self.compare_launch(&left, &right, exe, terminal).map_err(|e| e.replacen("Compare With:", "Compare Selected:", 1))
    }

    pub fn dispatch(&mut self, input: impl Into<Input>) -> Effect {
        let input = input.into();
        // A later semantic input supersedes a cold navigation even if it leaves the visible state unchanged
        // (Home while an End seek is pending, for example). Adapter layout/metric feedback is not user input.
        if matches!(&input, Input::Command(command) if !matches!(command, Command::PrepareLayout { .. } | Command::PrepareProjection(_) | Command::CancelProjection { .. } | Command::SetTextSize { .. })) {
            self.layout_intents = Default::default();
        }
        if matches!(&input, Input::Command(Command::Open(_) | Command::OpenRetained(_) | Command::OpenAt { .. } | Command::ActivateTab(_) | Command::Close | Command::CloseTab(_) | Command::Pin(_) | Command::Back | Command::Forward | Command::Select(_) | Command::MoveCaret(_) | Command::SelectAll | Command::Scroll(_) | Command::ScrollRows(_) | Command::ScrollColumn(_) | Command::GoToLine(_) | Command::GoToPlace(_) | Command::Split(_) | Command::Unsplit | Command::FocusPane(_) | Command::MoveTab { .. } | Command::SetExplorer(_) | Command::ToggleDir(_) | Command::ActivateTree { .. } | Command::SetLayout(_) | Command::SetSoftWrap(_))) { self.restore_workspace = false; }
        let reveal_selection = matches!(&input,
            Input::Command(Command::MoveCaret(_) | Command::Extend(_) | Command::ExtendSelection | Command::ShrinkSelection)
            | Input::Completion(Completion::SelectionSteps { .. }))
            && !matches!(&input, Input::Completion(Completion::SelectionSteps { request, .. }) if self.expanding.is_some_and(|e| e.3 == *request && !e.4));
        let mut effect = match input {
            Input::Command(c) => self.command(c),
            Input::Completion(Completion::Semantic(semantics::Event::Tick)) => self.tick(),
            Input::Completion(c) => self.complete(c),
        };
        if reveal_selection && effect.repaint {
            self.keep_selection_visible();
        }
        // Language servers follow the documents shown (cheap: a pass over the tabs).
        self.sync_semantics();
        self.sync_repo();
        // Search rows retain their identity while background server availability changes.
        if let Some(n) = &self.state.navigation {
            let updates: Vec<_> = n.rows.iter().enumerate().filter_map(|(i, row)| match row.target {
                crate::navigation::Target::Action(id) => Some((i, self.disabled(id), self.on(id))),
                crate::navigation::Target::Recent {ref path,..} => Some((i, self.state.missing.contains(path).then_some("File missing"), None)),
                crate::navigation::Target::Open {..} => None,
            }).collect();
            if let Some(n) = &mut self.state.navigation {
                for (i, disabled, on) in updates {
                    if let Some(row) = n.rows.get_mut(i) {
                        effect.repaint |= row.disabled != disabled || row.on != on;
                        row.disabled = disabled; row.on = on;
                    }
                }
            }
        }
        // Documentation whose tab is no longer shown as it was is closed, not kept for its return.
        if self.state.documentation.is_some() && self.state.documentation().is_none() {
            self.state.documentation = None;
        }
        for intent in &mut self.layout_intents {
            if intent.as_ref().is_some_and(|intent| !layout_intent_current(&self.state, intent)) { *intent = None; }
        }
        self.layouts.invalidate(|r| layout_current(&self.state, r));
        self.layouts.pump(&self.jobs, &self.tx);
        self.projections.invalidate(|r| projection_current(&self.state, r));
        self.projections.pump(&self.jobs, &self.tx);
        effect
    }

    fn admit_current(&mut self, id: TabId) {
        let changed = self.state.admit_current(id);
        for id in changed {
            if let Some(doc) = self.state.tabs.iter().find(|tab| tab.id == id).and_then(|tab| tab.doc.clone()) {
                if self.is_shown(id) {
                    self.mark_shown(id, doc.version());
                    self.decorate(&doc);
                }
                if let Some(query) = self.state.tabs.iter().find(|tab| tab.id == id).and_then(|tab| tab.find.as_ref()).map(|find| find.query.clone()) {
                    self.find_in(id, query, false);
                }
                self.annotate(id);
            }
        }
        self.release_current();
    }

    fn release_current(&mut self) {
        self.state.sources.buffers.retain(|id, entry| {
            let loaded = self.state.tabs.iter().any(|tab| tab.buffer == Some(*id) && tab.doc.is_some());
            let cached = entry.document.upgrade().is_some_and(|doc| self.cache.contains(doc.source()));
            if !loaded && !cached { entry.snapshot = None; }
            loaded || cached || self.state.tabs.iter().any(|tab| tab.buffer == Some(*id))
        });
        self.state.sources.aliases.retain(|_, id| self.state.sources.buffers.contains_key(id));
    }

    fn defer_layout(&mut self, tab: TabId, work: crate::layout::Work, purpose: LayoutPurpose) -> Effect {
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        let pane = view.tab().pane();
        let request = crate::layout::Request { pane, tab, work };
        self.layout_intents[pane as usize] = Some(LayoutIntent { request: request.clone(), purpose,
            selection: view.tab().selection(), top: (view.tab().top_line(), view.tab().top_row()), visible: view.visible() });
        self.prepare_layout(request);
        Effect::default()
    }

    fn prepare_layout(&mut self, request: crate::layout::Request) {
        if layout_current(&self.state, &request) && let Some(snapshot) = self.state.view(request.tab).map(|view| view.snapshot().clone()) {
            self.layouts.prepare(request, snapshot);
        }
    }

    fn command(&mut self, command: Command) -> Effect {
        let repaint = Effect { repaint: true, copy: None };
        match command {
            Command::PrepareLayout { pane, tab, work } => {
                if self.layout_intents.get(pane as usize).is_some_and(Option::is_some) { return Effect::default(); }
                self.prepare_layout(crate::layout::Request { pane, tab, work });
                Effect::default()
            }
            Command::PrepareProjection(request) => {
                if projection_current(&self.state, &request)
                    && let Some(snapshot) = self.state.view(request.tab).map(|v| v.snapshot().clone())
                {
                    self.projections.prepare(request, snapshot);
                }
                Effect::default()
            }
            Command::CancelProjection { pane, request } => {
                self.projections.cancel(pane, request);
                Effect::default()
            }
            Command::SearchEverywhere {scope, query} => {
                if scope == crate::navigation::Scope::Recent && self.state.navigation.as_ref().is_none_or(|n| n.scope != scope) {
                    self.command(Command::CheckPlaces);
                }
                self.state.navigation = Some(crate::navigation::Search {scope, query, rows: Vec::new(), revision: 0, pending: false, request: 0, dirty: false, matched: 0});
                self.rank_navigation();
                repaint
            }
            Command::CloseSearchEverywhere => {
                self.navigation_latest.fetch_add(1, Ordering::Relaxed);
                Effect {repaint: self.state.navigation.take().is_some(), copy: None}
            }
            Command::ForgetNavigation { revision, index } => {
                let Some(search) = self.state.navigation.as_ref().filter(|s| s.scope == crate::navigation::Scope::Recent && s.revision == revision) else { return Effect::default() };
                let Some(crate::navigation::Row {target: crate::navigation::Target::Recent {path,..},..}) = search.rows.get(index) else { return Effect::default() };
                let path = path.clone();
                self.command(Command::ForgetPlace(Place::File(path)))
            }
            Command::Open(path) => self.navigate(path, false, None, true),
            Command::OpenRetained(path) => self.navigate(path, true, None, true),
            Command::ActivateTab(id) => {
                let Some(pane) = self.state.tabs.iter().find(|t| t.id == id).map(|t| t.pane) else { return Effect::default() };
                let focused = self.focus_pane(pane);
                if self.state.active == Some(id) {
                    return Effect { repaint: focused, copy: None };
                }
                self.open_seq += 1;
                let from = self.here();
                self.activate(id);
                let to = self.here();
                if let Some(to) = to {
                    self.visit(from, to);
                }
                repaint
            }
            Command::ToggleBookmark | Command::SetMnemonic(_) => {
                let Some(mark) = self.caret_mark() else {
                    // Keys reach here without a menu's check: say why nothing happens.
                    if let Some(why) = self.disabled(CommandId::ToggleBookmark) {
                        self.state.messages.push(why.into());
                    }
                    return Effect::default();
                };
                match command {
                    Command::SetMnemonic(key) => Arc::make_mut(&mut self.state.places).set_mnemonic(mark, key),
                    _ => {
                        Arc::make_mut(&mut self.state.places).toggle(mark);
                    }
                }
                self.places_changed();
                repaint
            }
            Command::JumpMnemonic(key) => match self.state.places.mnemonic(key).cloned() {
                Some(m) => self.jump_to_mark(m),
                None => {
                    self.state.messages.push(format!("Bookmark {} is not set", key.to_ascii_uppercase()));
                    repaint
                }
            },
            Command::StepBookmark { forward } => {
                let (source, line, last) = self.state.active().and_then(|t| {
                    let doc = t.doc.as_ref()?;
                    Some((t.source.clone(), doc.line_of(t.selection.map_or(0, |s| s.head)), doc.line_count() - 1))
                }).unwrap_or((PathBuf::new(), 0, usize::MAX));
                match self.state.places.step(&source, line, last, forward).cloned() {
                    Some(m) => self.jump_to_mark(m),
                    None => Effect::default(),
                }
            }
            Command::GoToPlace(place) => match place {
                Place::File(path) => self.navigate(path, false, None, true),
                Place::Bookmark(source, line) => match self.state.places.bookmark(&source, line).cloned() {
                    Some(m) => self.jump_to_mark(m),
                    None => Effect::default(),
                },
                Place::Location(source, line) => match self.state.places.recent_locations().iter().find(|m| *m.source == *source && m.line == line).cloned() {
                    Some(m) => self.jump_to_mark(m),
                    None => Effect::default(),
                },
            },
            Command::ForgetPlace(place) => {
                let places = Arc::make_mut(&mut self.state.places);
                match &place {
                    Place::Bookmark(s, l) => places.remove_bookmark(s, *l),
                    Place::File(p) => places.forget_file(p),
                    Place::Location(s, l) => places.forget_location(s, *l),
                }
                self.places_changed();
                repaint
            }
            Command::CheckPlaces => {
                let p = &self.state.places;
                let files: std::collections::BTreeSet<PathBuf> = p.bookmarks().iter().map(|m| m.source.to_path_buf()).chain(p.recent_files().iter().cloned()).chain(p.recent_locations().iter().map(|m| m.source.to_path_buf())).collect();
                let (tx, generation) = (self.tx.clone(), self.project_generation.load(Ordering::Relaxed));
                self.jobs.submit(Priority::Background, move || {
                    let missing = files.into_iter().filter(|f| !f.exists()).collect();
                    let _ = tx.send_blocking(Completion::PlacesChecked { generation, missing });
                });
                Effect::default()
            }
            Command::CloseTab(id) => self.close_tab(id),
            Command::Close => match self.state.active {
                Some(id) => self.close_tab(id),
                None => Effect::default(),
            },
            Command::Pin(id) => match self.state.tabs.iter_mut().find(|t| t.id == id && !t.retained) {
                Some(t) => {
                    t.retained = true;
                    repaint
                }
                None => Effect::default(),
            },
            Command::Back | Command::Forward => {
                let back = command == Command::Back;
                let to = match (back, self.here()) {
                    (true, Some(here)) => self.state.history.back(here),
                    (false, Some(here)) => self.state.history.forward(here),
                    // Every tab closed: history still remembers where it was.
                    (true, None) => self.state.history.back_from_nowhere(),
                    (false, None) => self.state.history.forward_from_nowhere(),
                };
                match to {
                    Some(to) => self.navigate(to.source.clone(), false, Some(to), false),
                    None => Effect::default(),
                }
            }
            Command::Recheck => {
                for id in self.state.tabs.iter().map(|t| t.id).collect::<Vec<_>>() {
                    self.refresh(id);
                }
                if !self.watcher.available() {
                    self.rescan();
                }
                Effect::default()
            }
            Command::Search { query, scope } => {
                let (scope, pattern) = match scope {
                    SearchIn::Project => (Scope::Project, String::new()),
                    SearchIn::Matching(pattern) => (Scope::Project, pattern),
                    SearchIn::ActiveFile => match self.state.document() {
                        Some(doc) => (Scope::File(doc.source().to_owned()), String::new()),
                        None => return Effect::default(),
                    },
                };
                self.search(query, scope, pattern)
            }
            Command::ToggleResultGroup(group) => match self.state.results.as_mut().filter(|r| r.grouped()) {
                Some(r) => {
                    r.toggle(group);
                    repaint
                }
                None => Effect::default(),
            },
            Command::RevealResult(i) => match self.state.results.as_mut().filter(|r| r.grouped()) {
                Some(r) => {
                    let folded: Vec<_> = r.groups_of(i).into_iter().flatten().filter(|g| r.is_folded(g)).collect();
                    for g in folded {
                        r.collapsed.remove(&g);
                    }
                    r.rebuild_rows();
                    repaint
                }
                None => Effect::default(),
            },
            Command::ColourResults(wanted) => {
                let Some(r) = self.state.results.as_mut() else { return Effect::default() };
                if r.spans_asked.len() > search::SPANS_KEPT {
                    r.spans.clear();
                    r.spans_asked.clear();
                }
                let lines: Vec<_> = wanted.into_iter().filter(|&i| r.hits.get(i).is_some_and(|h| h.decl.is_none()) && r.spans_asked.insert(i)).map(|i| (i, r.hits[i].clone())).collect();
                if !lines.is_empty() {
                    let (id, latest, tx) = (r.id, self.search_latest.clone(), self.tx.clone());
                    self.jobs.submit(Priority::Visible, move || {
                        if latest.load(Ordering::Relaxed) != id {
                            return;
                        }
                        let spans = lines.iter().map(|(i, h)| (*i, search::snippet_spans(h))).collect();
                        let _ = tx.send_blocking(Completion::ResultSpans { id, spans });
                    });
                }
                Effect::default()
            }
            // The same query over the same scope (the same file, whichever tab is active now) and files.
            Command::Find(text) => match self.state.active {
                Some(id) => self.find_in(id, text, true),
                None => Effect::default(),
            },
            Command::FindStep { forward } => self.find_step(forward),
            Command::CloseFind => {
                if let Some(t) = self.active_mut() {
                    t.find = None;
                }
                Effect { repaint: true, ..Effect::default() }
            }
            Command::ToggleFindOption(option) => {
                self.find_options = self.find_options.toggled(option);
                match self.state.active().and_then(|t| Some((t.id, t.find.as_ref()?.query.clone()))) {
                    Some((id, text)) => self.find_in(id, text, true),
                    None => Effect { repaint: true, ..Effect::default() },
                }
            }
            Command::ListMatches => match self.state.active().and_then(|t| t.find.as_ref()).map(|f| f.query.clone()) {
                Some(query) => self.command(Command::Search { query, scope: SearchIn::ActiveFile }),
                None => Effect::default(),
            },
            Command::GoToLine(at) => self.go_to_line(at),
            Command::SetSoftWrap(on) => {
                self.state.settings.soft_wrap = on;
                self.save_settings();
                for t in &mut self.state.tabs {
                    t.set_top(t.top_line, 0);
                    if on {
                        t.column = 0;
                    }
                }
                Effect { repaint: true, copy: None }
            }
            Command::RevealInTree(rel) => {
                let changed = self.state.project.as_mut().is_some_and(|p| p.reveal(&rel) || p.reveal_folder(&rel));
                Effect { repaint: changed, copy: None }
            }
            Command::SetStickyScroll(on) => {
                self.state.settings.sticky_scroll = on;
                self.settings_changed_declarations()
            }
            Command::SetBreadcrumbs(on) => {
                self.state.settings.breadcrumbs = on;
                self.settings_changed_declarations()
            }
            Command::SetInlayTypes(on) => {
                self.state.settings.inlay_types = on;
                self.decorations_changed(Decor::Hints, on)
            }
            Command::SetInlayParameters(on) => {
                self.state.settings.inlay_parameters = on;
                self.decorations_changed(Decor::Hints, on)
            }
            Command::SetSemanticHighlighting(on) => {
                self.state.settings.semantic_highlighting = on;
                self.decorations_changed(Decor::Tokens, on)
            }
            Command::OpenAt { path, at } => {
                self.pending_goto = Some((normalized(&path), Goto::At(at)));
                let effect = self.command(Command::Open(path));
                self.apply_pending_goto();
                Effect { repaint: true, ..effect }
            }
            Command::SetTextSize { pane, cols, rows } => {
                let Some(size) = self.state.text_size.get_mut(pane as usize) else { return Effect::default() };
                let rewrap = size.0 != cols;
                *size = (cols, rows);
                if rewrap {
                    // Wrapped rows change with the width: the top line stays, from its first row.
                    self.state.tabs.iter_mut().filter(|t| t.pane == pane).for_each(|t| t.set_top(t.top_line, 0));
                }
                Effect { repaint: rewrap && self.state.settings.soft_wrap, copy: None }
            }
            Command::RerunSearch => match self.state.results.as_ref().map(|r| (r.kind, r.query.clone(), r.scope.clone())) {
                Some((ResultsKind::Text, query, scope)) => {
                    let pattern = self.state.results.as_ref().map(|r| r.pattern.clone()).unwrap_or_default();
                    self.search(query, scope, pattern)
                }
                Some((ResultsKind::Outline, _, _)) => self.outline(),
                Some((ResultsKind::Declarations, query, _)) => self.find_declarations(query),
                Some((ResultsKind::Symbols, query, _)) => self.symbols(query),
                Some((ResultsKind::Hierarchy, _, _)) => match self.state.results.as_ref().and_then(|r| r.hierarchy_direction()) {
                    Some(direction) => self.hierarchy_direction(direction),
                    None => Effect::default(),
                },
                // The same question: about the place first asked about, in the snapshot a tab holds now.
                Some((kind @ (ResultsKind::Definitions | ResultsKind::Implementations | ResultsKind::TypeDefinitions | ResultsKind::Peek | ResultsKind::Usages | ResultsKind::FileSymbols), _, _)) => {
                    let ask = Ask::of(kind).unwrap_or(Ask::Definition);
                    let asked = self.state.results.as_ref().and_then(|r| r.asked.clone());
                    let doc = asked.as_ref().and_then(|(path, _)| self.state.tabs.iter().find(|t| t.source == *path)?.doc.clone());
                    match (doc, asked) {
                        (Some(doc), Some((_, offset))) => self.ask_at(doc, offset, ask),
                        _ => {
                            self.state.messages.push(format!("{}: open the file it was asked in to ask again", ask.label()));
                            Effect { repaint: true, copy: None }
                        }
                    }
                }
                None => Effect::default(),
            },
            Command::CloseResults => {
                let open = self.state.results.take().is_some();
                self.search_latest.fetch_add(1, Ordering::Relaxed); // stops a running search
                self.highlight_preview(); // and colouring its preview
                Effect { repaint: open, copy: None }
            }
            Command::Outline => self.outline(),
            Command::FindDeclarations(query) => self.find_declarations(query),
            Command::SelectResult(i) => self.select_result(i),
            Command::OpenResult(i) => self.validate_result(i),
            Command::ClampColumn { pane, tab, identity, cols, max } => {
                let valid = self.state.text_size.get(pane as usize).is_some_and(|size| size.0 == cols)
                    && self.state.shown(pane).is_some_and(|t| t.id() == tab)
                    && self.state.view(tab).is_some_and(|view| identity.matches(view.snapshot(), view.inlay_hints()));
                if !valid { return Effect::default(); }
                let max = if self.state.shown(pane).is_some_and(|t| self.state.wrap_cols(t).is_some()) { 0 } else { max };
                match self.state.tabs.iter_mut().find(|t| t.id == tab) {
                    Some(t) if t.column > max => { t.column = max; repaint }
                    _ => Effect::default(),
                }
            }
            Command::ScrollColumn(_) if self.state.active().is_some_and(|t| self.state.wrap_cols(t).is_some()) => Effect::default(),
            Command::ScrollColumn(column) => match self.active_mut() {
                Some(t) if t.column != column => {
                    t.column = column;
                    repaint
                }
                _ => Effect::default(),
            },
            Command::SaveWorkspace => { self.checkpoint_workspace(); self.places_saver.flush(); Effect::default() }
            Command::SetExplorer(explorer) => { self.state.explorer = explorer; repaint }
            Command::OpenRoot(root) => {
                self.state.tree_marks.clear();
                self.open_root(root);
                repaint
            }
            Command::ActivateTree { path, retain } => self.activate_tree(path, retain),
            Command::ToggleDir(dir) => {
                if self.state.libraries.rows().iter().any(|r| r.path.as_ref() == dir) {
                    let requests = self.state.libraries.toggle(&dir, self.state.settings.folders_first);
                    self.read_libraries(requests);
                    return repaint;
                }
                let changed = self.state.project.as_mut().is_some_and(|p| p.toggle(&dir));
                Effect { repaint: changed, copy: None }
            }
            Command::Locate => {
                if let Some(path) = self.state.active().map(|t| t.source.clone()) && self.state.libraries.contains(&path) {
                    let requests = self.state.libraries.reveal(&path, self.state.settings.folders_first);
                    self.read_libraries(requests);
                    return repaint;
                }
                let rel = self.state.active_in_tree();
                let changed = rel.zip(self.state.project.as_mut()).is_some_and(|(rel, p)| p.reveal(&rel));
                Effect { repaint: changed, copy: None }
            }
            Command::SetFoldersFirst(on) => {
                if self.state.settings.folders_first == on {
                    return Effect::default();
                }
                self.state.settings.folders_first = on;
                self.state.libraries.rebuild(on);
                if let Some(p) = &mut self.state.project {
                    p.set_folders_first(on);
                }
                self.save_settings();
                Effect { repaint: true, copy: None }
            }
            Command::Split(split) => {
                let was = self.state.settings.split.replace(split);
                self.save_settings();
                if was.is_none() && let Some(t) = self.state.active() {
                    // The new pane shows the active source too, at the same place, sharing its snapshot.
                    let pane = 1 - self.state.focused;
                    self.next_tab += 1;
                    let copy = t.duplicate(TabId(self.next_tab), pane);
                    self.state.other = Some(copy.id);
                    self.other_current.store(copy.version(), Ordering::Relaxed);
                    let id = copy.id;
                    self.state.tabs.push(copy);
                    self.focus_pane(pane);
                    self.annotate(id);
                }
                repaint
            }
            Command::Unsplit => {
                if self.state.settings.split.take().is_none() {
                    return Effect::default();
                }
                let (focused, away) = (self.state.focused, 1 - self.state.focused);
                let other = self.state.other.take();
                if other.is_some() {
                    self.other_current.store(0, Ordering::Relaxed);
                }
                // From an empty pane, the other pane's file stays shown; else it is let go.
                let shown = other.filter(|_| self.state.active.is_none());
                if let Some(id) = other.filter(|_| shown.is_none()) {
                    self.release(id);
                }
                // A source open in both panes keeps the focused pane's tab.
                let twins: Vec<TabId> = self.state.tabs.iter().filter(|t| t.pane == away && self.state.tabs.iter().any(|u| u.pane == focused && u.source == t.source)).map(|t| t.id).collect();
                self.state.tabs.retain(|t| !twins.contains(&t.id));
                for t in &mut self.state.tabs {
                    t.pane = 0;
                }
                self.state.focused = 0;
                if let Some(id) = shown {
                    self.activate(id);
                }
                self.save_settings();
                self.rewatch();
                repaint
            }
            Command::FocusPane(pane) => Effect { repaint: self.focus_pane(pane), copy: None },
            Command::MoveTab { tab, pane, index } => self.move_tab(tab, pane, index),
            Command::SetLayout(layout) => {
                if self.state.settings.layout != layout {
                    self.state.settings.layout = layout;
                    self.save_settings();
                }
                repaint
            }
            Command::FindFile(query) => {
                let Some(p) = &mut self.state.project else { return Effect::default() };
                if p.finder.as_ref().is_some_and(|f| f.query == query) {
                    return Effect::default();
                }
                p.finder = Some(Finder { query, hits: Vec::new(), matched: 0, searched: 0, ready: false });
                self.rank();
                repaint
            }
            Command::CloseFinder => {
                let Some(p) = &mut self.state.project else { return Effect::default() };
                let open = p.finder.take().is_some();
                self.query_latest.fetch_add(1, Ordering::Relaxed); // stops a running ranking
                Effect { repaint: open, copy: None }
            }
            Command::Reload if self.disabled(CommandId::Reload).is_some() => Effect::default(),
            Command::Reload => match (self.state.active, self.state.document().is_some()) {
                (Some(id), true) => {
                    // Blamed again even when the bytes are the same: commits may have changed.
                    if let Some(b) = self.tab_mut(id).and_then(|t| t.blame.as_mut()) {
                        b.stamp = None;
                    }
                    self.load(id, true);
                    repaint
                }
                _ => Effect::default(),
            },
            Command::Select(s) => {
                let Some(doc) = self.state.current() else { return Effect::default() };
                let Some(t) = self.active_mut() else { return Effect::default() };
                let clamp = |b: usize| doc.floor_char_boundary(b);
                let s = Selection { anchor: clamp(s.anchor), head: clamp(s.head) };
                let changed = t.selection != Some(s);
                t.selection = Some(s);
                t.approximate = false;
                Effect { repaint: changed, copy: None }
            }
            c @ (Command::OpenBranches | Command::OpenWorktrees | Command::OpenRevisions) => {
                let (worktrees_only, revisions_only) = (matches!(c, Command::OpenWorktrees), matches!(c, Command::OpenRevisions));
                let id = match c { Command::OpenWorktrees => CommandId::CompareWithWorktree, Command::OpenRevisions => CommandId::CompareWithRevision, _ => CommandId::GitBranches };
                if self.disabled(id).is_some() {
                    return Effect::default();
                }
                let Some(root) = self.state.repo.as_ref().map(|r| r.root.clone()) else { return Effect::default() };
                self.state.branches = Some(crate::branches::BranchesView { loading: true, worktrees_only, revisions_only, ..Default::default() });
                self.branches_requests += 1;
                let (request, tx) = (self.branches_requests, self.tx.clone());
                self.jobs.submit(Priority::Visible, move || {
                    // Changed paths are counted only when the Worktrees section shows (more than one).
                    // Compare with Worktree… shows neither the refs nor the current worktree.
                    let refs = if worktrees_only { Ok(Default::default()) } else { lenz_content::branches::refs(&root) };
                    let result = refs.and_then(|r| {
                        // Compare with Branch, Tag or Revision… lists no worktrees.
                        let mut worktrees = if revisions_only { Vec::new() } else { lenz_content::branches::worktree_list(&root)? };
                        match worktrees.len() > 1 {
                            true => worktrees.iter_mut().filter(|w| !(worktrees_only && w.current)).for_each(|w| lenz_content::branches::count_changes(std::slice::from_mut(w))),
                            false => worktrees.iter_mut().for_each(|w| w.dirty = None),
                        }
                        Ok((r, worktrees))
                    });
                    let _ = tx.send_blocking(Completion::Branches { request, root, result });
                });
                repaint
            }
            Command::CloseBranches => Effect { repaint: self.state.branches.take().is_some(), copy: None },
            Command::MarkTreeRow { path, selected } => {
                if path.contains("://") {
                    self.state.messages.push("Compare Selected: this entry has no file on disk".into());
                    return repaint;
                }
                let selected = selected.filter(|s| !s.contains("://"));
                let marks = &mut self.state.tree_marks;
                if let Some(at) = marks.iter().position(|m| *m == path) {
                    marks.remove(at);
                } else {
                    // The first Ctrl+click keeps the selected entry too, as a selection would.
                    if marks.is_empty() && let Some(sel) = selected.filter(|s| *s != path) {
                        marks.push(sel);
                    }
                    marks.push(path);
                    if marks.len() > 2 {
                        marks.remove(0);
                    }
                }
                repaint
            }
            Command::ClearTreeMarks => Effect { repaint: !std::mem::take(&mut self.state.tree_marks).is_empty(), copy: None },
            Command::Checkout(target) => self.retry(crate::branches::Retry::Checkout(target), lenz_content::branches::Mode::Normal),
            Command::CheckoutChoice(choice) => {
                use crate::branches::CheckoutChoice as C;
                let Some(p) = self.state.checkout_problem.clone() else { return Effect::default() };
                if !self.checkout_choices().iter().any(|c| c.1 == choice) {
                    return Effect::default();
                }
                match choice {
                    C::Cancel => self.state.checkout_problem = None,
                    C::Smart => {
                        self.state.checkout_problem = None;
                        return self.retry(p.retry, lenz_content::branches::Mode::Smart);
                    }
                    C::Force => {
                        let Some(root) = self.state.repo.as_ref().map(|r| r.root.clone()) else { return Effect::default() };
                        match lenz_content::branches::discarded_by_force(&root) {
                            Ok(files) => self.state.checkout_problem = Some(crate::branches::CheckoutProblem { discards: Some(files), ..p }),
                            Err(e) => {
                                self.state.checkout_problem = None;
                                self.state.messages.push(e);
                            }
                        }
                    }
                    C::ConfirmForce => {
                        self.state.checkout_problem = None;
                        return self.retry(p.retry, lenz_content::branches::Mode::Force);
                    }
                }
                repaint
            }
            Command::NewBranch { name, from, checkout: true } => self.retry(crate::branches::Retry::NewBranch { name: name.trim().to_string(), from }, lenz_content::branches::Mode::Normal),
            Command::NewBranch { name, from, checkout: false } => {
                let Some(root) = self.writable_repo() else { return repaint };
                let request = self.begin_checkout();
                let tx = self.tx.clone();
                self.jobs.submit(Priority::Visible, move || {
                    let name = name.trim().to_string();
                    let result = lenz_content::branches::create_branch(&root, &name, from.as_deref(), false, lenz_content::branches::Mode::Normal);
                    let _ = tx.send_blocking(Completion::CheckedOut { request, root, label: name, made: true, result });
                });
                repaint
            }
            Command::OpenWorktree(path) => self.command(Command::OpenRoot(path)),
            Command::SetAnnotate(on) => {
                if on && self.disabled(CommandId::AnnotateBlame).is_some() {
                    return Effect::default();
                }
                let Some(t) = self.active_mut() else { return Effect::default() };
                if on == t.blame.is_some() {
                    return Effect::default();
                }
                t.blame = on.then(crate::blame::Blame::new);
                let id = t.id;
                self.annotate(id);
                repaint
            }
            Command::ShowCommitAt { tab, line } => {
                let Some(t) = self.tab_mut(tab) else { return Effect::default() };
                let (Some(doc), Some(b)) = (t.doc.clone(), t.blame.as_ref()) else { return Effect::default() };
                let (Some(Some(commit)), Some(root)) = (b.lines.get(line), b.root.clone()) else { return Effect::default() };
                if !commit.is_committed() {
                    return Effect::default();
                }
                let sha = commit.sha.clone();
                let start = doc.line_range(line.min(doc.line_count() - 1)).start;
                t.selection = Some(Selection { anchor: start, head: start });
                self.commit_requests += 1;
                let (request, tx) = (self.commit_requests, self.tx.clone());
                self.state.commit = Some(crate::blame::CommitPopup { tab, sha: sha.clone(), root: root.clone(), details: None, request });
                self.jobs.submit(Priority::Visible, move || {
                    let _ = tx.send_blocking(Completion::CommitLoaded { request, result: lenz_content::blame::commit_details(&root, &sha) });
                });
                repaint
            }
            Command::CloseCommit => Effect { repaint: self.state.commit.take().is_some(), copy: None },
            Command::CopyRevision => match self.commit_target() {
                Ok((sha, _)) => {
                    self.state.messages.push(format!("Copied {}", sha.get(..7).unwrap_or(&sha)));
                    Effect { repaint: true, copy: Some(sha) }
                }
                Err(_) => Effect::default(),
            },
            Command::ClickSelect { at, clicks } => {
                let Some(doc) = self.state.current() else { return Effect::default() };
                let at = doc.floor_char_boundary(at);
                let s = match clicks {
                    0 | 1 => Selection { anchor: at, head: at },
                    _ => {
                        let word = doc.word_at(at);
                        Selection { anchor: word.start, head: word.end }
                    }
                };
                let effect = self.command(Command::Select(s));
                match clicks {
                    0..=2 => effect,
                    _ => {
                        self.click_extend = true;
                        Effect { repaint: effect.repaint | self.command(Command::ExtendSelection).repaint, copy: None }
                    }
                }
            }
            Command::MoveCaret(motion) => {
                if matches!(self.state.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return Effect::default(); }
                let cols = self.state.active().and_then(|t| self.state.wrap_cols(t));
                let Some(doc) = self.state.current() else { return Effect::default() };
                let wrapped = if let Some(cols) = cols.filter(|_| matches!(motion, Motion::Up | Motion::Down)) {
                    let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id())) else { return Effect::default() };
                    match layout_moved(&view, view.tab().selection().map_or(0, |selection| selection.head), motion == Motion::Down, cols) {
                        crate::layout::Query::Ready(head) => Some(head),
                        crate::layout::Query::Pending(work) => return self.defer_layout(view.tab().id(), work, LayoutPurpose::Command(Command::MoveCaret(motion))),
                    }
                } else { None };
                let Some(t) = self.active_mut() else { return Effect::default() };
                if doc.source_meta().image().is_some() && !t.image_source { return Effect::default(); }
                let s = t.selection.unwrap_or(Selection {anchor:0,head:0});
                let visible = t.visible();
                let line = doc.line_of(s.head);
                let range = doc.reader_line_range(line);
                let head = match motion {
                    Motion::Left if s.anchor != s.head => s.anchor.min(s.head),
                    Motion::Right if s.anchor != s.head => s.anchor.max(s.head),
                    Motion::Left if s.head <= range.start && line > 0 => doc.reader_line_range(visible.step(line,-1)).end,
                    Motion::Right if s.head >= range.end && visible.step(line,1) > line => doc.reader_line_range(visible.step(line,1)).start,
                    Motion::Up | Motion::Down => wrapped.unwrap_or_else(|| moved(&doc, &visible, s.head, motion)),
                    _ => moved(&doc, &visible, s.head, motion),
                };
                let head = if visible.hidden(doc.line_of(head)) { doc.reader_line_range(visible.shown(doc.line_of(head))).end } else { head };
                t.selection = Some(Selection {anchor:head,head});
                t.approximate = false;
                t.show_head();
                repaint
            }
            Command::Extend(motion) => {
                let cols = self.state.active().and_then(|t| self.state.wrap_cols(t)).filter(|_| matches!(motion, Motion::Up | Motion::Down));
                let Some(doc) = self.state.current() else { return Effect::default() };
                let wrapped = if let Some(cols) = cols {
                    let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id())) else { return Effect::default() };
                    match layout_moved(&view, view.tab().selection().map_or(0, |selection| selection.head), motion == Motion::Down, cols) {
                        crate::layout::Query::Ready(head) => Some(head),
                        crate::layout::Query::Pending(work) => return self.defer_layout(view.tab().id(), work, LayoutPurpose::Command(Command::Extend(motion))),
                    }
                } else { None };
                let Some(t) = self.active_mut() else { return Effect::default() };
                let s = t.selection.unwrap_or(Selection { anchor: 0, head: 0 });
                let head = wrapped.unwrap_or_else(|| moved(&doc, &t.visible(), s.head, motion));
                let line = doc.line_of(head);
                // Keep the head on screen as far as the core knows the view: at or below the top line, and below the
                // sticky lines.
                t.top_line = t.top_line.min(line);
                t.selection = Some(Selection { head, ..s });
                repaint
            }
            Command::ExtendSelection if self.disabled(CommandId::ExtendSelection).is_some() => Effect::default(),
            Command::ShrinkSelection => {
                let Some(t) = self.active_mut() else { return Effect::default() };
                // Only while the expansion's last selection is still the tab's.
                let Some(e) = t.expansion.as_mut().filter(|e| Some(e.current()) == t.selection) else { return Effect::default() };
                match e.shrink() {
                    Some(s) => {
                        t.selection = Some(s);
                        t.show_head();
                        repaint
                    }
                    None => Effect::default(),
                }
            }
            Command::ExtendSelection => {
                let reveal = !std::mem::take(&mut self.click_extend);
                let Some(t) = self.active_mut() else { return Effect::default() };
                let Some(doc) = t.doc.clone() else { return Effect::default() };
                let from = t.selection.unwrap_or(Selection { anchor: 0, head: 0 });
                // The current expansion goes on while its last selection is still the tab's.
                if let Some(e) = t.expansion.as_mut().filter(|e| e.version == doc.version() && e.current() == from) {
                    return match e.extend() {
                        Some(next) => {
                            t.selection = Some(next);
                            if reveal {
                                t.show_head();
                            }
                            repaint
                        }
                        None => Effect::default(),
                    };
                }
                let id = t.id;
                match doc.language().filter(|_| !doc.is_large()) {
                    None => self.start_expansion(id, doc.version(), from, crate::expand::fallback_steps(&doc, from.range()), reveal),
                    // Already parsing for this selection: applied once when the steps arrive.
                    Some(_) if self.expanding.is_some_and(|(tab, v, s, ..)| tab == id && v == doc.version() && s == from) => Effect::default(),
                    Some(lang) => {
                        let request = self.expand_latest.fetch_add(1, Ordering::Relaxed) + 1;
                        self.expanding = Some((id, doc.version(), from, request, reveal));
                        let (latest, tx, text, range) = (self.expand_latest.clone(), self.tx.clone(), doc.text().clone(), from.range());
                        self.jobs.submit(Priority::Visible, move || {
                            let current = latest.clone();
                            let steps = highlight::selection_steps_while(lang, text, range, move || current.load(Ordering::Relaxed) == request);
                            let _ = tx.send_blocking(Completion::SelectionSteps { request, steps });
                        });
                        Effect::default()
                    }
                }
            }
            Command::SelectAll => {
                let Some(doc) = self.state.current() else { return Effect::default() };
                let Some(t) = self.active_mut() else { return Effect::default() };
                t.selection = Some(Selection { anchor: 0, head: doc.len_bytes() });
                repaint
            }
            Command::CopyPath { line } => {
                let Some(t) = self.state.active() else { return Effect::default() };
                let path = match self.state.project.as_ref().and_then(|p| t.source.strip_prefix(p.root()).ok()) {
                    Some(relative) => relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"),
                    None => t.source.to_string_lossy().into_owned(),
                };
                let at = self.state.view(t.id).map_or(0, |view| t.selection.map_or(0, |s| view.snapshot().line_of(s.head)) + 1);
                Effect { repaint: false, copy: Some(if line { format!("{path}:{at}") } else { path }) }
            }
            Command::CopyText(text) => Effect { repaint: false, copy: (!text.is_empty()).then_some(text) },
            Command::Notice(text) => {
                self.state.messages.push(text);
                Effect { repaint: true, copy: None }
            }
            Command::Copy => {
                let Some(view) = self.state.active().and_then(|t| self.state.view(t.id)) else { return Effect::default() };
                let Some(s) = view.tab().selection() else { return Effect::default() };
                let doc = view.snapshot();
                // No selection: the caret's visible line with its original ending.
                let range = if s.range().is_empty() { doc.line_with_ending(doc.line_of(s.head)).unwrap_or(0..0) } else { s.range() };
                Effect { repaint: false, copy: (!range.is_empty()).then(|| doc.copy_range(range)).flatten() }
            }
            Command::Scroll(top) => {
                let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id())) else { return Effect::default() };
                let line = view.visible().shown(top.min(view.line_count().saturating_sub(1)));
                let row = if top == usize::MAX && let Some(cols) = self.state.wrap_cols(view.tab()) {
                    match view.layout_last(line, Some(cols)) {
                        crate::layout::Query::Ready(last) => last.map_or(0, |row| row.index),
                        crate::layout::Query::Pending(work) => return self.defer_layout(view.tab().id(), work, LayoutPurpose::Command(Command::Scroll(top))),
                    }
                } else { 0 };
                let Some(tab) = self.active_mut() else { return Effect::default() };
                let changed = (tab.top_line(), tab.top_row()) != (line, row) || tab.approximate;
                tab.set_top(line, row); tab.approximate = false;
                Effect { repaint: changed, copy: None }
            }
            Command::ScrollRows(rows) => {
                let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id())) else { return Effect::default() };
                let (top, row) = match self.state.wrap_cols(view.tab()) {
                    Some(cols) => match view.layout_step((view.tab().top_line(), view.tab().top_row()), rows, cols) {
                        crate::layout::Query::Ready(top) => top,
                        crate::layout::Query::Pending(work) => return self.defer_layout(view.tab().id(), work, LayoutPurpose::Command(Command::ScrollRows(rows))),
                    },
                    None => (view.visible().step(view.tab().top_line(), rows), 0),
                };
                let Some(t) = self.active_mut() else { return Effect::default() };
                let changed = (t.top_line, t.top_row()) != (top, row) || t.approximate;
                t.set_top(top, row);
                t.approximate = false;
                Effect { repaint: changed, copy: None }
            }
            Command::Collapse | Command::Expand | Command::CollapseAll | Command::ExpandAll | Command::CollapseCommentsImports | Command::ToggleFold(_)
                if self.fold_unavailable().is_some() =>
            {
                Effect::default()
            }
            Command::Collapse => {
                let Some(t) = self.active_mut() else { return Effect::default() };
                let (Some(doc), Some(folds)) = (t.doc.clone(), t.folds.clone()) else { return Effect::default() };
                // The caret's row (a selection's head may be on a hidden line).
                let line = t.visible().shown(t.selection.map_or(t.top_line, |s| doc.line_of(s.head)));
                // The innermost open region holding the line: regions are sorted by first line, so the last that holds it.
                let open = folds.regions.iter().rev().find(|f| f.first as usize <= line && line <= f.last as usize && t.collapsed.get(f.first as usize) != Some(f.kind));
                let Some(f) = open.copied() else { return Effect::default() };
                t.collapsed.insert(f.first as usize, f.kind);
                t.leave_hidden();
                repaint
            }
            Command::Expand => {
                let Some(t) = self.active_mut() else { return Effect::default() };
                let Some(doc) = t.doc.clone() else { return Effect::default() };
                let line = t.visible().shown(t.selection.map_or(t.top_line, |s| doc.line_of(s.head)));
                Effect { repaint: t.collapsed.remove(line), copy: None }
            }
            Command::CollapseAll | Command::CollapseCommentsImports => {
                let only_runs = command == Command::CollapseCommentsImports;
                let Some(t) = self.active_mut() else { return Effect::default() };
                let Some(folds) = t.folds.clone() else { return Effect::default() };
                for f in folds.regions.iter().filter(|f| !only_runs || f.kind != FoldKind::Block) {
                    t.collapsed.insert(f.first as usize, f.kind);
                }
                t.leave_hidden();
                repaint
            }
            Command::ExpandAll => {
                let Some(t) = self.active_mut() else { return Effect::default() };
                Effect { repaint: t.collapsed.clear(), copy: None }
            }
            Command::ToggleFold(line) => {
                let Some(t) = self.active_mut() else { return Effect::default() };
                let Some((f, collapsed)) = t.fold_at(line) else { return Effect::default() };
                if collapsed {
                    t.collapsed.remove(line);
                } else {
                    t.collapsed.insert(line, f.kind);
                    t.leave_hidden();
                }
                repaint
            }
            Command::Theme(c) => {
                if matches!(c, ThemeCommand::Select(_)) { self.theme_preview = None; }
                let effect = self.themes.execute(c);
                self.theme_effect(effect)
            }
            Command::PreviewTheme(id) => {
                self.theme_preview = id;
                self.theme_effect(lenz_content::theme::ThemeEffect { repaint: true, notice: None })
            }
            Command::Definition { peek } => self.ask(if peek { Ask::Peek } else { Ask::Definition }),
            Command::HighlightAt => self.occurrences(),
            Command::Implementation => self.ask(Ask::Implementation),
            Command::Documentation => {
                let (Some(t), Some(at)) = (self.state.active(), self.state.selection()) else { return Effect::default() };
                let (id, name) = (t.id, t.name_at(at.head));
                match name {
                    Some(name) => self.document(id, name, DocTrigger::Caret),
                    None => {
                        self.state.messages.push("Quick documentation: No documentation here".into());
                        Effect { repaint: true, copy: None }
                    }
                }
            }
            Command::DocumentationAt { tab, offset } => {
                let Some(t) = self.state.tabs.iter().find(|t| t.id == tab) else { return Effect::default() };
                let Some(doc) = t.doc.as_ref() else { return Effect::default() };
                let line = doc.line_of(offset.min(doc.text().len()));
                let start = doc.line_range(line).start;
                match t.linkable(line, offset - start) {
                    Some(r) => self.document(tab, start + r.start..start + r.end, DocTrigger::Pointer),
                    None => Effect::default(),
                }
            }
            Command::CloseDocumentation => {
                self.pending_doc = None;
                self.state.semantics.cancel_lane(semantics::Lane::Hover);
                Effect { repaint: self.state.documentation.take().is_some(), copy: None }
            }
            Command::SetDocumentationOnHover(on) => {
                self.state.settings.documentation_on_hover = on;
                self.save_settings();
                Effect { repaint: true, copy: None }
            }
            Command::SetFileIcons(on) => {
                self.state.settings.file_icons = on;
                self.save_settings();
                Effect { repaint: true, copy: None }
            }
            Command::CallHierarchy(direction) => self.ask(Ask::CallHierarchy(direction)),
            Command::TypeHierarchy(direction) => self.ask(Ask::TypeHierarchy(direction)),
            Command::HierarchyDirection(direction) => self.hierarchy_direction(direction),
            Command::ToggleCall(i) => match self.state.results.as_ref().and_then(|r| r.call_nodes.get(i)).copied() {
                Some(node) => self.unfold_call(node),
                None => Effect::default(),
            },
            Command::TypeDefinition => self.ask(Ask::TypeDefinition),
            Command::Usages => self.ask(Ask::Usages),
            Command::ToggleImageSource => match self.disabled(CommandId::ImageSource) {
                None => {
                    if let Some(t) = self.active_mut() {
                        t.image_source = !t.image_source;
                    }
                    repaint
                }
                Some(_) => Effect::default(),
            },
            Command::ToggleMarkdownSource => {
                if self.disabled(CommandId::MarkdownSource).is_some() {
                    return Effect::default();
                }
                let Some(t) = self.active_mut() else { return Effect::default() };
                t.markdown_source = !t.markdown_source;
                // The source needs no preview: it and its images go (and any still being made stops) until it is
                // shown again.
                t.preview = None;
                let (source, preview) = (t.source.clone(), !t.markdown_source);
                // Remembered for this file: it opens the same way again (Back, reopening) in this session.
                if preview {
                    self.previewed.insert(source);
                } else {
                    self.previewed.remove(&source);
                }
                self.renders.fetch_add(1, Ordering::Relaxed);
                if let Some(doc) = self.state.document().cloned() {
                    self.render(&doc);
                }
                repaint
            }
            Command::FileSymbols => match self.state.document().cloned() {
                Some(doc) => self.ask_at(doc, 0, Ask::FileSymbols),
                None => Effect::default(),
            },
            Command::Symbols(query) => self.symbols(query),
            Command::OpenServerRuntime => {
                if self.disabled(CommandId::ServerRuntime).is_some() { return Effect::default(); }
                self.runtime_request += 1;
                let request = self.runtime_request;
                let Some(kind) = self.runtime_kind() else { return Effect::default() };
                let configured = semantics::runtime::configured(kind, self.state.semantics.settings());
                self.state.server_runtime = Some(semantics::runtime::Runtime { kind, configured: configured.clone(), candidates: Vec::new(), message: format!("Finding installed {} runtimes…", kind.label()) });
                let tx = self.tx.clone();
                self.jobs.submit(Priority::Visible, move || { let paths = semantics::runtime::discover(kind, configured); let _ = tx.send_blocking(Completion::ServerRuntimes { request, paths }); });
                repaint
            }
            Command::CloseServerRuntime => { self.state.server_runtime = None; repaint }
            Command::SetServerRuntime(path) => {
                if self.disabled(CommandId::ServerRuntime).is_some() { return Effect::default(); }
                let Some(kind) = self.state.server_runtime.as_ref().map(|m| m.kind).filter(|k| Some(*k) == self.runtime_kind()) else { return Effect::default() };
                self.runtime_request += 1;
                let request = self.runtime_request;
                self.state.runtime_saving = Some(request);
                if let Some(model) = &mut self.state.server_runtime { model.message = format!("Validating and saving {} runtime…", kind.label()); }
                let (file, tx) = (self.state.semantics.settings_file().to_owned(), self.tx.clone());
                self.jobs.submit(Priority::Visible, move || { let result = semantics::runtime::save(kind, &file, path); let _ = tx.send_blocking(Completion::ServerRuntimeSaved { request, kind, result }); });
                repaint
            }
            Command::RestartServer => {
                if self.state.runtime_saving.is_some() || self.state.provider_saves > 0 { return Effect::default(); }
                let Some(doc) = self.state.document().cloned() else { return Effect::default() };
                self.state.asking = None;
                self.state.semantics.cancel();
                if !self.state.semantics.restart(&doc) {
                    self.state.messages.push("Restart: no language server has started for this file yet".into());
                }
                self.synced = 0;
                repaint
            }
            Command::SetServerEnabled(enabled) => {
                if self.state.runtime_saving.is_some() || self.state.provider_saves > 0 { return Effect::default(); }
                let (Some(doc), Some(browse)) = (self.state.document(), self.state.project.as_ref().map(|p| p.root().to_owned())) else { return Effect::default() };
                let Some((provider, _)) = doc.language().and_then(semantics::provider::for_language) else { return Effect::default() };
                self.state.asking = None;
                let settings = self.state.semantics.set_enabled(provider, browse, enabled);
                self.save_provider_settings(settings);
                self.synced = 0;
                repaint
            }
            Command::CancelSemantic => {
                let asked = self.state.asking.take().is_some();
                self.state.semantics.cancel();
                Effect { repaint: asked, copy: None }
            }
            Command::SetWorkspaceRoot(root) => {
                if self.state.runtime_saving.is_some() || self.state.provider_saves > 0 { return Effect::default(); }
                let (Some(doc), Some(browse)) = (self.state.document(), self.state.project.as_ref().map(|p| p.root().to_owned())) else { return Effect::default() };
                let Some((provider, _)) = doc.language().and_then(semantics::provider::for_language) else { return Effect::default() };
                self.state.asking = None;
                let root = root.map(|r| normalized(&r));
                let settings = self.state.semantics.set_root(provider, browse, root);
                self.synced = 0;
                self.save_provider_settings(settings);
                repaint
            }
            Command::ExcludeRubyTooling => self.save_project_provider(Some(true)),
            Command::SetRubyToolingExclusions(on) => self.save_project_provider(Some(on)),
            Command::ResetProjectServerSettings => self.save_project_provider(None),
            Command::SetPersistentIndex(on) => {
                if self.state.settings.persistent_index != on {
                    self.state.settings.persistent_index = on;
                    self.save_settings();
                    self.reset_index(false);
                }
                repaint
            }
            Command::RebuildIndex => {
                if self.disabled(CommandId::RebuildIndex).is_none() { self.reset_index(true); }
                repaint
            }
        }
    }

    /// Idle servers stop; running ones have their process groups' memory sampled off the UI thread.
    fn tick(&mut self) -> Effect {
        let mut repaint = self.state.semantics.expire();
        // No repository yet, but a `.git` appeared (git init, a clone): look again. A file check, no git process.
        if self.state.repo.is_none()
            && self.state.repo_checked()
            && self.state.repo_anchor.as_ref().is_some_and(|a| a.ancestors().any(|d| d.join(".git").exists()))
            && !self.repo_git_seen
        {
            self.state.repo_anchor = None;
            self.sync_repo();
        }
        // A branch switched elsewhere (a terminal, another app): HEAD's file (or a reftable's table list) changed; it
        // is read again off the UI thread. Only a file's time is checked here.
        if let Some(r) = self.state.repo.as_mut()
            && let signal = lenz_content::branches::head_signal(&r.git_dir)
            && signal != r.signal
        {
            r.signal = signal;
            let (root, git_dir, tx) = (r.root.clone(), r.git_dir.clone(), self.tx.clone());
            self.jobs.submit(Priority::Background, move || {
                let head = lenz_content::branches::head(&root, &git_dir);
                let _ = tx.send_blocking(Completion::Head { root, head });
            });
        }
        // Symbols wait for every server asked, but not for one that never answers.
        let now = self.state.semantics.now();
        if let Some(a) = self.state.asking.as_ref().filter(|a| a.ask == Ask::Symbols && now.duration_since(a.since) >= SYMBOLS_WAIT) {
            let missing = a.remaining;
            self.state.asking = None;
            self.state.semantics.cancel();
            if let Some(r) = self.state.results.as_mut().filter(|r| r.kind == ResultsKind::Symbols) {
                r.done = true;
            }
            self.state.messages.push(format!("Symbols: {missing} language server(s) did not answer within {} s", SYMBOLS_WAIT.as_secs()));
            repaint = true;
        }
        let groups = self.state.semantics.groups();
        if !groups.is_empty() && !self.sampling {
            self.sampling = true; // one sample at a time, however busy the workers are
            let tx = self.tx.clone();
            self.jobs.submit(Priority::Background, move || {
                let samples = semantics::sample_memory(&groups);
                let _ = tx.send_blocking(Completion::Semantic(semantics::Event::Memory(samples)));
            });
        }
        Effect { repaint, copy: None }
    }

    /// Ask the active document's language server `ask` about the caret.
    fn ask(&mut self, ask: Ask) -> Effect {
        let (Some(doc), Some(at)) = (self.state.document().cloned(), self.state.selection()) else { return Effect::default() };
        // A selected name (as a jump leaves it) is asked about at its start; its end is past the name.
        self.ask_at(doc, at.range().start, ask)
    }

    /// Ask about `offset` of `doc` (a snapshot a tab holds).
    fn ask_at(&mut self, doc: Arc<Document>, offset: usize, ask: Ask) -> Effect {
        if let Ask::TypeHierarchy(direction) = ask
            && !self.state.semantics.view(&doc).is_some_and(|v| v.offers(semantics::Question::PrepareTypeHierarchy))
        {
            return self.type_hierarchy_by_syntax(doc, offset, direction);
        }
        self.synced = 0;
        self.sync_semantics(); // the server has this version before it is asked about it
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.forget_semantic_messages();
        let project = self.state.project.as_ref().map_or(0, |p| p.generation());
        match self.state.semantics.ask(&doc, offset, request, ask.question()) {
            Ok(()) => self.state.asking = Some(Asking { request, ask, at: Some((doc.source().to_owned(), offset, doc.version())), project, remaining: 1, since: self.state.semantics.now(), retrieving: None, root: None }),
            Err(why) => {
                self.state.asking = None;
                self.state.messages.push(format!("{}: {why}", ask.label()));
            }
        }
        Effect { repaint: true, copy: None }
    }

    /// A type hierarchy built from usages and syntax (the server offers none): the name at `offset` is the root.
    fn type_hierarchy_by_syntax(&mut self, doc: Arc<Document>, offset: usize, direction: Direction) -> Effect {
        self.synced = 0;
        self.sync_semantics();
        self.forget_semantic_messages();
        let label = Ask::TypeHierarchy(direction).label();
        let tab = self.state.tabs.iter().find(|t| t.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, &doc)));
        let (name, held) = (tab.and_then(|t| t.name_at(offset)), tab.and_then(|t| t.declarations.clone()));
        let Some(name) = name else {
            self.state.messages.push(format!("{label}: Place the caret on a name first"));
            return Effect { repaint: true, copy: None };
        };
        let Some(session) = self.state.semantics.session_of(&doc) else {
            let server = self.state.semantics.view(&doc).map_or("The language server", |v| v.name);
            self.state.messages.push(format!("{label}: {server} is not ready yet"));
            return Effect { repaint: true, copy: None };
        };
        let encoding = self.state.semantics.encoding(session);
        let uri = semantics::lsp::file_uri(doc.source());
        let range = serde_json::json!({ "start": semantics::lsp::position(&doc, name.start, encoding), "end": semantics::lsp::position(&doc, name.end, encoding) });
        let text = doc.text()[name.clone()].to_owned();
        let decl = held.as_deref().and_then(|d| root_kind(d, name.start));
        let root = Call { item: serde_json::json!({ "uri": uri, "selectionRange": range, "name": text }), name: text, kind: 0, decl, calls: 1, at: serde_json::json!({ "uri": uri, "range": range }) };
        let site = semantics::hit_at(doc.source(), &doc, name.start, name.end);
        self.search_seq += 1;
        let id = self.search_seq;
        self.search_latest.store(id, Ordering::Relaxed);
        let mut results = self.new_results(id, ResultsKind::Hierarchy, String::new(), Scope::Project, None, 0, false);
        results.provider = self.state.semantics.origin(session);
        results.asked = Some((doc.source().to_owned(), offset));
        results.hierarchy = Some(Hierarchy::new(root, site, direction, session, hierarchy::Source::Syntax));
        results.done = true;
        results.rebuild_hierarchy();
        self.state.results = Some(results);
        if held.is_none()
            && let Some(lang) = doc.language()
        {
            let (latest, tx, at) = (self.search_latest.clone(), self.tx.clone(), name.start);
            let doc = doc.clone();
            self.jobs.submit(Priority::Visible, move || {
                let Some(found) = highlight::declarations_while(lang, doc.text().clone(), move || latest.load(Ordering::Relaxed) == id) else { return };
                if let Some(kind) = root_kind(&found, at) {
                    let _ = tx.send_blocking(Completion::HierarchyRootKind { id, kind });
                }
            });
        }
        self.select_result(0);
        self.unfold_call(0)
    }

    /// Ask for the occurrences of the name at the caret, in the background.
    fn occurrences(&mut self) -> Effect {
        let (Some(t), Some(at)) = (self.state.active(), self.state.selection()) else { return Effect::default() };
        let (Some(doc), Some(name)) = (t.doc.clone(), t.name_at(at.head)) else { return Effect::default() };
        if self.state.occurrences().is_some_and(|o| o.ranges.iter().any(|(r, _)| *r == name)) {
            return Effect::default(); // already marked
        }
        self.sync_semantics();
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.pending_highlight = match self.state.semantics.ask(&doc, name.start, request, semantics::Question::Highlight) {
            Ok(()) => Some((request, doc.source().to_owned(), doc.version(), at.head)),
            Err(_) => None, // no server for it, or not ready: nothing is marked
        };
        Effect::default()
    }

    /// Ask about the documentation of `name` in `tab`, replacing any request before.
    fn document(&mut self, tab: TabId, name: std::ops::Range<usize>, trigger: DocTrigger) -> Effect {
        let Some(doc) = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.doc.clone()) else { return Effect::default() };
        if self.state.documentation().is_some_and(|d| d.tab == tab && d.at == name) {
            return Effect::default(); // shown already
        }
        self.sync_semantics();
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        match self.state.semantics.ask(&doc, name.start, request, semantics::Question::Hover) {
            Ok(()) => self.pending_doc = Some((request, tab, doc.version(), name, trigger)),
            Err(why) => {
                self.pending_doc = None;
                if trigger == DocTrigger::Caret {
                    self.state.messages.push(format!("Quick documentation: {why}"));
                    return Effect { repaint: true, copy: None };
                }
            }
        }
        Effect::default()
    }

    /// Documentation arrived: made into a preview off the UI thread when it answers the latest request about the
    /// tab's document as it is.
    fn documentation_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some((request, tab, version, _, trigger)) = self.pending_doc.clone().filter(|p| p.0 == reply.request) else { return Effect::default() };
        let current = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.doc.as_ref()).is_some_and(|d| d.version() == version);
        let text = reply.result.ok().filter(|_| current).and_then(|v| semantics::hover::markdown(&v));
        let Some(text) = text else {
            self.pending_doc = None;
            if trigger == DocTrigger::Caret && current {
                self.state.messages.push("Quick documentation: No documentation here".into());
                return Effect { repaint: true, copy: None };
            }
            return Effect::default();
        };
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Visible, move || {
            let preview = crate::markdown::render(text.into(), || true);
            let _ = tx.send_blocking(Completion::DocumentationReady { request, preview });
        });
        Effect::default()
    }

    /// Occurrences arrived: kept when they answer the latest request about the active document as it is, with the
    /// caret where it was.
    fn occurrences_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some((_, path, version, caret)) = self.pending_highlight.take_if(|p| p.0 == reply.request) else { return Effect::default() };
        let (Some(doc), Some(at)) = (self.state.document().cloned(), self.state.selection()) else { return Effect::default() };
        if doc.source() != path || doc.version() != version || at.head != caret {
            return Effect::default();
        }
        let Ok(serde_json::Value::Array(items)) = reply.result else { return Effect::default() };
        let ranges = items
            .iter()
            .filter_map(|i| {
                let start = semantics::lsp::offset(&doc, &i["range"]["start"], reply.encoding)?;
                let end = semantics::lsp::offset(&doc, &i["range"]["end"], reply.encoding)?;
                Some((start..end.max(start), i["kind"].as_u64() == Some(3)))
            })
            .collect();
        self.state.occurrences = Some(Occurrences { path, version, ranges });
        Effect { repaint: true, copy: None }
    }

    /// Ask the folder's language servers for workspace symbols matching `query`; results list as they arrive.
    fn symbols(&mut self, query: String) -> Effect {
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.forget_semantic_messages();
        let project = self.state.project.as_ref().map(|p| (p.generation(), p.root().to_owned()));
        match self.state.semantics.symbols(&query, request, project.as_ref().map(|(_, r)| r.as_path())) {
            Ok(asked) => {
                self.state.asking = Some(Asking { request, ask: Ask::Symbols, at: None, project: project.as_ref().map_or(0, |(g, _)| *g), remaining: asked.len(), since: self.state.semantics.now(), retrieving: None, root: None });
                self.search_seq += 1;
                let id = self.search_seq;
                self.search_latest.store(id, Ordering::Relaxed);
                let mut results = self.new_results(id, ResultsKind::Symbols, query, Scope::Project, None, 0, false);
                results.provider = Some(if asked.len() == 1 { asked[0].to_string() } else { "language servers".to_string() });
                self.state.results = Some(results);
            }
            Err(why) => {
                self.state.asking = None;
                self.state.messages.push(format!("Symbols: {why}"));
            }
        }
        Effect { repaint: true, copy: None }
    }

    fn forget_semantic_messages(&mut self) {
        let labels = [Ask::Definition, Ask::Implementation, Ask::TypeDefinition, Ask::CallHierarchy(Direction::Callers), Ask::TypeHierarchy(Direction::Subtypes), Ask::Usages, Ask::Symbols, Ask::FileSymbols].map(Ask::label);
        self.state.messages.retain(|m| !labels.iter().any(|l| m.strip_prefix(l).is_some_and(|r| r.starts_with(':'))));
    }

    /// Bring language servers in line with the documents tabs hold, then ask for the shown ones' decorations.
    fn sync_semantics(&mut self) {
        self.sync_documents();
        self.decorate_shown();
    }

    /// Decode a decoration reply off the UI thread, for the document version it was asked about (if a tab still
    /// holds it).
    fn decode(&mut self, d: semantics::Decoration) {
        let Some(doc) = self.state.tabs.iter().filter_map(|t| t.doc.as_ref()).find(|doc| doc.version() == d.version && doc.source() == d.path).cloned() else { return };
        let filter = crate::decor::HintFilter { types: self.state.settings.inlay_types, parameters: self.state.settings.inlay_parameters };
        let (generation, request) = (self.decor_generation[d.kind as usize], d.id);
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Visible, move || {
            let data = match d.kind {
                Decor::Hints => crate::decor::Decorated::Hints(Arc::new(crate::decor::decode_hints(&d.body, &doc, d.encoding, filter))),
                Decor::Tokens => crate::decor::Decorated::Tokens(Arc::new(crate::decor::decode_tokens(&d.body, &doc, d.encoding, &d.legend))),
            };
            let _ = tx.send_blocking(Completion::Decorated { version: doc.version(), generation, request, data });
        });
    }

    /// Ask for the decorations the settings want about the documents shown in the panes (text views only).
    fn decorate_shown(&mut self) {
        if !self.state.semantics.running() {
            return; // runs on every dispatch: nothing to do without a server
        }
        let s = &self.state.settings;
        let wanted: Vec<Decor> = [(s.inlay_types || s.inlay_parameters, Decor::Hints), (s.semantic_highlighting, Decor::Tokens)].into_iter().filter_map(|(on, k)| on.then_some(k)).collect();
        let docs: Vec<Arc<Document>> = [self.state.active, self.state.other].into_iter().flatten().filter_map(|id| self.state.tabs.iter().find(|t| t.id == id)).filter(|t| shows_text(t)).filter_map(|t| t.doc.clone()).collect();
        if !docs.is_empty() {
            self.state.semantics.decorate(&docs, &wanted);
        }
    }

    /// Bring language servers in line with the documents tabs hold; roots resolve off the UI thread. Nothing
    /// happens when the tabs' documents are as they were at the last synchronization.
    fn save_provider_settings(&mut self, settings: semantics::provider::Settings) {
        self.state.provider_saves += 1;
        let (file, tx) = (self.state.semantics.settings_file().to_owned(), self.tx.clone());
        self.jobs.submit(Priority::Background, move || { let ok = settings.save(&file); let _ = tx.send_blocking(Completion::ProviderSettingsSaved { ok }); });
    }

    fn save_project_provider(&mut self, excluded: Option<bool>) -> Effect {
        let reset = excluded.is_none();
        let id = if reset { CommandId::ResetProjectServerSettings } else { CommandId::ExcludeRubyTooling };
        if self.disabled(id).is_some() { return Effect::default(); }
        let Some((provider, root)) = self.state.project_provider() else { return Effect::default() };
        let (file, tx) = (self.state.semantics.settings_file().to_owned(), self.tx.clone());
        self.state.provider_saves += 1;
        self.jobs.submit(Priority::Visible, move || {
            let result = if reset { semantics::provider::reset_project(&file, semantics::provider::PROVIDERS[provider].id, &root) }
                else { semantics::provider::set_ruby_tooling_excluded(&file, &root, excluded == Some(true)) };
            let _ = tx.send_blocking(Completion::ProjectProviderSaved { provider, root, reset, result });
        });
        Effect { repaint: true, copy: None }
    }

    fn sync_documents(&mut self) {
        // Each snapshot's identity: a rename keeps the version but is a new snapshot (made while the old one lives,
        // so at another address).
        let fingerprint = self.state.tabs.iter().fold(1u64, |h, t| {
            let (version, at) = t.doc.as_ref().map_or((0, 0), |d| (d.version(), Arc::as_ptr(d) as usize as u64));
            h.wrapping_mul(1_000_003).wrapping_add(version).wrapping_mul(1_000_003).wrapping_add(at)
        }) | 1;
        if fingerprint == self.synced {
            return;
        }
        self.synced = fingerprint;
        // A source open in both panes is one document to the servers.
        let mut seen = std::collections::HashSet::new();
        let docs: Vec<Arc<Document>> = self.state.tabs.iter().filter(|t| t.retrieved.is_none() && seen.insert(&t.source)).filter_map(|t| t.doc.clone()).collect();
        let browse = self.state.project.as_ref().map(|p| p.root().to_owned());
        let (jobs, tx) = (&self.jobs, self.tx.clone());
        let mut resolve = |provider: usize, dir: PathBuf, file: PathBuf, over: Option<PathBuf>, settings: semantics::provider::Settings| {
            let (tx, browse) = (tx.clone(), browse.clone());
            jobs.submit(Priority::Background, move || {
                let root = over.or_else(|| (semantics::provider::PROVIDERS[provider].root)(&file, browse.as_deref()));
                let serve = root.as_deref().map_or(provider, |r| semantics::provider::serving(provider, r, &settings));
                let _ = tx.send_blocking(Completion::Semantic(semantics::Event::Resolved { provider, dir, root, serve }));
            });
        };
        // Only servers that run can be kept warm.
        let held: Vec<(&Path, Option<lenz_content::highlight::Language>)> = match self.state.semantics.running() {
            false => Vec::new(),
            true => {
                let mut seen = std::collections::HashSet::new();
                self.state.tabs.iter().filter(|t| t.retrieved.is_none() && seen.insert(&t.source)).map(|t| (t.source.as_path(), t.language.unwrap_or_else(|| highlight::detect(&t.source, "")))).collect()
            }
        };
        self.state.semantics.sync(&docs, &held, browse.as_deref(), &mut resolve, &self.tx);
    }

    /// A reply: check it answers the current request about the snapshot asked, in this project and server
    /// session, then locate its targets off the UI thread (reading files that are not open).
    fn semantic_reply(&mut self, reply: semantics::Reply) -> Effect {
        match reply.lane {
            semantics::Lane::Foreground => {}
            semantics::Lane::Hierarchy => return self.hierarchy_reply(reply),
            semantics::Lane::Highlight => return self.occurrences_reply(reply),
            semantics::Lane::Hover => return self.documentation_reply(reply),
        }
        let project = self.state.project.as_ref().map_or(0, |p| p.generation());
        let Some(asking) = self.state.asking.as_ref().filter(|a| a.request == reply.request && a.project == project && self.state.semantics.has_session(reply.session)) else {
            return Effect::default(); // another request, project or server session
        };
        let ask = asking.ask;
        let current = asking.at.as_ref().is_none_or(|(path, _, version)| self.state.tabs.iter().find(|t| t.source == *path).and_then(|t| t.doc.as_ref()).is_some_and(|d| d.version() == *version));
        if !current {
            self.state.asking = None;
            self.state.messages.push(format!("{}: the file changed while the language server answered; ask again", ask.label()));
            return Effect { repaint: true, copy: None };
        }
        let mut value = match reply.result {
            Ok(v) => v,
            Err(e) => {
                self.state.messages.push(format!("{}: {e}", ask.label()));
                return self.answered(reply.request);
            }
        };
        let request = asking.request;
        // A hierarchy's root: its place is located as a definition's is.
        if let Ask::CallHierarchy(_) | Ask::TypeHierarchy(_) = ask {
            let Some(root) = hierarchy::prepare(&value) else {
                self.state.asking = None;
                let none = if matches!(ask, Ask::TypeHierarchy(_)) { "Not a type" } else { "No call hierarchy here" };
                self.state.messages.push(format!("{}: {none}", ask.label()));
                return Effect { repaint: true, copy: None };
            };
            value = serde_json::json!([root.at.clone()]);
            if let Some(a) = self.state.asking.as_mut() {
                a.root = Some(root);
            }
        }
        let Some(asking) = self.state.asking.as_ref() else { return Effect::default() };
        if let Some(target) = asking.retrieving.clone() {
            // The text of a virtual source: decoded off the UI thread, opened as read-only retrieved source.
            let serde_json::Value::String(text) = value else {
                self.state.asking = None;
                self.state.messages.push(format!("{}: {} returned no source for {}", ask.label(), reply.provider, target.source.uri));
                return Effect { repaint: true, copy: None };
            };
            let (versions, tx) = (self.versions.clone(), self.tx.clone());
            self.jobs.submit(Priority::Visible, move || {
                let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                let name = target.source.name();
                let doc = Arc::new(Document::retrieved(PathBuf::from(&*target.source.uri), Path::new(&name), version, &text));
                let _ = tx.send_blocking(Completion::Retrieved { request, target, doc });
            });
            return Effect::default();
        }
        // Positions are about the snapshots the server was sent; other files are read as they are now.
        let open = self.state.semantics.sent(reply.session);
        let (versions, tx) = (self.versions.clone(), self.tx.clone());
        let (encoding, provider, session, indexing) = (reply.encoding, reply.provider, reply.session, reply.indexing);
        let origin = self.state.semantics.origin(session).unwrap_or_else(|| provider.to_string());
        let limit = if ask.question() == semantics::Question::Definition { semantics::TARGET_LIMIT } else { semantics::USAGE_LIMIT };
        // A file's symbols are about the very snapshot the server was sent.
        let asked_doc = asking.at.as_ref().filter(|_| ask == Ask::FileSymbols).and_then(|(path, _, _)| open.get(path).cloned());
        self.jobs.submit(Priority::Visible, move || {
            let read = |path: &Path| match read(path) {
                Ok((stamp, bytes)) => {
                    let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok((stamp, Arc::new(Document::decode(path.to_owned(), version, &bytes))))
                }
                Err(ReadError::Unstable) => Err("it is changing on disk".to_string()),
                Err(ReadError::Failed(e)) => Err(e),
            };
            let targets = match (ask, asked_doc) {
                (Ask::FileSymbols, Some(doc)) => semantics::file_symbols(&value, &doc, encoding, limit),
                // Its positions are about a snapshot the server no longer holds: nothing to place them in.
                (Ask::FileSymbols, None) => semantics::Targets { failed: vec!["the file is no longer open in the language server; ask again".into()], ..Default::default() },
                _ => semantics::targets(&value, limit, encoding, provider, session, &open, &read),
            };
            let _ = tx.send_blocking(Completion::Targets { request, session, provider, origin, indexing, targets });
        });
        Effect::default()
    }

    /// One server's answer to the current request arrived (or failed): the request ends with the last one.
    fn answered(&mut self, request: u64) -> Effect {
        let Some(a) = self.state.asking.as_mut().filter(|a| a.request == request) else { return Effect::default() };
        a.remaining = a.remaining.saturating_sub(1);
        if a.remaining == 0 {
            let ask = a.ask;
            self.state.asking = None;
            if ask == Ask::Symbols
                && let Some(r) = self.state.results.as_mut().filter(|r| r.kind == ResultsKind::Symbols)
            {
                r.done = true;
            }
        }
        Effect { repaint: true, copy: None }
    }

    fn complete(&mut self, completion: Completion) -> Effect {
        match completion {
            Completion::LaidOut(done) => {
                let pane = done.pane as usize;
                let result = self.layouts.finish(*done, |r| layout_current(&self.state, r));
                let failed = result.as_ref().is_some_and(|(_, delta)| delta.is_none());
                if failed {
                    self.layout_intents[pane] = None;
                    self.state.messages.push("Source layout failed; try the navigation again".into());
                }
                let applied = result.and_then(|(request, delta)| delta.map(|delta| (request, delta))).is_some_and(|(request, delta)| {
                    self.state.tabs.iter().find(|tab| tab.id == request.tab).is_some_and(|tab| tab.layout.lock().unwrap_or_else(std::sync::PoisonError::into_inner).apply(delta))
                });
                let mut effect = Effect { repaint: applied || failed, copy: None };
                if applied && let Some(intent) = self.layout_intents.get_mut(pane).and_then(Option::take).filter(|intent| layout_intent_current(&self.state, intent)) {
                    match intent.purpose {
                        LayoutPurpose::Command(command) => {
                            let reveal = matches!(&command, Command::MoveCaret(_) | Command::Extend(_));
                            let result = self.command(command);
                            if reveal && result.repaint { self.keep_selection_visible(); }
                            effect.repaint |= result.repaint; effect.copy = result.copy;
                        }
                        LayoutPurpose::Selection => self.keep_selection_visible(),
                        LayoutPurpose::Reveal { line, byte } => self.bring_into_view(intent.request.tab, line, byte),
                        LayoutPurpose::Band { line, row } => self.clear_of_band(intent.request.tab, line, row),
                    }
                }
                effect
            }
            Completion::Projected(done) => {
                Effect { repaint: self.projections.finish(done, |r| projection_current(&self.state, r)), copy: None }
            }
            Completion::ProviderSettingsSaved { ok } => {
                self.state.provider_saves = self.state.provider_saves.saturating_sub(1);
                if !ok { self.state.messages.push("Could not save language server settings".into()); }
                Effect { repaint: true, copy: None }
            }
            Completion::ProjectProviderSaved { provider, root, reset, result } => {
                self.state.provider_saves = self.state.provider_saves.saturating_sub(1);
                match result {
                    Ok(()) => {
                        if self.state.project_provider() == Some((provider, root.clone())) || self.state.asking.as_ref().is_some_and(|a| a.ask == Ask::Symbols) {
                            if self.state.asking.as_ref().is_some_and(|a| a.ask == Ask::Symbols)
                                && let Some(r) = self.state.results.as_mut().filter(|r| r.kind == ResultsKind::Symbols)
                            {
                                r.done = true;
                                r.error = Some("Language server settings changed; find symbols again".into());
                            }
                            self.state.asking = None;
                            self.state.semantics.cancel();
                        }
                        self.state.semantics.reload_project(provider, &root);
                        self.synced = 0;
                        self.state.messages.push(format!("{} for {}. Its enabled language server will restart.", if reset { "Project language server settings reset to inherited defaults" } else if self.state.semantics.settings().ruby_tooling_excluded(&root) { "Ruby tooling gem exclusions enabled; navigation into those gems is unavailable" } else { "Ruby tooling gem exclusions disabled" }, root.display()));
                    }
                    Err(why) => self.state.messages.push(format!("Project language server settings were not changed: {why}")),
                }
                Effect { repaint: true, copy: None }
            }
            Completion::ServerRuntimes { request, paths } => {
                if request != self.runtime_request { return Effect::default(); }
                if let Some(model) = &mut self.state.server_runtime { model.candidates = paths; model.message = format!("Enter {}, or choose an installed runtime below", model.kind.input_label()); }
                Effect { repaint: true, copy: None }
            }
            Completion::ServerRuntimeSaved { request, kind, result } => {
                if self.state.runtime_saving != Some(request) { return Effect::default(); }
                self.state.runtime_saving = None;
                let message = match result {
                    Ok(path) => {
                        if self.runtime_kind() == Some(kind) { self.state.asking = None; self.state.semantics.cancel(); }
                        self.state.semantics.reload_runtime(kind.provider());
                        self.synced = 0;
                        if let Some(model) = &mut self.state.server_runtime { model.configured = path; }
                        format!("{} runtime saved. Enabled {} language servers are restarting.", kind.label(), kind.provider())
                    }
                    Err(why) => format!("{} runtime was not changed: {why}", kind.label()),
                };
                if let Some(model) = &mut self.state.server_runtime { model.message = message.clone(); }
                self.state.messages.push(message);
                Effect { repaint: true, copy: None }
            }

            Completion::Checked { request, tab, path, stamp } => {
                if request != self.request {
                    return Effect::default();
                }
                match self.cache.take(&path, stamp) {
                    Some(doc) => {
                        self.state.loading = None;
                        self.pending = None;
                        self.show(tab, doc, stamp, false);
                    }
                    None => self.load(tab, false),
                }
                self.load_other();
                Effect { repaint: true, copy: None }
            }
            Completion::LibraryRead { generation, read } => {
                if generation != self.project_generation.load(Ordering::Relaxed) { return Effect::default(); }
                let (requests, message, revealed) = self.state.libraries.apply(read, self.state.settings.folders_first);
                self.read_libraries(requests);
                if let Some(message) = message { self.state.messages.push(message); }
                if revealed.as_ref().is_some_and(|p| self.state.active().is_some_and(|t| t.source == *p)) { self.state.tree_revision += 1; }
                Effect { repaint: true, copy: None }
            }
            Completion::Loaded { request, tab, path, result } => {
                if request != self.request {
                    return Effect::default(); // superseded by a later open, activation or reload
                }
                self.state.loading = None;
                let reload = self.pending.take().is_some_and(|(_, r)| r);
                match result {
                    Ok((stamp, doc, canonical)) => {
                        // Two spellings of one file (a symlink, `..`) are one document: keep the tab already showing it.
                        let pane = self.state.tabs.iter().find(|t| t.id == tab).map(|t| t.pane);
                        let duplicate = canonical.as_ref().and_then(|c| self.state.tabs.iter().find(|t| t.id != tab && Some(t.pane) == pane && t.canonical.as_ref() == Some(c)).map(|t| t.id));
                        match duplicate {
                            Some(other) if !reload => {
                                let (active, shown_other) = (self.state.active == Some(tab), self.state.other == Some(tab));
                                self.remove_tab(tab, false);
                                if active {
                                    self.activate(other);
                                } else if shown_other {
                                    self.state.other = Some(other);
                                    self.show_other();
                                }
                            }
                            _ => {
                                if let Some(t) = self.tab_mut(tab) {
                                    t.canonical = canonical;
                                }
                                self.show(tab, doc, stamp, reload);
                            }
                        }
                    }
                    Err(e) if reload => {
                        // Keep the last loaded content; never recreate the file.
                        if let Some(t) = self.tab_mut(tab) {
                            t.unavailable = Some(e);
                            t.stamp = None;
                        }
                    }
                    Err(e) => match self.tab_mut(tab) {
                        // It showed this source before its content was released: it stays, saying why it is empty.
                        Some(t) if t.restore.is_some() && t.restore.as_ref().is_some_and(|r| r.version > 0) => t.unavailable = Some(e),
                        _ => {
                            self.state.messages.push(format!("Cannot open {}: {e}", path.display()));
                            if self.pending_goto.as_ref().is_some_and(|(p, _)| *p == normalized(&path)) {
                                self.pending_goto = None;
                            }
                            // The tab never showed anything: it goes, and the reader returns to a neighbour.
                            self.remove_tab(tab, true);
                        }
                    },
                }
                self.load_other();
                Effect { repaint: true, copy: None }
            }
            // For each shown tab of that version (both panes may show one snapshot).
            Completion::CheckedOut { request, root, label, made, result } => {
                if self.state.checkout_running != Some(request) {
                    return Effect::default();
                }
                self.state.checkout_running = None;
                let retry = self.pending_checkout.take();
                // The folder changed meanwhile: what happened there is still said, naming it; nothing here reloads.
                let here = self.state.repo.as_ref().is_some_and(|r| r.root == root);
                let place = if here { String::new() } else { format!(" in {}", root.display()) };
                if here {
                    self.state.branches = None;
                    if let Some(r) = self.state.repo.as_mut()
                        && let Some(head) = lenz_content::branches::head(&r.root, &r.git_dir)
                    {
                        r.head = head;
                        r.signal = lenz_content::branches::head_signal(&r.git_dir);
                    }
                }
                let now = self.state.repo.as_ref().filter(|_| here).map_or(label.clone(), |r| r.head.label());
                match result {
                    Ok(_) if made => self.state.messages.push(format!("Created branch {label}{place}")),
                    Ok(outcome) => {
                        self.state.messages.push(format!("Checked out {now}{place}"));
                        if let Some(w) = outcome.warning {
                            self.state.messages.push(format!("Checkout{place}: {w}"));
                        }
                        let stash = lenz_content::branches::SMART_STASH;
                        match outcome.stash_kept {
                            Some(files) if !files.is_empty() => {
                                self.state.messages.push(format!("Smart Checkout{place}: your changes conflict with {now} in {}; they are kept in the stash \"{stash}\" (Git → Resolve Conflicts in Lenz Diff)", files.join(", ")));
                                if here {
                                    self.state.stash_conflicts = Some(files);
                                }
                            }
                            Some(_) => self.state.messages.push(format!("Smart Checkout{place}: your changes could not be put back; they are kept in the stash \"{stash}\"")),
                            None => {}
                        }
                        if here {
                            self.checked_out();
                        }
                    }
                    Err(lenz_content::branches::CheckoutError::Blocked { tracked, untracked }) if here => {
                        let retry = retry.unwrap_or(crate::branches::Retry::Checkout(lenz_content::branches::Target::Branch(label)));
                        self.state.checkout_problem = Some(crate::branches::CheckoutProblem { retry, tracked, untracked, discards: None });
                    }
                    Err(lenz_content::branches::CheckoutError::Blocked { .. }) => self.state.messages.push(format!("Checking out {label}{place} was refused: local changes would be overwritten")),
                    Err(lenz_content::branches::CheckoutError::Git(e)) => self.state.messages.push(format!("{e}{}", if here { String::new() } else { format!(" ({})", root.display()) })),
                }
                Effect { repaint: true, copy: None }
            }
            Completion::Head { root, head } => {
                match self.state.repo.as_mut().filter(|r| r.root == root && Some(&r.head) != head.as_ref()) {
                    Some(r) => {
                        if let Some(head) = head {
                            r.head = head;
                        }
                        Effect { repaint: true, copy: None }
                    }
                    None => Effect::default(),
                }
            }
            Completion::Repo { anchor, result } => {
                if self.state.repo_anchor.as_ref() != Some(&anchor) {
                    return Effect::default();
                }
                // Not a repository although a `.git` is there (unreadable, not trusted): not retried every tick.
                self.repo_git_seen = result.is_err() && anchor.ancestors().any(|d| d.join(".git").exists());
                self.state.repo_answered = Some(anchor);
                self.state.repo = result.ok();
                Effect { repaint: true, copy: None }
            }
            Completion::Branches { request, root, result } => match self.state.branches.as_mut().filter(|_| request == self.branches_requests && self.state.repo.as_ref().is_some_and(|r| r.root == root)) {
                Some(v) => {
                    v.loading = false;
                    match result {
                        Ok((refs, worktrees)) => (v.refs, v.worktrees) = (refs, worktrees),
                        Err(e) => v.error = Some(e),
                    }
                    let none = v.worktrees_only && v.error.is_none() && self.state.branch_rows("").is_empty();
                    if let Some(v) = self.state.branches.as_mut().filter(|_| none) {
                        v.error = Some("No other worktree".into());
                    }
                    Effect { repaint: true, copy: None }
                }
                None => Effect::default(),
            },
            Completion::CommitLoaded { request, result } => match self.state.commit.as_mut().filter(|c| c.request == request) {
                Some(c) => {
                    c.details = Some(result);
                    Effect { repaint: true, copy: None }
                }
                None => Effect::default(),
            },
            Completion::Blamed { tab, request, root, hunks, done } => {
                let Some(b) = self.tab_mut(tab).and_then(|t| t.blame.as_mut()).filter(|b| b.request == request) else { return Effect::default() };
                b.apply(root, hunks);
                match done {
                    Some(Ok(())) => b.done = true,
                    Some(Err(e)) => {
                        if let Some(t) = self.tab_mut(tab) {
                            t.blame = None;
                        }
                        self.state.messages.push(format!("Annotate with Git Blame: {e}"));
                    }
                    None => {}
                }
                Effect { repaint: true, copy: None }
            }
            Completion::SelectionSteps { request, steps } => {
                let Some((id, version, from, _, reveal)) = self.expanding.take_if(|e| e.3 == request) else { return Effect::default() };
                // Still that tab, version and selection; else the answer is stale.
                let Some(doc) = self.state.active().filter(|t| t.id == id && t.selection.unwrap_or(Selection { anchor: 0, head: 0 }) == from).and_then(|t| t.doc.clone()).filter(|d| d.version() == version) else {
                    return Effect::default();
                };
                let steps = steps.unwrap_or_else(|| crate::expand::fallback_steps(&doc, from.range()));
                self.start_expansion(id, version, from, steps, reveal)
            }
            Completion::Matched { tab, request, version, result } => {
                let Some(t) = self.tab_mut(tab) else { return Effect::default() };
                let Some(f) = t.find.as_mut().filter(|f| f.request == request && f.version == version) else { return Effect::default() };
                f.pending = false;
                match result {
                    Err(e) => {
                        (f.error, f.matches, f.current, f.capped) = (Some(e), Vec::new().into(), None, false);
                        Effect { repaint: true, ..Effect::default() }
                    }
                    Ok((matches, capped)) => {
                        (f.error, f.matches, f.capped) = (None, matches.into(), capped);
                        let at = t.selection.zip(t.doc.as_ref()).map_or((t.top_line, 0), |(s, d)| {
                            let at = s.anchor.min(s.head);
                            let line = d.line_of(at);
                            (line, at - d.line_range(line).start)
                        });
                        let f = t.find.as_mut().expect("checked above");
                        f.current = f.at_or_after(at.0, at.1);
                        if f.reveal {
                            self.show_match(tab)
                        } else {
                            Effect { repaint: true, ..Effect::default() }
                        }
                    }
                }
            }
            Completion::Highlighted { version, parsed, folds, declarations } => {
                let (parsed, folds, declarations) = (parsed.map(Arc::new), folds.map(Arc::new), declarations.map(Arc::new));
                let mut repaint = false;
                for t in self.shown_mut(version).filter(|t| t.highlights.is_none()) {
                    t.highlights = parsed.clone();
                    t.folds = folds.clone();
                    t.declarations = declarations.clone();
                    // Kept where a region of the same kind still starts on that line; the caret and top line stay shown.
                    if let Some(f) = &folds {
                        t.collapsed.retain(|first, kind| f.starting(first).is_some_and(|r| r.kind == kind));
                    }
                    t.reveal_view();
                    repaint |= parsed.is_some();
                }
                Effect { repaint, copy: None }
            }
            Completion::Decorated { version, generation, request, data } => {
                // Made under an older setting, or answering a request superseded since (a refresh, a server that
                // finished loading): dropped; the current one is asked for.
                let kind = data.kind();
                let source = self.state.tabs.iter().filter_map(|t| t.doc.as_ref()).find(|d| d.version() == version).map(|d| d.source().to_owned());
                if generation != self.decor_generation[kind as usize] || source.is_none_or(|p| self.state.semantics.latest(&p, kind) != Some(request)) {
                    return Effect::default();
                }
                let mut repaint = false;
                for t in self.state.tabs.iter_mut().filter(|t| t.doc.as_ref().is_some_and(|d| d.version() == version)) {
                    match &data {
                        crate::decor::Decorated::Hints(h) => {
                            t.hints = Some((version, h.clone()));
                            t.set_top(t.top_line, 0); // wrapped rows change
                        }
                        crate::decor::Decorated::Tokens(k) => t.tokens = Some((version, k.clone())),
                    }
                    repaint = true;
                }
                Effect { repaint, copy: None }
            }
            Completion::Rendered { version, preview } => {
                // Superseded: another version, tab or presentation.
                let preview = Some(preview.map(Arc::new));
                let mut repaint = false;
                for t in self.shown_mut(version).filter(|t| t.preview.is_none() && !t.markdown_source) {
                    t.preview = preview.clone();
                    repaint = true;
                }
                Effect { repaint, copy: None }
            }
            Completion::PreviewImages { version, doc, images } => {
                let mut repaint = false;
                for t in self.shown_mut(version) {
                    // Only for the preview they were decoded for, once.
                    let Some(Ok(p)) = t.preview.as_ref().filter(|p| p.as_ref().is_ok_and(|p| Arc::ptr_eq(&p.doc, &doc) && p.images.is_none())) else { continue };
                    let p = crate::markdown::Preview { doc: doc.clone(), syntax: p.syntax.clone(), images: Some(images.clone()) };
                    t.preview = Some(Ok(Arc::new(p)));
                    repaint = true;
                }
                Effect { repaint, copy: None }
            }
            Completion::PlacesChecked { generation, missing } => {
                if self.project_generation.load(Ordering::Relaxed) != generation {
                    return Effect::default(); // a replaced root's
                }
                self.state.missing = missing;
                Effect { repaint: true, copy: None }
            }
            Completion::PlacesRead { generation, file, from, result } => {
                if self.project_generation.load(Ordering::Relaxed) != generation {
                    return Effect::default();
                }
                match result {
                    Ok(saved) => {
                        let workspace = self.restore_workspace.then(|| saved.workspace.clone()).flatten();
                        self.apply_places(file, &from, saved);
                        if let Some(workspace) = workspace { self.apply_workspace(*workspace); }
                    }
                    Err(crate::places::ReadError::OtherVersion) => self.state.messages.push("Places file is from another version; not saved".into()),
                    Err(crate::places::ReadError::Broken(e)) => {
                        // Kept beside it rather than saved over; when it cannot be moved, nothing is saved.
                        let kept = std::fs::rename(&from, from.with_extension("json.unreadable")).is_ok();
                        if kept {
                            self.apply_places(file.clone(), &file, Default::default());
                        }
                        let kept = if kept { "; the file is kept as .json.unreadable" } else { "; nothing will be saved this session" };
                        self.state.messages.push(format!("Bookmarks and recent places could not be read: {e}{kept}"));
                    }
                }
                self.restore_workspace = false;
                // Open files' marks follow their current text.
                for t in &self.state.tabs {
                    if let Some(doc) = &t.doc
                        && self.state.places.has_marks(&t.source)
                    {
                        Arc::make_mut(&mut self.state.places).relocate(&t.source, doc);
                    }
                }
                self.places_changed();
                Effect { repaint: true, copy: None }
            }
            Completion::PlacesMerged { generation, seq, places } => {
                // Taken only for this project's latest save: a change made since sends a save that merges again.
                let current = self.project_generation.load(Ordering::Relaxed) == generation && self.places_seq == seq;
                let Some(file) = self.places_file.clone().filter(|_| current) else { return Effect::default() };
                self.state.places = Arc::new(places);
                self.places_saver.base(file, self.state.places.clone());
                let mut moved = false;
                for t in &self.state.tabs {
                    if let Some(doc) = &t.doc
                        && self.state.places.has_marks(&t.source)
                    {
                        moved |= Arc::make_mut(&mut self.state.places).relocate(&t.source, doc);
                    }
                }
                // Only a real move saves again; otherwise a merge would answer every save.
                if moved {
                    self.places_changed();
                }
                Effect { repaint: true, copy: None }
            }
            Completion::Scanned { generation, batch } => {
                let Some(p) = self.state.project.as_mut().filter(|p| p.walk == generation) else {
                    return Effect::default(); // a replaced root's (or superseded rescan's) walk
                };
                let done = batch.done;
                let paths: Vec<Arc<str>> = batch.paths.iter().map(|p| Arc::from(p.as_str())).collect();
                let rescan = p.refreshing();
                let before = if rescan && done { p.paths() } else { Vec::new() };
                p.add(batch);
                if !rescan {
                    self.state.index.enqueue(paths, false);
                } else if done {
                    // A rescan's inventory replaced the old one: drop records of files that went, queue new ones.
                    let now: std::collections::HashSet<Arc<str>> = p.paths().into_iter().collect();
                    self.state.index.retain(&now);
                    let before: std::collections::HashSet<_> = before.into_iter().collect();
                    let coarse = self.rescan_changes.overflow || !self.rescan_changes.paths.is_empty();
                    let changes: Vec<_> = before.difference(&now).map(|rel| (rel.clone(), false))
                        .chain(now.iter().filter(|rel| coarse || !before.contains(*rel)).filter_map(|rel| {
                            let path = p.root().join(rel.as_ref());
                            (!before.contains(rel) || self.rescan_changes.touches(&path)).then_some((rel.clone(), true))
                        })).collect();
                    self.notify_project_changes(changes);
                    if !self.rescan_again { self.rescan_changes = Change::default(); }
                }
                self.pump_index();
                if done {
                    self.rewatch(); // folders the walk found
                    self.rescanning = false;
                    if std::mem::take(&mut self.rescan_again) {
                        self.rescan();
                    }
                }
                let Some(p) = self.state.project.as_mut() else { return Effect::default() };
                if p.ranking.is_none() && p.finder.as_ref().is_some_and(|f| !f.ready) {
                    self.rank();
                }
                if let Some(n) = &mut self.state.navigation
                    && (matches!(n.scope, crate::navigation::Scope::All | crate::navigation::Scope::Files)
                        || rescan && done && n.scope == crate::navigation::Scope::Symbols)
                { n.dirty = true; if !n.pending { self.rank_navigation(); } }
                Effect { repaint: true, copy: None }
            }
            Completion::Navigation {request, files, indexed, found} => {
                let Some(n) = self.state.navigation.as_mut().filter(|n| n.request == request) else { return Effect::default() };
                n.pending = false;
                let dirty = n.dirty;
                if let Some((rows, matched)) = found { n.rows = rows; n.matched = matched; n.revision = request; }
                if dirty || self.state.project.as_ref().map_or(0, |p| p.files()) != files || self.state.index.coverage().indexed != indexed { self.rank_navigation(); }
                Effect {repaint: true, copy: None}
            }
            Completion::Ranked { generation, seq, searched, hits } => {
                let Some(p) = self.state.project.as_mut().filter(|p| p.generation() == generation) else { return Effect::default() };
                if p.ranking == Some(seq) {
                    p.ranking = None;
                }
                let (Some((hits, matched)), true) = (hits, seq == p.query_seq) else { return Effect::default() };
                let files = p.files();
                if let Some(f) = &mut p.finder {
                    f.hits = hits;
                    f.matched = matched;
                    f.searched = searched;
                    f.ready = searched == files;
                }
                if searched < files {
                    self.rank(); // files arrived while it ran
                }
                Effect { repaint: true, copy: None }
            }
            Completion::Theme(update) => {
                let effect = self.themes.apply(*update);
                self.theme_effect(effect)
            }
            Completion::Found { id, found } => {
                let generation = self.state.project.as_ref().map(|p| p.generation());
                let Some(r) = self.state.results.as_mut().filter(|r| r.id == id && r.root.as_ref().is_none_or(|(_, g)| Some(*g) == generation)) else {
                    return Effect::default(); // another search, or results of a replaced root
                };
                r.hits.extend(found.hits);
                r.rebuild_rows();
                if let Some(files) = found.files {
                    r.files = files;
                }
                r.searched = found.searched;
                r.failed = found.failed;
                r.error = found.error;
                r.syntax_errors = found.syntax_errors;
                r.done = found.done;
                r.limited = found.limited;
                if r.selected.is_none() && !r.hits.is_empty() {
                    // The first result is selected for preview as soon as it arrives.
                    return self.select_result(0);
                }
                Effect { repaint: true, copy: None }
            }
            Completion::ResultSpans { id, spans } => match self.state.results.as_mut().filter(|r| r.id == id) {
                Some(r) => {
                    r.spans.extend(spans.into_iter().filter(|(i, _)| r.spans_asked.contains(i)));
                    Effect { repaint: true, copy: None }
                }
                None => Effect::default(),
            },
            Completion::PreviewHighlighted { request, version, parsed } => {
                if self.preview_parsing.is_some_and(|(_, r)| r == request) {
                    self.preview_parsing = None;
                }
                match self.state.results.as_mut().and_then(|r| r.preview.as_mut()).filter(|p| p.doc.version() == version && p.highlights.is_none()) {
                    Some(p) => {
                        p.highlights = parsed.map(Arc::new);
                        Effect { repaint: p.highlights.is_some(), copy: None }
                    }
                    None => Effect::default(),
                }
            }
            Completion::Previewed { id, index, doc } => {
                let Some(r) = self.state.results.as_mut().filter(|r| r.id == id && r.selected == Some(index)) else { return Effect::default() };
                match doc {
                    Ok((stamp, doc)) => {
                        r.preview = Some(search::Preview { index, doc: doc.clone(), highlights: None });
                        // Nearby results in the same file preview from this snapshot, within the cache's budget.
                        self.cache.put(stamp, doc);
                        self.highlight_preview();
                    }
                    Err(e) => {
                        r.preview = None;
                        self.state.messages.push(format!("Cannot preview: {e}"));
                        self.highlight_preview();
                    }
                }
                Effect { repaint: true, copy: None }
            }
            Completion::Validated { id, open, index, result } => {
                if open != self.open_seq {
                    return Effect::default(); // the reader moved elsewhere meanwhile
                }
                let Some(r) = self.state.results.as_mut().filter(|r| r.id == id) else { return Effect::default() };
                let (Some(hit), Some(path)) = (r.hits.get(index).cloned(), r.target(index)) else { return Effect::default() };
                match result {
                    // Opening is committed: the main area moved, the results stay for the next one.
                    Ok((stamp, doc)) => self.open_at(path, &hit, stamp, doc),
                    Err(e) => {
                        r.stale.insert(hit.path.clone());
                        self.state.messages.retain(|m| !m.starts_with("This result"));
                        self.state.messages.push(format!("This result no longer matches ({e}); rerun the search to update it"));
                        Effect { repaint: true, copy: None }
                    }
                }
            }
            Completion::Indexed { generation, revision, files } => {
                if self.index_revision.load(Ordering::Relaxed) != revision || self.state.project.as_ref().is_none_or(|p| p.generation() != generation) {
                    return Effect::default(); // a replaced root's index
                }
                let mut checked = Vec::new();
                for (path, result) in files {
                    if self.overflow_fingerprints.contains_key(&path) { checked.push((path.clone(), true)); }
                    self.state.index.add(path, result);
                }
                self.state.index.running = false;
                let coverage = self.state.index.coverage();
                if !self.rescan_changes.overflow {
                    if coverage.limited || coverage.pending == 0 {
                        // A byte-budget stop must not leave semantic recovery waiting for unqueued reads.
                        checked = self.overflow_fingerprints.keys().map(|p| (p.clone(), true)).collect();
                    }
                    self.notify_project_changes(checked);
                }
                let mut rerun = None;
                if let Some(r) = self.state.results.as_mut().filter(|r| r.kind == ResultsKind::Declarations) {
                    // Once indexing settles, a query ranked over fewer files runs again to include the rest.
                    if coverage.pending == 0 && r.files < coverage.indexed {
                        rerun = Some(r.query.clone());
                    }
                    r.coverage = Some(coverage);
                }
                if let Some(query) = rerun {
                    self.find_declarations(query);
                }
                if let Some(n) = &mut self.state.navigation
                    && matches!(n.scope, crate::navigation::Scope::All | crate::navigation::Scope::Symbols)
                { n.dirty = true; if !n.pending { self.rank_navigation(); } }
                self.pump_index();
                Effect { repaint: self.state.results.as_ref().is_some_and(|r| r.kind == ResultsKind::Declarations), copy: None }
            }
            Completion::IndexReset { generation, revision, result } => {
                if self.project_generation.load(Ordering::Relaxed) != generation || self.index_revision.load(Ordering::Relaxed) != revision { return Effect::default(); }
                self.index_rebuilding = false;
                match result {
                    Ok(()) => self.state.messages.push("Saved declarations cleared; rebuilding the project index and restarting its language servers…".into()),
                    Err(why) => {
                        self.index_cache = None;
                        self.state.messages.push(format!("Could not clear the saved index: {why}. Rebuilding without cache reuse this session."));
                    }
                }
                if let Some(p) = &self.state.project {
                    self.state.index.enqueue(p.paths(), false);
                    self.state.semantics.restart_project(p.root());
                }
                self.cancel_for_index_rebuild(); self.synced = 0;
                self.changed(Change { overflow: true, ..Change::default() }); self.pump_index();
                Effect { repaint: true, copy: None }
            }
            Completion::Changed(mut change) => {
                // Changed files' declaration records go and are rebuilt.
                let mut coarse = Change { overflow: change.overflow, ..Change::default() };
                let mut files = Vec::new();
                if let Some(p) = &self.state.project {
                    if change.overflow {
                        // Overlapping recovery has no single validated baseline: retain full notifications.
                        self.overflow_fingerprints = if self.overflow_fingerprints.is_empty() && !self.rescan_changes.overflow
                            && self.state.semantics.can_filter_changes() {
                            self.state.index.fingerprints()
                        } else { Default::default() };
                    }
                    if !change.registered.is_empty() {
                        let checks = self.state.index.registration_checks(p.root(), &change.registered);
                        let (tx, generation, current) = (self.tx.clone(), p.generation(), self.project_generation.clone());
                        self.jobs.submit(Priority::Background, move || {
                            let paths = checks.into_iter().filter_map(|(path, stamp)| (Stamp::of(&path) != Some(stamp)).then_some(path)).collect::<std::collections::BTreeSet<_>>();
                            if !paths.is_empty() && current.load(Ordering::Relaxed) == generation {
                                let _ = tx.send_blocking(Completion::Changed(Change { paths, structure: true, ..Change::default() }));
                            }
                        });
                    }
                    for path in &change.paths {
                        if let Some(rel) = p.relative(path) {
                            if rel.is_empty() || p.has_dir(&rel) || path.is_dir() {
                                coarse.paths.insert(path.clone());
                                continue;
                            }
                            // Exact reports are sent now, including precise paths accompanying an overflow.
                            self.overflow_fingerprints.remove(&rel);
                            self.state.index.invalidate(&rel, path.exists());
                        }
                        if !path.is_dir() { files.push((path.clone(), path.exists())); }
                    }
                    if coarse.overflow || !coarse.paths.is_empty() {
                        if !coarse.overflow && !self.overflow_fingerprints.is_empty() {
                            self.overflow_fingerprints.retain(|rel, _| !coarse.touches(&p.root().join(rel.as_ref())));
                        }
                        self.state.index.invalidate_changed(p.root(), &coarse);
                        self.rescan_changes.overflow |= coarse.overflow;
                        self.rescan_changes.paths.extend(coarse.paths.iter().cloned());
                        change.structure = true;
                    }
                    self.pump_index();
                } else {
                    files.extend(change.paths.iter().filter(|p| !p.is_dir()).map(|p| (p.clone(), p.exists())));
                }
                if let Some(query) = self.state.results.as_ref().filter(|r| r.kind == ResultsKind::Declarations).map(|r| r.query.clone()) {
                    self.find_declarations(query);
                }
                // Deletions may leave no indexing job to refresh an open symbol search.
                if let Some(n) = &mut self.state.navigation
                    && matches!(n.scope, crate::navigation::Scope::All | crate::navigation::Scope::Symbols)
                { n.rows.clear(); self.rank_navigation(); }
                // Servers that rely on the client hear of changed files they do not hold open (a rename's ends are
                // among the paths).
                self.state.semantics.files_changed(&files);
                // A workspace manifest changed: language-server roots are resolved again.
                if change.overflow || change.paths.iter().any(|p| p.file_name().is_some_and(|n| n == "Cargo.toml" || n == "rust-project.json")) {
                    self.state.semantics.manifests_changed();
                    self.synced = 0;
                }
                // Results in files that changed (including ones still to arrive) are marked stale; they are checked
                // again before opening. Only the changed paths are visited.
                if let Some(r) = &mut self.state.results {
                    for path in &change.paths {
                        let key: Option<Arc<str>> = match &r.root {
                            Some((root, _)) => path.strip_prefix(root).ok().map(|p| p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/").into()),
                            None => Some(path.to_string_lossy().into()),
                        };
                        r.stale.extend(key);
                    }
                    if change.overflow {
                        r.stale.extend(r.hits.iter().map(|h| h.path.clone()));
                    } else if !coarse.paths.is_empty() {
                        r.stale.extend(r.hits.iter().filter(|h| r.root.as_ref().is_some_and(|(root, _)| coarse.touches(&root.join(h.path.as_ref())))).map(|h| h.path.clone()));
                    }
                }
                self.changed(change)
            }
            Completion::Refreshed { tab, token, result } => self.refreshed(tab, token, result),
            Completion::Watch { key, folders } => {
                if self.watching.as_ref() == Some(&key) {
                    self.inventory_folders = Some((key.0.clone(), folders.clone()));
                    let mut wanted = (*folders).clone();
                    wanted.extend(key.1.iter().cloned());
                    self.watch(key, wanted);
                }
                Effect::default()
            }
            Completion::Semantic(event) => {
                self.sampling &= !matches!(event, semantics::Event::Memory(_));
                let mut outcome = self.state.semantics.event(event, &self.tx);
                for d in std::mem::take(&mut outcome.decorations) {
                    self.decode(d);
                }
                let mut effect = Effect { repaint: outcome.repaint || !self.state.semantics.notices.is_empty(), copy: None };
                self.state.messages.append(&mut self.state.semantics.notices);
                if outcome.ready {
                    // Documents open in the server before a waiting request is sent about them.
                    self.synced = 0;
                    self.sync_semantics();
                    let waited = self.state.semantics.waiting().is_some();
                    let label = self.state.asking.as_ref().map_or("Definition", |a| a.ask.label());
                    match self.state.document().cloned().and_then(|doc| Some((self.state.semantics.take_waiting(&doc)?, doc))) {
                        Some(((request, offset, question), doc)) => {
                            if let Err(why) = self.state.semantics.ask(&doc, offset, request, question) {
                                self.state.asking = None;
                                self.state.messages.push(format!("{label}: {why}"));
                            }
                        }
                        // Asked about another file or version: the question no longer applies.
                        None if waited && self.state.asking.is_some() => {
                            self.state.asking = None;
                            self.state.messages.push(format!("{label}: the file changed while the language server started; ask again"));
                        }
                        None => {}
                    }
                }
                if self.state.asking.is_some() && !outcome.replies.iter().any(|r| r.lane == semantics::Lane::Foreground) {
                    // A request whose server failed to start (or whose file has no workspace): say why instead of
                    // waiting forever.
                    if let Some(why) = self.state.provider().and_then(|v| match v.status {
                        semantics::Status::Missing(w) | semantics::Status::Failed(w) => Some(w),
                        _ => None,
                    }) && self.state.asking.as_ref().is_some_and(|a| a.ask != Ask::Symbols)
                    {
                        let label = self.state.asking.as_ref().map_or("Definition", |a| a.ask.label());
                        self.state.asking = None;
                        self.state.semantics.cancel();
                        self.state.messages.push(format!("{label}: {why}"));
                        effect.repaint = true;
                    }
                }
                for reply in outcome.replies {
                    effect.repaint |= self.semantic_reply(reply).repaint;
                }
                effect
            }
            Completion::DocumentationReady { request, preview } => {
                let Some((_, tab, version, at, trigger)) = self.pending_doc.take_if(|p| p.0 == request) else { return Effect::default() };
                match preview {
                    Ok(preview) => self.state.documentation = Some(Documentation { tab, version, at, trigger, preview: Arc::new(preview) }),
                    Err(why) if trigger == DocTrigger::Caret => self.state.messages.push(format!("Quick documentation: cannot be shown: {why}")),
                    Err(_) => {}
                }
                Effect { repaint: true, copy: None }
            }
            Completion::HierarchyLevel { request, level, limited, indexing } => {
                let Some(h) = self.state.results.as_mut().and_then(|r| r.hierarchy.as_mut()) else { return Effect::default() };
                let Some(node) = h.loading(request) else { return Effect::default() };
                if limited {
                    self.state.messages.push(format!("Type hierarchy: only the first {} usages of {} were read; this level may be incomplete", semantics::TYPE_LEVEL_LIMIT, h.nodes[node].call.name));
                }
                // A level built from usages read at most 200 of them; a server's own answer is shown as it came.
                let limit = if h.source == hierarchy::Source::Syntax { semantics::TYPE_LEVEL_LIMIT } else { semantics::USAGE_LIMIT };
                let (sites, found): (Vec<_>, Vec<_>) = level.into_iter().map(|(call, t)| ((call, t.hit.clone()), t)).unzip();
                h.loaded(node, request, sites, limit);
                if let Some(r) = self.state.results.as_mut() {
                    r.provider_indexing |= indexing;
                    r.rebuild_hierarchy();
                }
                self.keep_previews(found);
                Effect { repaint: true, copy: None }
            }
            Completion::HierarchyRootKind { id, kind } => {
                let Some(r) = self.state.results.as_mut().filter(|r| r.id == id) else { return Effect::default() };
                let Some(h) = r.hierarchy.as_mut() else { return Effect::default() };
                h.nodes[0].call.decl = Some(kind);
                // The root's row shows its kind once rebuilt.
                r.rebuild_hierarchy();
                Effect { repaint: true, copy: None }
            }
            Completion::Targets { request, session, provider, origin, indexing, targets } => {
                if !self.state.semantics.has_session(session) {
                    return self.answered(request); // its server was stopped or replaced meanwhile: one answer fewer
                }
                self.targets_found(request, session, provider, origin, indexing, targets)
            }
            Completion::Retrieved { request, target, doc } => {
                let Some(asked) = self.state.asking.take_if(|a| a.request == request) else { return Effect::default() };
                if !self.state.semantics.has_session(target.source.session) {
                    self.state.messages.push(format!("{}: the {} session that named it has ended; ask again", asked.ask.label(), target.source.provider));
                    return Effect { repaint: true, copy: None };
                }
                if asked.ask == Ask::Peek {
                    self.state.messages.push("Peek: retrieved source opens in a tab; use Go to Definition".into());
                }
                // At the named range, as for any definition: a committed navigation, recorded.
                let encoding = self.state.semantics.encoding(target.source.session);
                let start = semantics::lsp::offset(&doc, &target.range["start"], encoding).unwrap_or(0);
                let end = semantics::lsp::offset(&doc, &target.range["end"], encoding).unwrap_or(start).max(start);
                let line = doc.line_of(start);
                let bounds = doc.line_range(line);
                let at = Location {
                    source: PathBuf::from(&*target.source.uri),
                    version: doc.version(),
                    range: start..end.min(bounds.end),
                    selected: true,
                    top_line: line.saturating_sub(3),
                    anchor: doc.line(line).chars().take(crate::tabs::ANCHOR_CHARS).collect::<String>().into(),
                    line,
                    offset: start - bounds.start,
                };
                self.state.libraries.add_virtual(target.source.clone(), self.state.settings.folders_first);
                self.state.tree_revision += 1;
                // A kept tab: its content cannot be read again once replaced.
                let effect = self.navigate_to(PathBuf::from(&*target.source.uri), true, Some(at), true, Some((doc, target.source)));
                Effect { repaint: true, ..effect }
            }
            Completion::Moved { tab, token, to, stamp, canonical } => {
                let Some(t) = self.tab_mut(tab).filter(|t| t.refresh == token) else { return Effect::default() };
                t.moving = false;
                let from = std::mem::replace(&mut t.source, to.clone());
                t.display = to.clone();
                t.canonical = canonical;
                if let Some(doc) = &t.doc {
                    // Same file, new name: the same snapshot, re-identified.
                    t.doc = Some(Arc::new(doc.renamed(to.clone())));
                    t.stamp = stamp;
                }
                t.unavailable = None;
                let mut current = t.doc.clone();
                let buffer = t.buffer;
                let identity = t.canonical.clone().unwrap_or_else(|| t.source.clone());
                if current.is_none() {
                    self.cache.rename(&from, &to);
                    current = self.cache.peek(&to);
                }
                if let Some(id) = buffer {
                    self.state.sources.aliases.retain(|source, buffer| *buffer != id || *source == identity);
                }
                if let Some(doc) = &current {
                    if let Some(entry) = buffer.and_then(|id| self.state.sources.buffers.get_mut(&id)) {
                        entry.snapshot = entry.snapshot.as_ref().map(|snapshot| Arc::new(snapshot.with_source_meta(doc.source_meta().clone())));
                        entry.document = Arc::downgrade(doc);
                        entry.stamp = stamp;
                    }
                    self.state.sources.admit(&identity, buffer, doc, stamp);
                    self.admit_current(tab);
                }
                // Its bookmarks and recent places follow it.
                if Arc::make_mut(&mut self.state.places).rename(&from, &to) {
                    self.places_changed();
                }
                self.rewatch();
                Effect { repaint: true, copy: None }
            }
        }
    }

    /// Located targets. A definition: one opens in the main area (a committed navigation, from the snapshot just
    /// read); several, or a peek, list in the results panel with the first previewed. Usages list the same way;
    /// symbols add to the list each server's answer.
    fn targets_found(&mut self, request: u64, session: u64, provider: &'static str, origin: String, indexing: bool, targets: semantics::Targets) -> Effect {
        let Some(asked) = self.state.asking.as_ref().filter(|a| a.request == request) else {
            return Effect::default(); // cancelled, or replaced by a later request
        };
        let (ask, at) = (asked.ask, asked.at.clone());
        let label = ask.label();
        // Virtual definition targets only: the first one's text, through the provider's documented method.
        if matches!(ask, Ask::Definition | Ask::Peek) && targets.found.is_empty() && targets.unavailable.first().is_some_and(|v| v.source.retrievable()) {
            if targets.unavailable.len() > 1 {
                self.state.messages.push(format!("{label}: {} targets, all in sources only {provider} can show; showing the first", targets.unavailable.len()));
            }
            let target = targets.unavailable[0].clone();
            match self.state.semantics.retrieve(&target.source, request) {
                Ok(()) => {
                    if let Some(a) = self.state.asking.as_mut() {
                        a.retrieving = Some(target);
                    }
                }
                Err(why) => {
                    self.state.asking = None;
                    self.state.messages.push(format!("{label}: source unavailable: {why}"));
                }
            }
            return Effect { repaint: true, copy: None };
        }
        let retrievable = targets.unavailable.iter().filter(|v| v.source.retrievable()).count();
        if retrievable > 0 {
            self.state.messages.push(format!("{label}: {retrievable} more target(s) are in sources only {provider} can show; not listed"));
        }
        for v in targets.unavailable.iter().filter(|v| !v.source.retrievable()) {
            self.state.messages.push(format!("{label}: source unavailable: {}", v.source.unavailable()));
        }
        if !targets.failed.is_empty() {
            let shown: Vec<&String> = targets.failed.iter().take(3).collect();
            self.state.messages.push(format!("{label}: {} target(s) could not be read: {}", targets.failed.len(), shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("; ")));
        }
        // Usages a provider cannot see in this project (Kotlin callers of Java through JDT LS), said with them.
        if ask == Ask::Usages
            && let Some(p) = semantics::provider::PROVIDERS.iter().find(|p| p.name == provider)
            && let Some((_, note)) = p.blind_to.iter().find(|(ext, _)| self.state.project.as_ref().is_some_and(|pr| pr.has_extension(ext)))
        {
            self.state.messages.push(format!("{label}: {note}"));
        }
        let mut found = targets.found;
        if ask == Ask::Symbols {
            // Each server's answer joins the list; the request ends with the last.
            let first = self.state.results.as_ref().is_some_and(|r| r.kind == ResultsKind::Symbols && r.hits.is_empty());
            if let Some(r) = self.state.results.as_mut().filter(|r| r.kind == ResultsKind::Symbols) {
                let room = semantics::USAGE_LIMIT.saturating_sub(r.hits.len());
                r.limited |= targets.limited || found.len() > room;
                // Each keeps the server it came from; servers of overlapping workspaces may name the same place.
                let origin: Arc<str> = origin.into();
                let new: Vec<search::Hit> = found
                    .iter()
                    .filter(|t| !r.hits.iter().any(|h| h.path == t.hit.path && h.line == t.hit.line && h.range == t.hit.range))
                    .take(room)
                    .map(|t| search::Hit { origin: Some(origin.clone()), ..t.hit.clone() })
                    .collect();
                r.hits.extend(new);
                r.rebuild_rows();
                r.files = r.hits.len();
                r.provider_indexing |= indexing;
            }
            self.keep_previews(found);
            let effect = self.answered(request);
            return if first && self.state.results.as_ref().is_some_and(|r| !r.hits.is_empty()) { self.select_result(0) } else { effect };
        }
        let root = self.state.asking.take().and_then(|a| a.root);
        if let (Ask::CallHierarchy(direction) | Ask::TypeHierarchy(direction), Some(root), Some(t)) = (ask, root, found.first()) {
            let site = t.hit.clone();
            self.search_seq += 1;
            let id = self.search_seq;
            self.search_latest.store(id, Ordering::Relaxed);
            let mut results = self.new_results(id, ResultsKind::Hierarchy, String::new(), Scope::Project, None, 0, false);
            results.provider = Some(origin);
            results.provider_indexing = indexing;
            results.asked = at.map(|(path, offset, _)| (path, offset));
            results.hierarchy = Some(Hierarchy::new(root, site, direction, session, hierarchy::Source::Server));
            results.done = true;
            results.rebuild_hierarchy();
            self.state.results = Some(results);
            self.keep_previews(found);
            self.select_result(0);
            return self.unfold_call(0);
        }
        if found.is_empty() {
            if targets.unavailable.is_empty() && targets.failed.is_empty() {
                let note = if indexing { " (it may still be loading or indexing its workspace; try again shortly)" } else { "" };
                let limits = semantics::provider::PROVIDERS.iter().find(|p| p.name == provider).map_or(String::new(), |p| format!(". Known limits: {}", p.limits));
                let what = match ask {
                    Ask::Usages => "no usages",
                    Ask::FileSymbols => "no symbols in this file",
                    Ask::Implementation => "no implementations",
                    Ask::TypeDefinition => "no type definition here",
                    _ => "no definition here",
                };
                self.state.messages.push(format!("{label}: {provider} found {what}{note}{limits}"));
            }
            return Effect { repaint: true, copy: None };
        }
        if found.len() == 1 && matches!(ask, Ask::Definition | Ask::Implementation | Ask::TypeDefinition) {
            let Some(t) = found.pop() else { return Effect::default() };
            let effect = self.open_at(t.path, &t.hit, t.stamp, t.doc);
            if indexing {
                // An early answer can stop short (at an import) while the server is still loading its project.
                self.state.messages.push(format!("{label}: {provider} may still be loading its workspace; if this is not the declaration, ask again shortly"));
            }
            return effect;
        }
        self.search_seq += 1;
        let id = self.search_seq;
        self.search_latest.store(id, Ordering::Relaxed);
        let scope = match (&at, ask) {
            (Some((path, _, _)), Ask::FileSymbols) => Scope::File(path.clone()),
            _ => Scope::Project,
        };
        let mut results = self.new_results(id, ask.results(), String::new(), scope, None, found.len(), false);
        results.provider = Some(origin);
        results.provider_indexing = indexing;
        results.asked = at.map(|(path, offset, _)| (path, offset));
        results.hits = found.iter().map(|t| t.hit.clone()).collect();
        results.rebuild_rows();
        results.done = true;
        results.limited = targets.limited;
        self.state.results = Some(results);
        self.keep_previews(found);
        self.select_result(0)
    }

    /// Unfold call-hierarchy node `node`, asking for its level the first time (or again after a failure); fold it
    /// when it is unfolded. One level loads at a time: one still loading is given up (unfolding it asks again).
    fn unfold_call(&mut self, node: usize) -> Effect {
        let Some(r) = self.state.results.as_mut() else { return Effect::default() };
        let Some(h) = r.hierarchy.as_mut() else { return Effect::default() };
        let Some(n) = h.nodes.get_mut(node).filter(|n| !n.recursive) else { return Effect::default() };
        let gone = !self.state.semantics.can_ask(h.session);
        match n.state {
            hierarchy::Children::Loading(_) if !gone => return Effect::default(),
            hierarchy::Children::Loaded { .. } => {
                n.open = !n.open;
                r.rebuild_hierarchy();
                return Effect { repaint: true, copy: None };
            }
            _ => {}
        }
        for other in &mut h.nodes {
            if matches!(other.state, hierarchy::Children::Loading(_)) {
                other.state = hierarchy::Children::NotAsked;
                other.open = false;
            }
        }
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.hierarchy_latest.store(request, Ordering::Relaxed);
        let question = match (h.direction, h.source) {
            (Direction::Callers, _) => semantics::Question::IncomingCalls,
            (Direction::Callees, _) => semantics::Question::OutgoingCalls,
            (_, hierarchy::Source::Syntax) => semantics::Question::TypeUsages,
            (Direction::Supertypes, hierarchy::Source::Server) => semantics::Question::Supertypes,
            (Direction::Subtypes, hierarchy::Source::Server) => semantics::Question::Subtypes,
        };
        let item = h.nodes[node].call.item.clone();
        let asked = match h.source {
            // A syntax node is asked about where its name is: a file the server need not have open.
            hierarchy::Source::Syntax => self.state.semantics.ask_at(h.session, item["uri"].as_str().unwrap_or_default(), item["selectionRange"]["start"].clone(), request, question),
            hierarchy::Source::Server => self.state.semantics.ask_item(h.session, item, request, question),
        };
        h.nodes[node].state = match asked {
            Ok(()) => hierarchy::Children::Loading(request),
            Err(why) => hierarchy::Children::Failed(why),
        };
        r.rebuild_hierarchy();
        Effect { repaint: true, copy: None }
    }

    /// Show the hierarchy the other way from its root; without one of that kind (calls or types), ask at the caret.
    fn hierarchy_direction(&mut self, direction: Direction) -> Effect {
        let ask = if direction.is_type() { Ask::TypeHierarchy(direction) } else { Ask::CallHierarchy(direction) };
        let Some(h) = self.state.results.as_mut().and_then(|r| r.hierarchy.as_mut()).filter(|h| h.direction.is_type() == direction.is_type()) else { return self.ask(ask) };
        let root = &h.nodes[0];
        *h = Hierarchy::new(root.call.clone(), root.site.clone(), direction, h.session, h.source);
        if let Some(r) = self.state.results.as_mut() {
            r.selected = Some(0);
            r.rebuild_hierarchy();
        }
        self.unfold_call(0)
    }

    /// A call-hierarchy level's answer: each function is located at its call site off the UI thread.
    fn hierarchy_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some(h) = self.state.results.as_mut().and_then(|r| r.hierarchy.as_mut()).filter(|h| h.session == reply.session) else { return Effect::default() };
        let Some(node) = h.loading(reply.request) else { return Effect::default() };
        let value = match reply.result {
            Ok(v) => v,
            Err(e) => {
                h.failed(node, reply.request, e);
                if let Some(r) = self.state.results.as_mut() {
                    r.rebuild_hierarchy();
                }
                return Effect { repaint: true, copy: None };
            }
        };
        let parent = h.nodes[node].call.item["uri"].as_str().unwrap_or_default().to_owned();
        if h.source == hierarchy::Source::Syntax {
            let direction = h.direction;
            let open = self.state.semantics.sent(reply.session);
            let (versions, tx, request, encoding, provider, session, indexing) = (self.versions.clone(), self.tx.clone(), reply.request, reply.encoding, reply.provider, reply.session, reply.indexing);
            let latest = self.hierarchy_latest.clone();
            self.jobs.submit(Priority::Visible, move || {
                let read = |path: &Path| match read(path) {
                    Ok((stamp, bytes)) => {
                        let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                        Ok((stamp, Arc::new(Document::decode(path.to_owned(), version, &bytes))))
                    }
                    Err(ReadError::Unstable) => Err("it is changing on disk".to_string()),
                    Err(ReadError::Failed(e)) => Err(e),
                };
                let usages = semantics::targets(&value, semantics::TYPE_LEVEL_LIMIT, encoding, provider, session, &open, &read);
                let limited = usages.limited;
                let level = hierarchy::merge_targets(syntax_level(usages.found, direction, encoding, latest, request));
                let _ = tx.send_blocking(Completion::HierarchyLevel { request, level, limited, indexing });
            });
            return Effect::default();
        }
        let types = h.direction.is_type();
        let calls = if types { hierarchy::types(&value) } else { hierarchy::calls(&value, h.direction, &parent) };
        let open = self.state.semantics.sent(reply.session);
        let (versions, tx, request, encoding, provider, session, indexing) = (self.versions.clone(), self.tx.clone(), reply.request, reply.encoding, reply.provider, reply.session, reply.indexing);
        self.jobs.submit(Priority::Visible, move || {
            let read = |path: &Path| match read(path) {
                Ok((stamp, bytes)) => {
                    let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok((stamp, Arc::new(Document::decode(path.to_owned(), version, &bytes))))
                }
                Err(ReadError::Unstable) => Err("it is changing on disk".to_string()),
                Err(ReadError::Failed(e)) => Err(e),
            };
            // Each file is read once for all the calls in it.
            let files = std::cell::RefCell::new(std::collections::HashMap::new());
            let read = |path: &Path| files.borrow_mut().entry(path.to_owned()).or_insert_with(|| read(path)).clone();
            let level = calls
                .into_iter()
                .filter_map(|call| {
                    let t = semantics::targets(&serde_json::json!([call.at.clone()]), 1, encoding, provider, session, &open, &read).found.pop()?;
                    Some((call, t))
                })
                .collect();
            // A type may be named twice at one place; call sites are distinct already.
            let level = if types { hierarchy::merge_targets(level) } else { level };
            let _ = tx.send_blocking(Completion::HierarchyLevel { request, level, limited: false, indexing });
        });
        Effect::default()
    }

    /// The snapshots just read serve the previews (within the cache's budget).
    fn keep_previews(&mut self, found: Vec<semantics::Target>) {
        for t in found {
            if let Some(stamp) = t.stamp {
                self.cache.put(stamp, t.doc);
            }
        }
    }

    /// Open `path` in the preview tab at `hit`, from `doc` (the snapshot it was checked against, read as `stamp`
    /// when it came from disk): a committed navigation.
    fn open_at(&mut self, path: PathBuf, hit: &search::Hit, stamp: Option<Stamp>, doc: Arc<Document>) -> Effect {
        let dependency = semantics::provider::origin(&path, doc.language()) != semantics::provider::OUTSIDE;
        let outside = self.state.project.as_ref().map_or_else(|| self.state.provider().and_then(|p| p.root).is_some_and(|root| !path.starts_with(root)), |p| !path.starts_with(p.root()));
        if dependency || outside || self.state.libraries.contains(&path) {
            let requests = self.state.libraries.reveal(&path, self.state.settings.folders_first);
            self.read_libraries(requests);
            if self.state.libraries.contains(&path) { self.state.tree_revision += 1; }
        }
        let start = doc.line_range(hit.line).start;
        let mut at = Location {
            source: path.clone(),
            version: doc.version(),
            range: start + hit.range.start..start + hit.range.end,
            selected: true,
            top_line: hit.line.saturating_sub(3),
            anchor: doc.line(hit.line).chars().take(crate::tabs::ANCHOR_CHARS).collect::<String>().into(),
            line: hit.line,
            offset: hit.range.start,
        };
        match (self.state.tabs.iter().find(|t| t.source == path).and_then(|t| t.doc.as_ref()), stamp) {
            // Already open with this snapshot (or the same text): that is the checked one.
            (Some(open), _) if Arc::ptr_eq(open, &doc) || open.text() == doc.text() => at.version = open.version(),
            // Open but older: its refresh brings the checked content; the anchor finds the line.
            (Some(_), _) => {}
            (None, Some(stamp)) => self.cache.put(stamp, doc),
            (None, None) => {}
        }
        let effect = self.navigate(path, false, Some(at), true);
        Effect { repaint: true, ..effect }
    }

    /// Keep tree activation out of the dispatcher used by scrolling and startup.
    #[cold]
    fn activate_tree(&mut self, path: Arc<str>, retain: bool) -> Effect {
        let Some(row) = self.state.tree_rows().find(|r| r.path == path).cloned() else { return Effect::default() };
        if row.dir.is_some() {
            return if retain { Effect::default() } else { self.command(Command::ToggleDir(path.to_string())) };
        }
        if let Some(source) = self.state.libraries.virtual_source(&path).cloned() {
            if let Some((doc, held)) = self.state.tabs.iter().find(|t| t.source == Path::new(path.as_ref()))
                .and_then(|t| Some((t.doc.clone()?, t.retrieved.clone()?)))
            {
                return self.navigate_to(PathBuf::from(path.as_ref()), true, None, true, Some((doc, held)));
            }
            self.state.semantics.cancel();
            self.semantic_seq += 1;
            let request = self.semantic_seq;
            let target = semantics::VirtualTarget { source, range: serde_json::Value::Null };
            match self.state.semantics.retrieve(&target.source, request) {
                Ok(()) => self.state.asking = Some(Asking { request, ask: Ask::Definition, at: None, project: self.project_generation.load(Ordering::Relaxed), remaining: 1, since: std::time::Instant::now(), retrieving: Some(target), root: None }),
                Err(why) => { self.state.asking = None; self.state.messages.push(format!("Library source unavailable: {why}")); }
            }
            return Effect { repaint: true, copy: None };
        }
        let file = if Path::new(path.as_ref()).is_absolute() || is_virtual(Path::new(path.as_ref())) {
            PathBuf::from(path.as_ref())
        } else if let Some(p) = &self.state.project { p.root().join(path.as_ref()) } else { return Effect::default() };
        self.navigate(file, retain, None, true)
    }

    fn read_libraries(&self, requests: Vec<crate::libraries::Request>) {
        for request in requests {
            let (tx, latest) = (self.tx.clone(), self.project_generation.clone());
            let generation = latest.load(Ordering::Relaxed);
            self.jobs.submit(Priority::Background, move || {
                let current = || latest.load(Ordering::Relaxed) == generation;
                if !current() { return; }
                let read = request.read(current);
                if current() && read.current() { let _ = tx.send_blocking(Completion::LibraryRead { generation, read }); }
            });
        }
    }

    /// Start a search, replacing (and stopping) any earlier one.
    fn search(&mut self, text: String, scope: Scope, pattern: String) -> Effect {
        self.search_seq += 1;
        let id = self.search_seq;
        self.search_latest.store(id, Ordering::Relaxed);
        let (latest, tx) = (self.search_latest.clone(), self.tx.clone());
        let options = self.find_options;
        let query = lenz_content::find::Query { text, options };
        let base = |s: &Self, scope, root, files, partial| Results { options, ..s.new_results(id, ResultsKind::Text, query.text.clone(), scope, root, files, partial) };
        match scope {
            Scope::Project => {
                let Some(p) = &self.state.project else { return Effect::default() };
                let paths = p.paths();
                let partial = !matches!(p.coverage(), project::Coverage::Complete(_));
                let root = p.root().to_owned();
                let generation = p.generation();
                let mut results = base(self, Scope::Project, Some((root.clone(), generation)), paths.len(), partial);
                results.pattern = pattern.clone();
                self.state.results = Some(results);
                self.jobs.submit(Priority::Visible, move || {
                    // Picked and put in folder order off the UI thread: the results' tree follows that order.
                    let paths = match search::select_files(paths, &pattern) {
                        Ok(paths) => paths,
                        Err(e) => {
                            let _ = tx.send_blocking(Completion::Found { id, found: search::Found { done: true, error: Some(e), files: Some(0), ..Default::default() } });
                            return;
                        }
                    };
                    if !tx.send_blocking(Completion::Found { id, found: search::Found { files: Some(paths.len()), ..Default::default() } }).is_ok() {
                        return;
                    }
                    search::search_files(&root, &paths, &query, id, &latest, &mut |found| tx.send_blocking(Completion::Found { id, found }).is_ok());
                });
            }
            Scope::File(source) => {
                // The document as the reader holds it (a tab's snapshot, even if its file is gone), else as on disk.
                let held = self.state.tabs.iter().find(|t| t.source == source).and_then(|t| t.doc.clone()).or_else(|| self.cache.peek(&source));
                let path: Arc<str> = source.to_string_lossy().into();
                self.state.results = Some(base(self, Scope::File(source.clone()), None, 1, false));
                self.jobs.submit(Priority::Visible, move || {
                    if latest.load(Ordering::Relaxed) != id {
                        return;
                    }
                    let found = match held {
                        Some(doc) => search::search_text(&query, &path, doc.text()),
                        None => match read(&source) {
                            Ok((_, bytes)) => search::search_text(&query, &path, &lenz_content::encoding::decode(&bytes, None).text),
                            Err(ReadError::Unstable) => search::Found { done: true, error: Some("the file is changing on disk".into()), ..Default::default() },
                            Err(ReadError::Failed(e)) => search::Found { done: true, error: Some(e), ..Default::default() },
                        },
                    };
                    let _ = tx.send_blocking(Completion::Found { id, found });
                });
            }
        }
        Effect { repaint: true, copy: None }
    }

    /// The active document's declarations, as results: extracted off the UI thread from the shown snapshot.
    fn outline(&mut self) -> Effect {
        if self.disabled(CommandId::Outline).is_some() {
            return Effect::default();
        }
        let Some(doc) = self.state.document().cloned() else { return Effect::default() };
        let Some(lang) = doc.language() else { return Effect::default() };
        self.search_seq += 1;
        let id = self.search_seq;
        self.search_latest.store(id, Ordering::Relaxed);
        self.state.results = Some(self.new_results(id, ResultsKind::Outline, String::new(), Scope::File(doc.source().to_owned()), None, 1, false));
        let (latest, tx) = (self.search_latest.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            let cancelled = latest.clone();
            let found = match lenz_content::highlight::declarations_while(lang, doc.text().clone(), move || cancelled.load(Ordering::Relaxed) == id) {
                Some(found) => {
                    let path: Arc<str> = doc.source().to_string_lossy().into();
                    let hits = found
                        .items
                        .into_iter()
                        .map(|d| {
                            let line = doc.line_of(d.name_range.start as usize);
                            search::Hit::declaration(path.clone(), line, doc.line_range(line).start, &d)
                        })
                        .collect();
                    search::Found { hits, searched: 1, done: true, syntax_errors: found.syntax_errors, ..Default::default() }
                }
                None if latest.load(Ordering::Relaxed) != id => return,
                None => search::Found { done: true, error: Some("the outline could not be built".into()), ..Default::default() },
            };
            let _ = tx.send_blocking(Completion::Found { id, found });
        });
        Effect { repaint: true, copy: None }
    }

    /// Declarations across the project matching `query`, ranked off the UI thread over the index as it is now.
    fn find_declarations(&mut self, query: String) -> Effect {
        let Some(p) = &self.state.project else { return Effect::default() };
        let (root, generation) = (p.root().to_owned(), p.generation());
        self.search_seq += 1;
        let id = self.search_seq;
        self.search_latest.store(id, Ordering::Relaxed);
        let coverage = self.state.index.coverage();
        let scanning = !matches!(p.coverage(), project::Coverage::Complete(_));
        let mut results = self.new_results(id, ResultsKind::Declarations, query.clone(), Scope::Project, Some((root, generation)), coverage.indexed, scanning);
        results.coverage = Some(coverage);
        self.state.results = Some(results);
        let (files, latest, tx) = (self.state.index.snapshot(), self.search_latest.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            let found = match index::rank(&files, &query, || latest.load(Ordering::Relaxed) == id) {
                Some((hits, matched)) => search::Found {
                    hits: hits
                        .into_iter()
                        .map(|(path, i)| search::Hit::declaration(path, i.line as usize, i.line_start as usize, &i.decl))
                        .collect(),
                    searched: files.len(),
                    done: true,
                    limited: matched > index::DECL_LIMIT,
                    ..Default::default()
                },
                None => return,
            };
            let _ = tx.send_blocking(Completion::Found { id, found });
        });
        Effect { repaint: true, copy: None }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_results(&self, id: u64, kind: ResultsKind, query: String, scope: Scope, root: Option<(PathBuf, u64)>, files: usize, partial: bool) -> Results {
        Results {
            id,
            kind,
            provider: None,
            asked: None,
            provider_indexing: false,
            coverage: None,
            query,
            options: Default::default(),
            scope,
            root,
            hits: Vec::new(),
            searched: 0,
            failed: 0,
            error: None,
            syntax_errors: false,
            files,
            partial_inventory: partial,
            done: false,
            limited: false,
            selected: None,
            preview: None,
            stale: Default::default(),
            pattern: String::new(),
            collapsed: Default::default(),
            rows: Vec::new(),
            hierarchy: None,
            call_nodes: Vec::new(),
            spans: Default::default(),
            spans_asked: Default::default(),
            matched_files: 0,
        }
    }

    /// Select result `i` and show it in the panel: from a tab or the cache when they hold the file, else read off the
    /// UI thread. The main area does not move and no history step is recorded.
    fn select_result(&mut self, i: usize) -> Effect {
        let Some(r) = self.state.results.as_mut() else { return Effect::default() };
        let Some(path) = r.target(i) else { return Effect::default() };
        r.selected = Some(i);
        let id = r.id;
        let stale = r.is_stale(i);
        // The same file as the current preview: reuse its snapshot and colours (no read per arrow press) unless it
        // changed; else a tab's, with its colours, or the cache's.
        let current = r.preview.as_ref().filter(|p| !stale && p.doc.source() == path).map(|p| (p.doc.clone(), p.highlights.clone()));
        let shared = current.or_else(|| {
            let tab = self.state.tabs.iter().find(|t| t.source == path);
            tab.and_then(|t| Some((t.doc.clone()?, t.highlights.clone())))
                .or_else(|| (!stale).then(|| self.cache.peek(&path)).flatten().map(|d| (d, None)))
        });
        let preview = self.preview_latest.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(r) = self.state.results.as_mut() else { return Effect::default() };
        match shared {
            Some((doc, highlights)) => {
                r.preview = Some(search::Preview { index: i, doc, highlights });
                self.highlight_preview();
            }
            None => {
                r.preview = None;
                self.highlight_preview();
                let (versions, tx, latest) = (self.versions.clone(), self.tx.clone(), self.preview_latest.clone());
                self.jobs.submit(Priority::Visible, move || {
                    if latest.load(Ordering::Relaxed) != preview {
                        return; // a later selection replaced this one before it started
                    }
                    let doc = match read(&path) {
                        Ok((stamp, bytes)) => {
                            let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                            Ok((stamp, Arc::new(Document::decode(path, version, &bytes))))
                        }
                        Err(ReadError::Unstable) => Err("it is changing on disk".to_string()),
                        Err(ReadError::Failed(e)) => Err(e),
                    };
                    let _ = tx.send_blocking(Completion::Previewed { id, index: i, doc });
                });
            }
        }
        Effect { repaint: true, copy: None }
    }

    /// Check result `i` against the file as it is now, off the UI thread; opening follows if it still matches.
    fn validate_result(&mut self, i: usize) -> Effect {
        let Some(r) = self.state.results.as_ref() else { return Effect::default() };
        let (Some(hit), Some(path)) = (r.hits.get(i).cloned(), r.target(i)) else { return Effect::default() };
        let (id, file_scope, kind) = (r.id, matches!(r.scope, Scope::File(_)), r.kind);
        let query = lenz_content::find::Query { text: r.query.clone(), options: r.options };
        // A file search is checked against the document the reader holds (its file may be gone); a project search
        // against the file as it is now.
        let held = file_scope.then(|| self.state.tabs.iter().find(|t| t.source == path).and_then(|t| t.doc.clone())).flatten();
        self.open_seq += 1;
        let (open, versions, tx) = (self.open_seq, self.versions.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            let checked = match held {
                Some(doc) => Ok((None, doc)),
                None => match read(&path) {
                    Ok((stamp, bytes)) => {
                        let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                        Ok((Some(stamp), Arc::new(Document::decode(path, version, &bytes))))
                    }
                    Err(ReadError::Unstable) => Err("it is changing on disk".to_string()),
                    Err(ReadError::Failed(e)) => Err(e),
                },
            };
            let result = checked.and_then(|(stamp, doc)| {
                let line = (hit.line < doc.line_count()).then(|| doc.line(hit.line));
                match line {
                    Some(line) if search::still_matches(&hit, kind, &query, line) => Ok((stamp, doc)),
                    _ => Err("the file changed".to_string()),
                }
            });
            let _ = tx.send_blocking(Completion::Validated { id, open, index: i, result });
        });
        Effect::default()
    }

    /// Filesystem changes: refresh affected tabs, follow confirmed moves, rescan the project's structure.
    fn changed(&mut self, change: Change) -> Effect {
        let mut moved = Vec::new();
        for (from, to) in &change.moves {
            for id in self.state.tabs.iter().filter(|t| t.source == *from).map(|t| t.id).collect::<Vec<_>>() {
                moved.push(id);
                self.check(id, Some(to.clone()));
            }
        }
        let affected: Vec<TabId> = self.state.tabs.iter().filter(|t| change.touches(&t.source) && !moved.contains(&t.id)).map(|t| t.id).collect();
        for id in affected {
            self.refresh(id);
        }
        let structural = change.overflow
            || change.structure
                && self.state.project.as_ref().is_some_and(|p| change.paths.iter().any(|c| c.starts_with(p.root()) && !c.components().any(|x| x.as_os_str() == ".git")));
        if structural {
            self.rescan();
        } else if self.state.project.as_ref().is_some_and(|p| change.registered.iter().any(|d| d.starts_with(p.root()))) {
            // Finish the first walk before checking the discovery/registration gap.
            if self.state.project.as_ref().is_some_and(|p| matches!(p.coverage(), project::Coverage::Scanning(_))) {
                self.rescan_again = true;
            } else {
                self.rescan();
            }
        }
        Effect::default()
    }

    /// Check tab `id`'s source against the disk off the UI thread.
    fn refresh(&mut self, id: TabId) {
        if self.state.tabs.iter().any(|t| t.id == id && (t.moving || t.retrieved.is_some())) {
            return;
        }
        self.check(id, None);
    }

    /// Check tab `id` off the UI thread. With `moved_to`, follow the rename when the file there is the same file
    /// (identity) and nothing new took its old place; otherwise check the old path like any refresh.
    fn check(&mut self, id: TabId, moved_to: Option<PathBuf>) {
        let token = self.next_refresh();
        let Some(t) = self.tab_mut(id) else { return };
        t.refresh = token;
        t.moving = moved_to.is_some();
        let (path, stamp, shown, versions, tx) = (t.source.clone(), t.stamp, t.doc.clone(), self.versions.clone(), self.tx.clone());
        // A released tab's file identity is kept with its snapshot in the cache (gone once evicted).
        let identity = stamp.or_else(|| self.cache.stamp(&path));
        self.jobs.submit(Priority::Visible, move || {
            if let Some(to) = moved_to {
                let same_file = identity.and_then(|s| s.file).is_some_and(|f| Stamp::of(&to).and_then(|s| s.file) == Some(f));
                // An editor that renames a file to a backup and writes a new one in its place keeps the reader on the
                // original path: a file there again means a replacement, not a move.
                if same_file && Stamp::of(&path).is_none() {
                    let now = Stamp::of(&to);
                    let canonical = std::fs::canonicalize(&to).ok();
                    let _ = tx.send_blocking(Completion::Moved { tab: id, token, to, stamp: now, canonical });
                    return;
                }
            }
            let result = check(&path, stamp, shown.as_ref(), &versions);
            let _ = tx.send_blocking(Completion::Refreshed { tab: id, token, result });
        });
    }

    fn next_refresh(&mut self) -> u64 {
        self.refresh_seq += 1;
        self.refresh_seq
    }

    fn refreshed(&mut self, id: TabId, token: u64, result: Refresh) -> Effect {
        let shown = self.is_shown(id);
        let Some(t) = self.tab_mut(id).filter(|t| t.refresh == token) else { return Effect::default() };
        t.moving = false;
        if !matches!(result, Refresh::Unstable) {
            t.changing = 0;
        }
        match result {
            Refresh::Same(stamp) => {
                let changed = t.unavailable.take().is_some() || std::mem::take(&mut t.stale_on_disk);
                t.stamp = Some(stamp);
                Effect { repaint: changed, copy: None }
            }
            Refresh::Released => {
                t.stale_on_disk = true;
                Effect::default()
            }
            Refresh::Changed(stamp, doc) => {
                // A new version: the view follows its context, without a history step or a role change.
                let here = t.location();
                t.doc = Some(doc.clone());
                t.stamp = Some(stamp);
                t.unavailable = None;
                t.stale_on_disk = false;
                t.new_content();
                t.restore_to(&here);
                let source = t.source.clone();
                self.admit_current(id);
                if shown {
                    self.mark_shown(id, doc.version());
                    self.decorate(&doc);
                }
                if self.state.places.has_marks(&source) && Arc::make_mut(&mut self.state.places).relocate(&source, &doc) {
                    self.places_changed();
                }
                // An open find follows the new content, without moving the view.
                if let Some(text) = self.tab_mut(id).and_then(|t| t.find.as_ref()).map(|f| f.query.clone()) {
                    self.find_in(id, text, false);
                }
                self.annotate(id);
                Effect { repaint: true, copy: None }
            }
            Refresh::Gone(e) => {
                // Keep (or take back from the cache) the last content; it is no longer reloadable, so it is never
                // released again until the tab closes.
                if t.doc.is_none() {
                    let path = t.source.clone();
                    let doc = self.cache.take_any(&path);
                    let Some(t) = self.tab_mut(id) else { return Effect::default() };
                    t.doc = doc;
                    if t.doc.is_some()
                        && let Some(at) = t.restore.take()
                    {
                        t.restore_to(&at);
                    }
                }
                self.admit_current(id);
                let Some(t) = self.tab_mut(id) else { return Effect::default() };
                t.stamp = None;
                t.unavailable = Some(e);
                Effect { repaint: true, copy: None }
            }
            Refresh::Unstable => {
                // The reads raced a writer every time: the shown content stays the last confirmed one. Retry shortly,
                // with a growing delay, in case the writer's last notification was already coalesced away.
                t.changing += 1;
                let (attempt, path) = (t.changing, t.source.clone());
                if attempt <= UNSTABLE_RETRIES {
                    let tx = self.tx.clone();
                    let delay = std::time::Duration::from_millis(100 << attempt.min(5));
                    std::thread::spawn(move || {
                        std::thread::sleep(delay);
                        let _ = tx.send_blocking(Completion::Changed(Change { paths: [path].into(), ..Change::default() }));
                    });
                }
                Effect { repaint: true, copy: None }
            }
        }
    }

    /// Rescan the project after structural changes: at most one walk at a time, one more if changes arrive meanwhile.
    fn rescan(&mut self) {
        if self.state.project.is_none() {
            return;
        }
        if self.rescanning {
            self.rescan_again = true;
            return;
        }
        self.rescanning = true;
        let walk = self.root_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(p) = &mut self.state.project else { return };
        p.begin_rescan(walk);
        let (root, latest, tx) = (p.root().to_owned(), self.root_generation.clone(), self.tx.clone());
        self.jobs.submit(Priority::Background, move || {
            project::scan(&root, walk, &latest, |batch| {
                let _ = tx.send_blocking(Completion::Scanned { generation: walk, batch });
            });
        });
    }

    /// Watch the root's folders as the inventory knows them (ignored folders are not watched) and the folders of
    /// open sources outside it.
    /// Rebuilt only when the inventory or the tabs' folders changed: a large project has thousands of folders, and
    /// navigation must not pay for them.
    fn rewatch(&mut self) {
        let project = self.state.project.as_ref().map(|p| (p.root().to_owned(), p.walk, p.files(), !p.refreshing() && !matches!(p.coverage(), project::Coverage::Scanning(_))));
        // Tabs inside the inventory's folders add nothing: opening one of the project's files costs no rebuild.
        let inside = |d: &Path| self.state.project.as_ref().is_some_and(|p| d == p.root() || d.strip_prefix(p.root()).is_ok_and(|r| p.has_dir(&r.to_string_lossy())));
        let extra: std::collections::BTreeSet<PathBuf> = self.state.tabs.iter().filter(|t| t.retrieved.is_none()).filter_map(|t| t.source.parent()).filter(|d| !inside(d)).map(Path::to_path_buf).collect();
        let key: WatchKey = (project, extra);
        if self.watching.as_ref() == Some(&key) {
            return;
        }
        self.watching = Some(key.clone());
        // Only the tabs' folders changed (or there is no project): applied now, so a file just opened outside the
        // inventory is watched before this returns.
        let reuse = match &key.0 {
            None => Some(Arc::default()),
            Some(_) => self.inventory_folders.as_ref().filter(|(i, _)| *i == key.0).map(|(_, f)| f.clone()),
        };
        if let Some(folders) = reuse {
            let mut wanted = (*folders).clone();
            wanted.extend(key.1.iter().cloned());
            return self.watch(key, wanted);
        }
        // A new inventory: its set (thousands of folders in a large project) is built off the UI thread; the
        // latest key wins.
        let (root, sorted) = self.state.project.as_ref().map_or((None, Vec::new()), |p| (Some(p.root().to_owned()), p.sorted()));
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Background, move || {
            let mut folders = std::collections::BTreeSet::new();
            if let Some(root) = root {
                folders.extend(project::folders(&sorted).map(|d| root.join(d)));
                folders.insert(root);
            }
            let _ = tx.send_blocking(Completion::Watch { key, folders: Arc::new(folders) });
        });
    }

    /// Watch a set built for the current inventory and tabs (an older one is dropped).
    fn watch(&mut self, key: WatchKey, folders: std::collections::BTreeSet<PathBuf>) {
        if self.watching.as_ref() != Some(&key) {
            return;
        }
        let n = folders.len();
        let all = self.watcher.set(folders);
        self.state.messages.retain(|m| !m.starts_with("Watching"));
        if !all {
            self.state.messages.push(format!(
                "Watching {} of {n} folders; changes elsewhere show on focus or Reload",
                crate::watch::MAX_WATCHED
            ));
        }
    }

    fn open_root(&mut self, root: PathBuf) {
        self.command(Command::CloseSearchEverywhere);
        // Opening a folder looks its repository up again (it may have become one since).
        self.state.repo_anchor = None;
        self.checkpoint_workspace();
        if self.state.project.is_some() {
            for id in [self.state.other, self.state.active].into_iter().flatten() { self.release(id); }
            for tab in &self.state.tabs { tab.find_latest.store(u64::MAX, Ordering::Relaxed); }
            self.state.tabs.clear(); self.state.active = None; self.state.other = None; self.state.focused = 0;
            self.state.history = History::default();
            self.next_request(); self.pending = None; self.state.loading = None;
            self.current.store(0, Ordering::Relaxed); self.other_current.store(0, Ordering::Relaxed);
        }
        self.restore_workspace = self.state.tabs.is_empty();
        self.state.explorer = Default::default(); self.state.workspace_revision += 1;
        self.state.libraries = Default::default();
        let root = normalized(&root);
        self.pending_goto = None;
        let generation = self.root_generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.query_latest.fetch_add(1, Ordering::Relaxed); // stops a ranking over the old root
        self.rescanning = false;
        self.rescan_again = false;
        self.rescan_changes = Change::default();
        self.overflow_fingerprints.clear();
        // Results belong to the root they searched.
        if self.state.results.as_ref().is_some_and(|r| r.scope == Scope::Project) {
            self.state.results = None;
            self.search_latest.fetch_add(1, Ordering::Relaxed);
        }
        // Project closure stops its language servers now.
        self.state.asking = None;
        self.state.semantics.stop_all();
        self.synced = 0;
        // The saver holds the old project's last save; a save for the new file writes it first.
        self.state.places = Default::default();
        self.state.missing.clear();
        self.places_file = None;
        self.state.project = Some(Project::new(root.clone(), generation, self.state.settings.folders_first));
        self.project_generation.store(generation, Ordering::Relaxed);
        self.state.index = DeclIndex::new(index::INDEX_BYTES);
        self.index_revision.fetch_add(1, Ordering::Relaxed);
        self.index_rebuilding = false;
        self.index_cache = self.state.settings.persistent_index.then(|| Arc::new(std::sync::Mutex::new(crate::index_cache::Cache::new(&self.index_cache_dir, &root))));
        self.rewatch();
        self.state.recent.retain(|r| *r != root);
        self.state.recent.insert(0, root.clone());
        self.state.recent.truncate(RECENT_ROOTS);
        let (latest, tx) = (self.root_generation.clone(), self.tx.clone());
        let (recent, file) = (self.state.recent.clone(), self.recent_file.clone());
        self.recent_seq += 1;
        let (seq, saves) = (self.recent_seq, self.recent_saves.clone());
        let places_dir = self.places_dir.clone();
        let saved = self.places_saver.flush();
        // Discovery is background work: loads of files the user is waiting for go first.
        self.jobs.submit(Priority::Background, move || {
            // Two quick root changes save from two workers: the newer list wins.
            let mut written = saves.lock().unwrap_or_else(|e| e.into_inner());
            if *written < seq {
                save_json(&file, &recent);
                *written = seq;
            }
            drop(written);
            let _ = saved.recv();
            let (file, from) = crate::places::files_for(&places_dir, &root);
            let result = crate::places::read(&from, &root);
            let _ = tx.send_blocking(Completion::PlacesRead { generation, file, from, result });
            project::scan(&root, generation, &latest, |batch| {
                let _ = tx.send_blocking(Completion::Scanned { generation, batch });
            });
        });
    }

    /// Resolve a row from the currently displayed search snapshot; adapters only present or dispatch it.
    pub fn navigation_choice(&self, revision: u64, index: usize) -> Option<crate::navigation::Chosen> {
        use crate::navigation::{Chosen, Target};
        let search = self.state.navigation.as_ref().filter(|s| s.revision == revision)?;
        let row = search.rows.get(index)?;
        match &row.target {
            Target::Recent {path, at} => (!self.state.missing.contains(path)).then(|| Chosen::Command(match at {
                Some(at) => Command::OpenAt {path:path.clone(),at:*at},
                None => Command::GoToPlace(Place::File(path.clone())),
            })),
            Target::Action(id) => self.disabled(*id).is_none().then_some(Chosen::Action(*id)),
            Target::Open {path, at} => Some(Chosen::Command(match at {
                Some(at) => Command::OpenAt {path: path.clone(), at: *at},
                None => Command::Open(path.clone()),
            })),
        }
    }

    pub fn navigation_status(&self) -> String {
        use crate::navigation::Scope as S;
        let Some(n) = &self.state.navigation else { return String::new() };
        let mut parts = vec![if n.pending { "Searching…".into() } else if n.rows.is_empty() { "No matches".into() } else if n.matched > crate::navigation::LIMIT { format!("Best {} results — type to narrow", n.rows.len()) } else { format!("{} results", n.rows.len()) }];
        if matches!(n.scope, S::All | S::Files | S::Symbols) {
            match &self.state.project {
                None => parts.push("Open a folder to search project files and symbols".into()),
                Some(p) => {
                    if !matches!(p.coverage(), project::Coverage::Complete(_)) { parts.push(format!("Scanning folder: {} files", p.files())); }
                    if matches!(n.scope, S::All | S::Symbols) {
                        parts.push("Syntactic symbol candidates".into());
                        let c = self.state.index.coverage();
                        if c.pending > 0 { parts.push(format!("Indexing symbols: {}/{} files", c.indexed, c.candidates)); }
                        if c.limited { parts.push("Symbol index reached its size limit".into()); }
                        if c.too_large + c.unreadable > 0 { parts.push(format!("{} files too large or unreadable for symbols", c.too_large + c.unreadable)); }
                        if c.syntax_errors > 0 { parts.push(format!("{} files have syntax errors; symbols may be incomplete", c.syntax_errors)); }
                    }
                }
            }
        }
        if n.scope == S::Symbols && n.query.is_empty() { parts.push("Type a symbol name".into()); }
        if n.scope == S::Recent { parts.push("Recent files · Delete forgets".into()); }
        parts.join(" · ")
    }

    fn rank_navigation(&mut self) {
        use crate::navigation::{self, Snapshot};
        let Some(n) = &mut self.state.navigation else { return };
        let request = self.navigation_latest.fetch_add(1, Ordering::Relaxed) + 1;
        n.request = request; n.pending = true; n.dirty = false;
        if n.rows.is_empty() { n.revision = request; }
        let (scope, query) = (n.scope, n.query.clone());
        let files = self.state.project.as_ref().map_or(0, |p| p.files());
        let indexed = self.state.index.coverage().indexed;
        let recent = if scope == navigation::Scope::Recent {
            self.state.place_rows(PlaceList::RecentFiles, "").into_iter().filter_map(|row| match row.place { Place::File(path) => Some(path), Place::Bookmark(..) | Place::Location(..) => None }).take(navigation::LIMIT).collect()
        } else { self.state.active().into_iter().chain(self.state.tabs.iter().rev()).map(|t| t.source().to_owned()).chain(self.state.places.recent_files().iter().cloned()).take(navigation::LIMIT).collect() };
        let snapshot = Snapshot {
            root: self.state.project.as_ref().map(|p| p.root().to_owned()),
            files: if matches!(scope, navigation::Scope::All | navigation::Scope::Files) { self.state.project.as_ref().map_or_else(Vec::new, Project::entries) } else { Vec::new() },
            symbols: if !query.is_empty() && matches!(scope, navigation::Scope::All | navigation::Scope::Symbols) { self.state.index.snapshot() } else { Vec::new() },
            recent,
            actions: if matches!(scope, navigation::Scope::All | navigation::Scope::Actions) { self.actions() } else { Vec::new() },
        };
        // Recent files are bounded to fifty entries; populate immediately for Ctrl-E followed by Enter.
        if scope == navigation::Scope::Recent {
            let found = navigation::rank(snapshot, scope, &query, || true);
            self.complete(Completion::Navigation {request,files,indexed,found});
            return;
        }
        let (latest, tx) = (self.navigation_latest.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            let found = navigation::rank(snapshot, scope, &query, || latest.load(Ordering::Relaxed) == request);
            if latest.load(Ordering::Relaxed) == request { let _ = tx.send_blocking(Completion::Navigation {request, files, indexed, found}); }
        });
    }

    /// Rank the finder's query over the files known now, off the UI thread.
    fn rank(&mut self) {
        let Some(p) = &mut self.state.project else { return };
        let Some(query) = p.finder.as_ref().map(|f| lenz_content::goto::split_path_line(&f.query).0.to_owned()) else { return };
        p.query_seq += 1;
        let seq = p.query_seq;
        self.query_latest.store(seq, Ordering::Relaxed);
        let files = p.files();
        if query.is_empty() {
            if let Some(f) = &mut p.finder {
                f.hits.clear();
                f.searched = files;
                f.ready = true;
            }
            return;
        }
        p.ranking = Some(seq);
        let (entries, generation) = (p.entries(), p.generation());
        let (latest, tx) = (self.query_latest.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            let hits = project::rank(&entries, &query, || latest.load(Ordering::Relaxed) == seq);
            let searched = entries.iter().map(|b| b.len()).sum();
            let _ = tx.send_blocking(Completion::Ranked { generation, seq, searched, hits });
        });
    }

    fn theme_effect(&mut self, effect: lenz_content::theme::ThemeEffect) -> Effect {
        self.theme_preview = self.theme_preview.take().filter(|id| id == "system" || self.themes.entries().iter().any(|entry| &entry.id == id));
        let palette = self.theme_preview.as_deref().and_then(|id| self.themes.preview(id))
            .unwrap_or_else(|| self.themes.snapshot().palette);
        let repaint = effect.repaint || *self.state.palette != *palette;
        self.state.palette = palette;
        self.state.messages.extend(effect.notice);
        Effect { repaint, copy: None }
    }

    fn next_request(&mut self) -> u64 {
        self.request += 1;
        self.latest.store(self.request, Ordering::Relaxed);
        self.request
    }

    pub fn find_options(&self) -> lenz_content::find::FindOptions {
        self.find_options
    }

    /// Why `query` cannot be searched with the session's options (a bad regular expression); project search refuses it.
    pub fn search_refusal(&self, query: &str) -> Option<String> {
        lenz_content::find::Matcher::new(&lenz_content::find::Query { text: query.to_owned(), options: self.find_options }).err().map(|e| e.to_string())
    }

    /// Search `tab`'s document for `text` with the session's options, off the UI thread; the newest request wins.
    /// Find `text` in tab `tab`, going to the match at or after the caret when it arrives if `reveal`.
    fn find_in(&mut self, tab: TabId, text: String, reveal: bool) -> Effect {
        self.find_seq += 1;
        let request = self.find_seq;
        let Some(t) = self.tab_mut(tab) else { return Effect::default() };
        let Some(doc) = t.doc.clone() else { return Effect::default() };
        // The last matches stay drawn (and counted) until the new ones arrive, when they are for this text: a new
        // version's lines may be fewer.
        let mut state = crate::find::FindState::new(text.clone(), doc.version(), request);
        state.reveal = reveal;
        if let Some(old) = t.find.take().filter(|o| o.error.is_none() && o.version == doc.version()) {
            (state.matches, state.current, state.capped) = (old.matches, old.current, old.capped);
        }
        t.find = Some(state);
        let latest = t.find_latest.clone();
        let (query, tx) = (lenz_content::find::Query { text, options: self.find_options }, self.tx.clone());
        latest.store(request, Ordering::Relaxed);
        self.jobs.submit(Priority::Visible, move || {
            let version = doc.version();
            let Some(result) = crate::find::find_all(&doc, &query, || latest.load(Ordering::Relaxed) == request) else { return };
            let _ = tx.send_blocking(Completion::Matched { tab, request, version, result });
        });
        Effect { repaint: true, ..Effect::default() }
    }

    /// The caret at `at` (1-based, clamped) in the active document, its fold open and its line brought into view;
    /// a step in history.
    fn go_to_line(&mut self, at: lenz_content::goto::LineColumn) -> Effect {
        let from = self.here();
        let Some(doc) = self.state.current() else { return Effect::default() };
        let Some(t) = self.active_mut() else { return Effect::default() };
        let line = (at.line as usize).saturating_sub(1).min(doc.line_count().saturating_sub(1));
        let range = doc.reader_line_range(line);
        let offset = doc.byte_at_column(line, at.column.unwrap_or(1).saturating_sub(1) as usize);
        let byte = offset - range.start;
        t.selection = Some(Selection { anchor: offset, head: offset });
        t.expansion = None;
        t.reveal(line);
        let id = t.id;
        self.bring_into_view(id, line, byte);
        if let Some(to) = self.here() {
            self.visit(from, to);
        }
        Effect { repaint: true, ..Effect::default() }
    }

    /// Go where [`Command::OpenAt`] asked, once its source is the active, loaded document.
    fn apply_pending_goto(&mut self) {
        let Some((path, at)) = self.pending_goto.clone() else { return };
        let Some(t) = self.state.active().filter(|t| t.source == path || t.canonical.as_ref() == Some(&path)) else { return };
        if t.doc.is_some() {
            self.pending_goto = None;
            let at = match at {
                Goto::At(at) => at,
                Goto::Mark(m) => {
                    let doc = t.doc.clone().expect("checked above");
                    let line = self.current_line_of(&m).min(doc.line_count() - 1);
                    let text = doc.line(line);
                    let column = text[..crate::places::floor(text, m.offset)].chars().count() as u32 + 1;
                    lenz_content::goto::LineColumn { line: u32::try_from(line + 1).unwrap_or(u32::MAX), column: Some(column) }
                }
            };
            self.go_to_line(at);
        }
    }

    /// Where `m` is now: its bookmark's or recent location's line after relocation (they move together with the
    /// file's text), else the line it carried.
    fn current_line_of(&self, m: &crate::places::Mark) -> usize {
        let same = |o: &&crate::places::Mark| o.source == m.source && o.anchor == m.anchor && o.mnemonic == m.mnemonic;
        self.state.places.bookmarks().iter().chain(self.state.places.recent_locations()).filter(same).min_by_key(|o| o.line.abs_diff(m.line)).map_or(m.line, |o| o.line)
    }

    /// Open `m`'s file in the focused pane and put the caret on its line (history; its fold opens).
    fn jump_to_mark(&mut self, m: crate::places::Mark) -> Effect {
        self.pending_goto = Some((normalized(&m.source), Goto::Mark(m.clone())));
        let effect = self.command(Command::Open(m.source.to_path_buf()));
        self.apply_pending_goto();
        Effect { repaint: true, ..effect }
    }

    /// Step the active tab's find to its next (or previous) match, wrapping and saying so.
    fn find_step(&mut self, forward: bool) -> Effect {
        let Some(t) = self.active_mut() else { return Effect::default() };
        let id = t.id;
        let Some(f) = t.find.as_mut().filter(|f| !f.matches.is_empty()) else { return Effect::default() };
        let Some((next, wrapped)) = lenz_content::find::next_index(f.matches.len(), f.current, forward) else { return Effect::default() };
        f.current = Some(next);
        if wrapped {
            self.state.messages.push(if forward { "Search wrapped to the first match" } else { "Search wrapped to the last match" }.into());
        }
        self.show_match(id)
    }

    /// Select `tab`'s current match and bring it into view: its fold opens, and a line off the pane is placed
    /// three rows below the top, as other navigation does.
    fn show_match(&mut self, tab: TabId) -> Effect {
        let Some(t) = self.tab_mut(tab) else { return Effect::default() };
        let Some((line, range)) = t.find.as_ref().and_then(|f| f.matches.get(f.current?).cloned()) else { return Effect::default() };
        let Some(doc) = t.doc.clone() else { return Effect::default() };
        let start = doc.line_range(line).start;
        t.selection = Some(Selection { anchor: start + range.start, head: start + range.end });
        t.expansion = None;
        t.reveal(line);
        self.bring_into_view(tab, line, range.start);
        Effect { repaint: true, ..Effect::default() }
    }

    /// Selection grows one row at a time; scroll only enough to keep its head inside the reported viewport.
    fn keep_selection_visible(&mut self) {
        use crate::layout::Query;
        let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id())) else { return };
        let Some(selection) = view.tab().selection() else { return };
        let (id, pane) = (view.tab().id(), view.tab().pane() as usize);
        let rows = self.state.text_size[pane].1.min(4096);
        if rows == 0 { return; }
        let source_line = view.snapshot().line_of(selection.head);
        let line = view.visible().shown(source_line);
        let head = if line == source_line { selection.head } else { view.snapshot().reader_line_range(line).end };
        let cols = self.state.wrap_cols(view.tab());
        let position = match view.layout_position(head, cols) {
            Query::Ready(Some(position)) => position, Query::Ready(None) => return,
            Query::Pending(work) => { self.defer_layout(id, work, LayoutPurpose::Selection); return; }
        };
        let width = self.state.text_size[pane].0;
        let column = (cols.is_none() && width > 0).then(|| {
            let old = view.tab().column();
            if position.column < old { position.column } else { old.max(position.column.saturating_add(1).saturating_sub(width)) }
        });
        let shown = view.layout_rows(rows, cols);
        let top = if shown.rows.iter().any(|row| row.line == line && row.index == position.row) { None }
        else {
            if let Some(work) = shown.pending { self.defer_layout(id, work, LayoutPurpose::Selection); return; }
            Some(match cols {
                Some(cols) => match view.layout_step((line, position.row), -(rows.saturating_sub(1) as isize), cols) {
                    Query::Ready(top) => top,
                    Query::Pending(work) => { self.defer_layout(id, work, LayoutPurpose::Selection); return; }
                },
                None => (view.visible().step(line, -(rows.saturating_sub(1) as isize)), 0),
            })
        };
        if let Some(tab) = self.tab_mut(id) {
            if let Some((line, row)) = top { tab.set_top(line, row); }
            if let Some(column) = column { tab.column = column; }
        }
        self.clear_of_band(id, line, position.row);
    }

    /// Bring a line-relative source byte into the reported viewport using exact shared rows.
    fn bring_into_view(&mut self, tab: TabId, line: usize, byte: usize) {
        use crate::layout::Query;
        let Some(view) = self.state.view(tab) else { return };
        let rows = self.state.text_size[view.tab().pane() as usize].1.min(4096);
        let cols = self.state.wrap_cols(view.tab());
        let range = view.snapshot().reader_line_range(line);
        let source = view.snapshot().floor_char_boundary(range.start.saturating_add(byte).min(range.end));
        let position = match view.layout_position(source, cols) {
            Query::Ready(Some(position)) => position, Query::Ready(None) => return,
            Query::Pending(work) => { self.defer_layout(tab, work, LayoutPurpose::Reveal { line, byte }); return; }
        };
        let shown = view.layout_rows(rows, cols);
        if rows > 0 && shown.rows.iter().any(|row| row.line == line && row.index == position.row) {
            return self.clear_of_band(tab, line, position.row);
        }
        if let Some(work) = shown.pending { self.defer_layout(tab, work, LayoutPurpose::Reveal { line, byte }); return; }
        let top = match cols {
            Some(cols) => match view.layout_step((line, position.row), -3, cols) {
                Query::Ready(top) => top,
                Query::Pending(work) => { self.defer_layout(tab, work, LayoutPurpose::Reveal { line, byte }); return; }
            },
            None => (view.visible().step(line, -3), 0),
        };
        if let Some(tab) = self.tab_mut(tab) { tab.set_top(top.0, top.1); }
        self.clear_of_band(tab, line, position.row);
    }

    /// Keep a known exact row below the sticky band. Cold layout resumes through the same bounded lane.
    fn clear_of_band(&mut self, tab: TabId, line: usize, row: usize) {
        use crate::layout::Query;
        if !self.state.settings.sticky_scroll { return; }
        let Some(view) = self.state.view(tab) else { return };
        let rows = self.state.text_size[view.tab().pane() as usize].1;
        let cap = crate::tabs::sticky_cap(rows);
        for _ in 0..=cap {
            let Some(view) = self.state.view(tab) else { return };
            let cols = self.state.wrap_cols(view.tab());
            let band = match view.layout_sticky(cap, cols) {
                Query::Ready(sticky) => sticky.len(),
                Query::Pending(work) => { self.defer_layout(tab, work, LayoutPurpose::Band { line, row }); return; }
            };
            let shown = view.layout_rows(cap + 1, cols);
            let at = shown.rows.iter().position(|shown| shown.line == line && shown.index == row);
            if at.is_none() && let Some(work) = shown.pending { self.defer_layout(tab, work, LayoutPurpose::Band { line, row }); return; }
            if at.is_none_or(|at| at >= band) || (view.tab().top_line() == 0 && view.tab().top_row() == 0) { return; }
            let top = match cols {
                Some(cols) => match view.layout_step((view.tab().top_line(), view.tab().top_row()), -1, cols) {
                    Query::Ready(top) => top,
                    Query::Pending(work) => { self.defer_layout(tab, work, LayoutPurpose::Band { line, row }); return; }
                },
                None => (view.visible().step(view.tab().top_line(), -1), 0),
            };
            if let Some(tab) = self.tab_mut(tab) { tab.set_top(top.0, top.1); }
        }
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.state.tabs.iter_mut().find(|t| t.id == id)
    }

    fn active_mut(&mut self) -> Option<&mut Tab> {
        let id = self.state.active?;
        self.tab_mut(id)
    }

    /// Where the reader is now, for history.
    fn here(&self) -> Option<Location> {
        self.state.active().map(Tab::location)
    }

    /// A committed navigation to `path`: the tab already showing it, else the preview tab (or a new retained tab
    /// when `retain`). `restore` positions the view (Back/Forward); `record` adds a history step.
    fn navigate(&mut self, requested: PathBuf, retain: bool, restore: Option<Location>, record: bool) -> Effect {
        self.navigate_to(requested, retain, restore, record, None)
    }

    /// [`Session::navigate`], optionally to source a language server returned (`retrieved`): its tab holds that
    /// snapshot and never reads the disk.
    fn navigate_to(&mut self, requested: PathBuf, retain: bool, restore: Option<Location>, record: bool, retrieved: Option<(Arc<Document>, semantics::VirtualSource)>) -> Effect {
        self.open_seq += 1; // a result still being checked must not move the view after this
        // A jump to another file that has not landed is given up: it must not fire on that file's next open.
        if self.pending_goto.as_ref().is_some_and(|(p, _)| *p != normalized(&requested)) {
            self.pending_goto = None;
        }
        if self.state.asking.take().is_some() {
            self.state.semantics.cancel(); // nor may a definition still being answered
        }
        // Retrieved source is identified by its opaque URI, never a path; once its tab is gone it is not held.
        let virtual_source = is_virtual(&requested);
        if virtual_source && retrieved.is_none() && !self.state.tabs.iter().any(|t| t.source == requested) {
            self.state.messages.push(format!("{} was retrieved from a language server and is no longer held; ask for its definition again", requested.display()));
            return Effect { repaint: true, copy: None };
        }
        // Identity is the absolute, normalized path (and the resolved path once loaded); the requested spelling is
        // what the tab shows.
        let path = if virtual_source { requested.clone() } else { normalized(&requested) };
        let from = self.here();
        // Within the focused pane: the other pane keeps its own tabs, of the same sources too.
        let pane = self.state.focused;
        let existing = self.state.tabs.iter().find(|t| t.pane == pane && (t.source == path || t.canonical.as_ref() == Some(&path))).map(|t| t.id);
        let id = match existing {
            Some(id) => id,
            None => {
                let preview = self.state.tabs.iter().find(|t| t.pane == pane && !t.retained).map(|t| t.id).filter(|_| !retain);
                match preview {
                    Some(id) => {
                        // The preview shows something else now: its old content becomes inactive.
                        self.release(id);
                        let markdown_source = !self.previewed.contains(&path);
                        let fresh = self.fresh_tab(id, &path, &requested, false, markdown_source);
                        if let Some(t) = self.tab_mut(id) {
                            *t = fresh;
                        }
                        if self.state.active == Some(id) {
                            self.state.active = None; // so activation loads the new source
                        }
                        id
                    }
                    None => {
                        self.next_tab += 1;
                        let id = TabId(self.next_tab);
                        let at = self.state.active.and_then(|a| self.state.tabs.iter().position(|t| t.id == a)).map_or(self.state.tabs.len(), |i| i + 1);
                        let tab = self.fresh_tab(id, &path, &requested, retain, !self.previewed.contains(&path));
                        self.state.tabs.insert(at, tab);
                        id
                    }
                }
            }
        };
        if let Some((doc, source)) = retrieved
            && let Some(t) = self.tab_mut(id)
        {
            t.display = PathBuf::from(format!("{} ({})", source.name(), source.provider));
            t.language = Some(doc.language());
            t.doc = Some(doc);
            t.new_content(); // what was derived from earlier text does not fit this one
            t.retrieved = Some(source);
            t.stamp = None;
            t.unavailable = None;
        }
        if retain && let Some(t) = self.tab_mut(id) {
            t.retained = true;
        }
        if self.state.tabs.iter().any(|tab| tab.id == id && tab.retrieved.is_some()) { self.admit_current(id); }
        let same = self.state.active == Some(id);
        self.activate(id);
        self.index_visible(id);
        if let Some(to) = &restore
            && let Some(t) = self.tab_mut(id)
        {
            t.restore_to(to);
        }
        if record && !same && let Some(to) = self.here() {
            self.visit(from, to);
        }
        self.rewatch();
        Effect { repaint: !same || restore.is_some() || retain, copy: None }
    }

    /// Index the shown document's declarations before the rest of the project.
    fn index_visible(&mut self, id: TabId) {
        let (Some(p), Some(t)) = (&self.state.project, self.state.tabs.iter().find(|t| t.id == id)) else { return };
        if let Some(rel) = p.relative(&t.source)
            && !self.state.index.has(&rel)
        {
            self.state.index.enqueue([rel], true);
            self.pump_index();
        }
    }

    fn cancel_for_index_rebuild(&mut self) {
        self.state.asking = None;
        self.state.semantics.cancel();
        if let Some(r) = self.state.results.as_mut().filter(|r| r.kind == ResultsKind::Symbols) {
            r.done = true;
            r.error = Some("Project index is rebuilding; find symbols again".into());
        }
    }

    fn reset_index(&mut self, clear: bool) {
        let Some(p) = &self.state.project else { return };
        let (root, generation, paths) = (p.root().to_owned(), p.generation(), p.paths());
        let revision = self.index_revision.fetch_add(1, Ordering::Relaxed) + 1;
        self.index_rebuilding = clear;
        self.state.index = DeclIndex::new(index::INDEX_BYTES);
        self.state.index.enqueue(paths, false);
        let cache = Arc::new(std::sync::Mutex::new(crate::index_cache::Cache::new(&self.index_cache_dir, &root)));
        self.index_cache = self.state.settings.persistent_index.then(|| cache.clone());
        if let Some(query) = self.state.results.as_ref().filter(|r| r.kind == ResultsKind::Declarations).map(|r| r.query.clone()) { self.find_declarations(query); }
        if let Some(n) = &mut self.state.navigation { n.rows.clear(); self.rank_navigation(); }
        if clear {
            self.overflow_fingerprints.clear(); // the replacement servers read the entire project
            self.cancel_for_index_rebuild();
            let (tx, latest) = (self.tx.clone(), self.index_revision.clone());
            self.state.messages.push("Clearing the saved project index…".into());
            self.jobs.submit(Priority::Visible, move || {
                if latest.load(Ordering::Relaxed) != revision { return; }
                let result = cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
                let _ = tx.send_blocking(Completion::IndexReset { generation, revision, result });
            });
        } else { self.pump_index(); }
    }

    /// Reuse the indexer's current reads; no second walk, file read, or hash runs on the event thread.
    fn notify_project_changes(&mut self, paths: impl IntoIterator<Item = (Arc<str>, bool)>) {
        let Some(project) = &self.state.project else { return };
        let mut changes = Vec::new();
        for (path, exists) in paths {
            if exists && let Some(old) = self.overflow_fingerprints.get(&path) {
                // Keep the baseline through registration-triggered follow-up scans of the same overflow.
                if self.rescan_changes.overflow && self.rescan_again { continue; }
                let Some(changed) = self.state.index.changed_since(&path, *old) else { continue };
                self.overflow_fingerprints.remove(&path);
                if !changed { continue; }
            } else { self.overflow_fingerprints.remove(&path); }
            changes.push((project.root().join(path.as_ref()), exists));
        }
        self.state.semantics.files_changed(&changes);
    }

    /// Start the next indexing chunk in the background, if any is due.
    fn pump_index(&mut self) {
        if self.index_rebuilding { return; }
        let Some(p) = &self.state.project else { return };
        let chunk = self.state.index.next_chunk();
        let settled = matches!(p.coverage(), project::Coverage::Complete(_)) && self.state.index.coverage().pending <= 1;
        if chunk.is_empty() && (!settled || self.index_cache.is_none() || self.state.index.running) { return; }
        let (root, generation, project, tx) = (p.root().to_owned(), p.generation(), self.project_generation.clone(), self.tx.clone());
        let (revision, latest, cache) = (self.index_revision.load(Ordering::Relaxed), self.index_revision.clone(), self.index_cache.clone());
        self.jobs.submit(Priority::Background, move || {
            let current = move || project.load(Ordering::Relaxed) == generation && latest.load(Ordering::Relaxed) == revision;
            if !current() { return; }
            let mut cache = cache.as_ref().map(|c| c.lock().unwrap_or_else(|e| e.into_inner()));
            if let Some(c) = &mut cache { c.load(); }
            if chunk.is_empty() {
                if let Some(c) = &mut cache { c.flush(current); }
                return;
            }
            let files = chunk.into_iter().map(|rel| {
                let result = index::index_file_cached(&root, &rel, current.clone(), cache.as_deref_mut());
                (rel, result)
            }).collect();
            if settled && let Some(c) = &mut cache { c.flush(current.clone()); }
            let _ = tx.send_blocking(Completion::Indexed { generation, revision, files });
        });
    }

    /// Show tab `id`, loading its content if it was released; the tab left releases its own.
    fn activate(&mut self, id: TabId) {
        // A jump to another file that has not landed is given up (a tab switched to or closed meanwhile).
        if let Some(t) = self.state.tabs.iter().find(|t| t.id == id)
            && self.pending_goto.as_ref().is_some_and(|(p, _)| t.source != *p && t.canonical.as_ref() != Some(p))
        {
            self.pending_goto = None;
        }
        if self.state.active != Some(id) {
            // A question about the tab left is not answered in another.
            if self.state.asking.take().is_some() {
                self.state.semantics.cancel();
            }
            if let Some(old) = self.state.active {
                self.release(old);
            }
            self.state.active = Some(id);
            self.state.messages.clear();
        }
        let Some(t) = self.tab_mut(id) else { return };
        match &t.doc {
            Some(doc) => {
                let doc = doc.clone();
                self.current.store(doc.version(), Ordering::Relaxed);
                self.decorate(&doc);
            }
            None => {
                let path = t.source.clone();
                if self.cache.contains(&path) {
                    // Check the kept snapshot is current off the UI thread before reusing it.
                    let (request, tx) = (self.next_request(), self.tx.clone());
                    self.pending = Some((id, false));
                    self.state.loading = Some(path.clone());
                    self.jobs.submit(Priority::Visible, move || {
                        let stamp = Stamp::of(&path);
                        let _ = tx.send_blocking(Completion::Checked { request, tab: id, path, stamp });
                    });
                } else {
                    self.load(id, false);
                }
            }
        }
    }

    fn checkpoint_workspace(&mut self) {
        if self.restore_workspace { return; } // A directory closed before its state arrived keeps the saved workspace.
        let Some(project) = &self.state.project else { return };
        let tabs: Vec<_> = self.state.tabs.iter().filter(|t| t.retrieved.is_none()).take(crate::workspace::MAX_TABS).collect();
        let index = |id| tabs.iter().position(|t| Some(t.id) == id);
        let workspace = crate::workspace::Workspace {
            active: index(self.state.active), other: index(self.state.other), focused: self.state.focused,
            split: self.state.settings.split, layout: self.state.settings.layout, soft_wrap: self.state.settings.soft_wrap,
            expanded: project.expanded(), explorer: self.state.explorer.clone(),
            tabs: tabs.into_iter().map(|t| crate::workspace::SavedTab { path: t.source.clone(), selection: t.selection, top_line: t.top_line, top_row: t.top_row(), column: t.column, pane: t.pane, retained: t.retained, markdown_source: t.markdown_source, image_source: t.image_source }).collect(),
        };
        Arc::make_mut(&mut self.state.places).workspace = Some(Box::new(workspace));
        self.places_changed();
    }

    fn apply_workspace(&mut self, saved: crate::workspace::Workspace) {
        self.state.settings.split = saved.split; self.state.settings.layout = saved.layout; self.state.settings.soft_wrap = saved.soft_wrap;
        self.state.explorer = saved.explorer; self.state.workspace_revision += 1;
        if let Some(project) = &mut self.state.project { project.restore_expanded(saved.expanded); }
        self.state.tabs = saved.tabs.into_iter().map(|s| {
            self.next_tab += 1;
            let mut tab = Tab::new(TabId(self.next_tab), s.path, s.retained);
            tab.pane = s.pane; tab.selection = s.selection; tab.set_top(s.top_line, s.top_row); tab.column = s.column;
            tab.markdown_source = s.markdown_source; tab.image_source = s.image_source;
            tab
        }).collect();
        self.state.focused = saved.focused;
        let shown = |index: Option<usize>, pane| index.and_then(|i| self.state.tabs.get(i)).filter(|t| t.pane == pane).or_else(|| self.state.tabs.iter().find(|t| t.pane == pane)).map(|t| t.id);
        let mut active = shown(saved.active, saved.focused);
        self.state.other = self.state.settings.split.and_then(|_| shown(saved.other, 1 - saved.focused));
        if active.is_none() && self.state.other.is_some() { active = self.state.other.take(); self.state.focused = 1 - saved.focused; }
        self.state.active = None;
        if let Some(active) = active { self.activate(active); }
        self.load_other();
        self.rewatch();
    }

    /// An inactive tab's content goes to the inactive cache when it can be read again; positions stay.
    /// Save the places: debounced, then merged with the file and written off the UI thread from a snapshot. Without
    /// a project, or before its file is read, they last for the session.
    fn places_changed(&mut self) {
        if let Some(n) = &mut self.state.navigation
            && n.scope == crate::navigation::Scope::Recent
        {
            n.dirty = true;
            if !n.pending { self.rank_navigation(); }
        }
        self.places_seq += 1;
        let (Some(file), Some(p)) = (&self.places_file, &self.state.project) else { return };
        let save = crate::places::Save { file: file.clone(), root: p.root().to_owned(), generation: self.project_generation.load(Ordering::Relaxed), seq: self.places_seq, places: self.state.places.clone() };
        self.places_saver.save(save);
    }

    /// The project's saved places, read from `from`, go behind what is here. Later saves go to `file` and merge from
    /// what was read, or from nothing when `from` is the file's old name (nothing is under the new one yet).
    fn apply_places(&mut self, file: PathBuf, from: &Path, saved: crate::places::Places) {
        let base = if from == file { saved.clone() } else { Default::default() };
        self.places_saver.base(file.clone(), Arc::new(base));
        Arc::make_mut(&mut self.state.places).merge(saved);
        self.places_file = Some(file);
    }

    /// A committed navigation: a history step, and both ends become recent locations.
    fn visit(&mut self, from: Option<Location>, to: Location) {
        for at in from.iter().chain([&to]) {
            if self.state.tabs.iter().any(|t| t.source == at.source && shows_text(t) && !t.before_root) {
                Arc::make_mut(&mut self.state.places).visited(crate::places::Mark::from_location(at));
            }
        }
        self.state.history.visit(from, to);
        self.places_changed();
    }

    /// The caret's line in the active tab, when it shows text.
    fn caret_mark(&self) -> Option<crate::places::Mark> {
        let t = self.state.active().filter(|t| shows_text(t))?;
        let doc = t.doc.as_ref()?;
        let at = t.selection.map_or_else(|| doc.line_range(t.top_line.min(doc.line_count() - 1)).start, |s| s.head);
        let line = doc.line_of(at);
        Some(crate::places::Mark::at(&t.source, doc, line, at - doc.line_range(line).start))
    }

    fn release(&mut self, id: TabId) {
        if let Some(t) = self.state.tabs.iter().find(|t| t.id == id).filter(|t| t.retrieved.is_none() && !t.before_root) {
            let (source, at) = (t.source.clone(), shows_text(t).then(|| t.location()));
            Arc::make_mut(&mut self.state.places).left_file(&source);
            if let Some(at) = at {
                Arc::make_mut(&mut self.state.places).visited(crate::places::Mark::from_location(&at));
            }
            self.places_changed();
        }
        if self.state.other == Some(id) {
            self.other_current.store(0, Ordering::Relaxed);
        }
        if self.state.active == Some(id) {
            self.next_request(); // supersedes its pending load
            self.state.loading = None;
            self.pending = None;
            self.current.store(0, Ordering::Relaxed);
        }
        let indexed_bytes = self.state.view(id).map_or(0, |view| view.snapshot().storage_bytes());
        let token = self.next_refresh();
        let Some(t) = self.tab_mut(id) else { return };
        t.refresh = token; // a check started while it was shown must not put content back
        // Only reloadable content is released: an unavailable source's last content stays with its tab.
        if t.stamp.is_none() {
            return;
        }
        let at = t.location();
        t.new_content();
        t.restore = Some(at);
        if let (Some(doc), Some(stamp)) = (t.doc.take(), t.stamp.take()) {
            self.cache.put_current(stamp, doc, indexed_bytes);
        }
        self.release_current();
    }

    fn close_tab(&mut self, id: TabId) -> Effect {
        let Some(pane) = self.state.tabs.iter().find(|t| t.id == id).map(|t| t.pane) else {
            return Effect::default();
        };
        self.release(id);
        self.remove_tab(id, false);
        self.left_pane(pane)
    }

    /// After the user closed or moved a tab out of `pane`: a split pane left without tabs closes the split.
    fn left_pane(&mut self, pane: u8) -> Effect {
        if self.state.settings.split.is_some() && !self.state.tabs.iter().any(|t| t.pane == pane) {
            return self.command(Command::Unsplit);
        }
        self.rewatch();
        Effect { repaint: true, copy: None }
    }

    /// Remove a tab; when it was active, the nearest remaining tab becomes active (not a history step). `keep_messages`
    /// keeps the reason it went.
    fn remove_tab(&mut self, id: TabId, keep_messages: bool) {
        let Some(i) = self.state.tabs.iter().position(|t| t.id == id) else { return };
        let pane = self.state.tabs.remove(i).pane;
        if self.state.active == Some(id) {
            self.state.active = None;
            let messages = std::mem::take(&mut self.state.messages);
            if let Some(next) = self.nearest(pane, i) {
                self.activate(next);
            }
            if keep_messages {
                self.state.messages = messages;
            }
        }
        if self.state.other == Some(id) {
            self.state.other = self.nearest(pane, i);
            self.show_other();
        }
        self.release_current();
    }

    /// The tab of pane `pane` nearest position `i` of the tab list: the first at or after it, else the last before.
    fn nearest(&self, pane: u8, i: usize) -> Option<TabId> {
        let tabs = &self.state.tabs;
        tabs.iter().skip(i).find(|t| t.pane == pane).or_else(|| tabs.iter().take(i).rev().find(|t| t.pane == pane)).map(|t| t.id)
    }

    /// A new tab for `path` in the focused pane; a source the other pane shows lends it its snapshot (and place).
    fn fresh_tab(&mut self, id: TabId, path: &Path, requested: &Path, retained: bool, markdown_source: bool) -> Tab {
        let pane = self.state.focused;
        let mut tab = match self.state.tabs.iter().find(|t| t.pane != pane && t.source == path && t.doc.is_some() && t.retrieved.is_none()) {
            Some(twin) => twin.duplicate(id, pane),
            None => {
                let mut tab = Tab::new(id, path.to_path_buf(), retained);
                tab.pane = pane;
                tab.markdown_source = markdown_source;
                tab
            }
        };
        tab.retained = retained;
        tab.display = requested.to_path_buf();
        tab
    }

    fn is_shown(&self, id: TabId) -> bool {
        self.state.active == Some(id) || self.state.other == Some(id)
    }

    /// The shown tabs whose document is `version`.
    fn shown_mut(&mut self, version: u64) -> impl Iterator<Item = &mut Tab> {
        let (active, other) = (self.state.active, self.state.other);
        self.state.tabs.iter_mut().filter(move |t| (Some(t.id) == active || Some(t.id) == other) && t.doc.as_ref().is_some_and(|d| d.version() == version))
    }

    /// Record that shown tab `id` shows `version`, so work on it is wanted.
    fn mark_shown(&self, id: TabId, version: u64) {
        if self.state.active == Some(id) {
            self.current.store(version, Ordering::Relaxed);
        }
        if self.state.other == Some(id) {
            self.other_current.store(version, Ordering::Relaxed);
        }
    }

    /// Work on `version` is wanted while either pane shows it.
    fn wanted(&self, version: u64) -> impl Fn() -> bool + Send + Clone + 'static {
        let (current, other) = (self.current.clone(), self.other_current.clone());
        move || current.load(Ordering::Relaxed) == version || other.load(Ordering::Relaxed) == version
    }

    /// The other pane's shown tab: decorated when in memory, else read once nothing else is loading.
    fn show_other(&mut self) {
        let Some(id) = self.state.other else {
            self.other_current.store(0, Ordering::Relaxed);
            return;
        };
        match self.state.tabs.iter().find(|t| t.id == id).and_then(|t| t.doc.clone()) {
            Some(doc) => {
                self.other_current.store(doc.version(), Ordering::Relaxed);
                self.decorate(&doc);
            }
            None => self.load_other(),
        }
    }

    /// Read the other pane's shown tab when it has no content and no load is in flight (one load at a time: the
    /// focused pane's goes first, this one follows it).
    fn load_other(&mut self) {
        let Some(id) = self.state.other.filter(|_| self.pending.is_none()) else { return };
        if self.state.tabs.iter().any(|t| t.id == id && t.doc.is_none() && t.retrieved.is_none()) {
            self.load(id, false);
        }
    }

    /// Give keys and commands to pane `pane`: the panes' shown tabs trade roles, both stay in memory. `false` when
    /// nothing changed.
    fn focus_pane(&mut self, pane: u8) -> bool {
        if pane == self.state.focused || pane > 1 || self.state.settings.split.is_none() {
            return false;
        }
        if self.state.asking.take().is_some() {
            self.state.semantics.cancel(); // a question about the pane left is not answered in the other
        }
        self.open_seq += 1;
        std::mem::swap(&mut self.state.active, &mut self.state.other);
        let left = self.current.swap(self.other_current.load(Ordering::Relaxed), Ordering::Relaxed);
        self.other_current.store(left, Ordering::Relaxed);
        self.state.focused = pane;
        self.state.messages.clear();
        if let Some(id) = self.state.active {
            self.activate(id); // reads it when it has no content yet
        }
        self.load_other();
        true
    }

    fn move_tab(&mut self, id: TabId, pane: u8, index: usize) -> Effect {
        let Some(from) = self.state.tabs.iter().find(|t| t.id == id).map(|t| t.pane) else { return Effect::default() };
        if pane > 1 || (pane == 1 && self.state.settings.split.is_none()) {
            return Effect::default();
        }
        if from != pane {
            // It moves as what its pane shows.
            self.focus_pane(from);
            if self.state.active != Some(id) {
                self.activate(id);
            }
            let source = self.state.tabs.iter().find(|t| t.id == id).map(|t| t.source.clone());
            if let Some(twin) = self.state.tabs.iter().find(|t| t.pane == pane && Some(&t.source) == source.as_ref()).map(|t| t.id) {
                // That pane has this source already: its tab is shown there instead.
                self.release(id);
                self.remove_tab(id, false);
                self.focus_pane(pane);
                self.activate(twin);
                return self.left_pane(from);
            }
        }
        let Some(i) = self.state.tabs.iter().position(|t| t.id == id) else { return Effect::default() };
        let mut tab = self.state.tabs.remove(i);
        tab.pane = pane;
        let places: Vec<usize> = self.state.tabs.iter().enumerate().filter(|(_, t)| t.pane == pane).map(|(i, _)| i).collect();
        let at = places.get(index).copied().unwrap_or_else(|| places.last().map_or(self.state.tabs.len(), |l| l + 1));
        self.state.tabs.insert(at, tab);
        if from != pane {
            // Its old pane shows its nearest tab; its new pane shows it, focused; what that pane showed is released.
            let left = self.nearest(from, i);
            let replaced = self.state.other.replace(id);
            self.state.active = None;
            if let Some(old) = replaced {
                self.release(old);
            }
            self.state.other = left;
            self.state.active = Some(id);
            self.state.focused = pane;
            let version = self.state.tabs.iter().find(|t| t.id == id).map_or(0, |t| t.version());
            self.current.store(version, Ordering::Relaxed);
            self.activate(id);
            self.show_other();
            return self.left_pane(from);
        }
        Effect { repaint: true, copy: None }
    }

    /// A decoration setting changed: what tabs hold of `kind` goes (a hint kind turned off or on: all hints are asked
    /// again with the new kinds), work in flight is dropped, and turning it on says when the server cannot.
    fn decorations_changed(&mut self, kind: Decor, on: bool) -> Effect {
        self.save_settings();
        self.decor_generation[kind as usize] += 1;
        self.state.semantics.forget(kind);
        for t in &mut self.state.tabs {
            match kind {
                Decor::Hints => {
                    t.hints = None;
                    t.set_top(t.top_line, 0); // wrapped rows change
                }
                Decor::Tokens => t.tokens = None,
            }
        }
        if on && let Some(name) = self.state.document().and_then(|d| self.state.semantics.lacks(d, kind)) {
            self.state.messages.push(match kind {
                Decor::Hints => format!("No inlay hints from {name}"),
                Decor::Tokens => format!("No semantic tokens from {name}"),
            });
        }
        Effect { repaint: true, copy: None }
    }

    fn save_settings(&mut self) {
        let (settings, file) = (self.state.settings.clone(), self.settings_file.clone());
        self.settings_seq += 1;
        let (seq, saves) = (self.settings_seq, self.settings_saves.clone());
        self.jobs.submit(Priority::Background, move || {
            // Two quick changes save from two workers: the newer one wins.
            let mut written = saves.lock().unwrap_or_else(|e| e.into_inner());
            if *written < seq {
                save_json(&file, &settings);
                *written = seq;
            }
        });
    }

    /// Read and decode tab `id`'s source off the UI thread; a load superseded before it starts reads nothing.
    fn load(&mut self, id: TabId, reload: bool) {
        let Some(path) = self.state.tabs.iter().find(|t| t.id == id).map(|t| t.source.clone()) else { return };
        let request = self.next_request();
        self.pending = Some((id, reload));
        self.state.loading = Some(path.clone());
        let (latest, versions, tx) = (self.latest.clone(), self.versions.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            if latest.load(Ordering::Relaxed) != request {
                return;
            }
            let result = match read(&path) {
                Ok((stamp, bytes)) => {
                    let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok((Some(stamp), Arc::new(Document::decode(path.clone(), version, &bytes)), std::fs::canonicalize(&path).ok()))
                }
                Err(ReadError::Unstable) => Err("it kept changing while being read; try Reload".to_string()),
                Err(ReadError::Failed(e)) => Err(e),
            };
            let _ = tx.send_blocking(Completion::Loaded { request, tab: id, path, result });
        });
    }

    fn show(&mut self, id: TabId, doc: Arc<Document>, stamp: Option<Stamp>, reload: bool) {
        let token = self.next_refresh(); // a check started before this load is now stale
        let Some(t) = self.tab_mut(id) else { return };
        t.refresh = token;
        t.stale_on_disk = false;
        t.changing = 0;
        t.language = Some(doc.language());
        t.doc = Some(doc.clone());
        t.stamp = stamp;
        t.unavailable = None;
        t.new_content();
        // Positions kept while released, or across a reload, stay valid for this text.
        let text = doc.text().clone();
        t.selection = t.selection.map(|s| Selection { anchor: floor_char_boundary(&text, s.anchor), head: floor_char_boundary(&text, s.head) });
        t.top_line = t.top_line.min(doc.line_count() - 1);
        if let Some(at) = t.restore.take().filter(|_| !reload) {
            t.restore_to(&at);
        }
        let source = t.source.clone();
        self.admit_current(id);
        if self.is_shown(id) {
            self.mark_shown(id, doc.version());
            self.decorate(&doc);
        }
        if self.state.places.has_marks(&source) && Arc::make_mut(&mut self.state.places).relocate(&source, &doc) {
            self.places_changed();
        }
        self.apply_pending_goto();
        // An open find follows the new content.
        if let Some(text) = self.tab_mut(id).and_then(|t| t.find.as_ref()).map(|f| f.query.clone()) {
            self.find_in(id, text, true);
        }
        self.annotate(id);
    }

    /// Blames tab `id` when its annotations are on and are not of its content as it is (new, changed or reloaded).
    fn annotate(&mut self, id: TabId) {
        self.blame_requests += 1;
        let (request, tx) = (self.blame_requests, self.tx.clone());
        let Some(t) = self.tab_mut(id) else { return };
        let (Some(doc), Some(b)) = (t.doc.as_ref(), t.blame.as_mut()) else { return };
        if b.of(t.stamp) || t.retrieved.is_some() {
            return;
        }
        let file = t.canonical.clone().unwrap_or_else(|| t.source.clone());
        b.start(id, file, t.stamp, doc.line_count(), request, tx);
    }

    /// Work derived from the active document: syntax spans, and a Markdown file's preview.
    fn decorate(&mut self, doc: &Arc<Document>) {
        self.highlight(doc);
        self.render(doc);
    }

    /// The active Markdown document's preview, made off the UI thread (what the user waits to see), then its local
    /// images: files inside the browsing root, or beside the document when it is outside the root.
    fn render(&mut self, doc: &Arc<Document>) {
        let wants = |t: &&Tab| !t.markdown_source && t.preview.is_none() && t.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, doc));
        if doc.language() != Some(Language::Markdown) || !self.state.active().into_iter().chain(self.state.shown(1 - self.state.focused)).any(|t| wants(&t)) {
            return;
        }
        let root = match self.state.project.as_ref().map(|p| p.root()).filter(|r| doc.source().starts_with(r)) {
            Some(root) => root.to_path_buf(),
            None => doc.source().parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        let (version, text, file, shown, tx) = (doc.version(), doc.text().clone(), doc.source().to_path_buf(), self.wanted(doc.version()), self.tx.clone());
        let (renders, render) = (self.renders.clone(), self.renders.load(Ordering::Relaxed));
        self.jobs.submit(Priority::Visible, move || {
            let wanted = move || shown() && renders.load(Ordering::Relaxed) == render;
            if !wanted() {
                return;
            }
            let preview = crate::markdown::render(text, wanted.clone());
            let doc = preview.as_ref().ok().map(|p| p.doc.clone());
            if !wanted() || tx.send_blocking(Completion::Rendered { version, preview }).is_err() {
                return;
            }
            if let Some(doc) = doc {
                let images = crate::markdown::images(&doc, &file, &root, &wanted);
                if wanted() {
                    let _ = tx.send_blocking(Completion::PreviewImages { version, doc, images });
                }
            }
        });
    }

    /// Colour the previewed snapshot off the UI thread, once per snapshot however often the selection moves in it; a
    /// parse for a snapshot no longer previewed stops.
    fn highlight_preview(&mut self) {
        let wanted = self.state.results.as_ref().and_then(|r| r.preview.as_ref()).filter(|p| p.highlights.is_none());
        let Some((doc, lang)) = wanted.and_then(|p| Some((p.doc.clone(), p.doc.language().filter(|_| !p.doc.is_large())?))) else {
            if self.preview_parsing.take().is_some() {
                self.preview_parse.fetch_add(1, Ordering::Relaxed);
            }
            return;
        };
        let version = doc.version();
        if self.preview_parsing.is_some_and(|(v, _)| v == version) {
            return; // already being parsed
        }
        let request = self.preview_parse.fetch_add(1, Ordering::Relaxed) + 1;
        self.preview_parsing = Some((version, request));
        let (latest, tx) = (self.preview_parse.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            if latest.load(Ordering::Relaxed) != request {
                return;
            }
            let current = latest.clone();
            let parsed = highlight::parse_while(lang, doc.text().clone(), move || current.load(Ordering::Relaxed) == request);
            let _ = tx.send_blocking(Completion::PreviewHighlighted { request, version, parsed });
        });
    }

    /// Begin an expansion of tab `id`'s selection `from` through `steps`, taking the first.
    fn start_expansion(&mut self, id: TabId, version: u64, from: Selection, steps: Vec<std::ops::Range<usize>>, reveal: bool) -> Effect {
        let Some(t) = self.tab_mut(id) else { return Effect::default() };
        let mut e = crate::expand::Expansion { version, origin: from, steps, at: 0 };
        let next = e.extend();
        t.expansion = Some(e);
        match next {
            Some(s) => {
                t.selection = Some(s);
                if reveal {
                    t.show_head();
                }
                Effect { repaint: true, copy: None }
            }
            None => Effect::default(),
        }
    }

    /// After a declarations setting changed: save, and parse the shown tabs again where declarations are now wanted
    /// but their parse had none (spans and folds come back with them).
    fn settings_changed_declarations(&mut self) -> Effect {
        self.save_settings();
        if self.state.wants_declarations() {
            let shown = [self.state.active, self.state.other];
            let mut docs = Vec::new();
            for t in self.state.tabs.iter_mut().filter(|t| shown.contains(&Some(t.id))) {
                let Some(doc) = t.doc.clone().filter(|d| d.language().is_some_and(highlight::has_declarations)) else { continue };
                if t.highlights.is_some() && t.declarations.is_none() {
                    t.highlights = None;
                    docs.push(doc);
                }
            }
            for doc in docs {
                self.highlight(&doc);
            }
        }
        Effect { repaint: true, copy: None }
    }

    fn highlight(&mut self, doc: &Arc<Document>) {
        let shown = [self.state.active, self.state.other];
        if !self.state.tabs.iter().any(|t| shown.contains(&Some(t.id)) && t.highlights.is_none() && t.doc.as_ref().is_some_and(|d| Arc::ptr_eq(d, doc))) {
            return;
        }
        if let Some(lang) = doc.language().filter(|_| !doc.is_large()) {
            let (version, text, wanted, tx) = (doc.version(), doc.text().clone(), self.wanted(doc.version()), self.tx.clone());
            let declarations = self.state.wants_declarations() && highlight::has_declarations(lang);
            self.jobs.submit(Priority::Background, move || {
                if !wanted() {
                    return;
                }
                let want = highlight::Want { folds: true, declarations };
                let (parsed, folds, declarations) = highlight::parse_with(lang, text, want, wanted).map_or((None, None, None), |s| (Some(s.parsed), s.folds, s.declarations));
                let _ = tx.send_blocking(Completion::Highlighted { version, parsed, folds, declarations });
            });
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // Places made before the project's file was read: read it now and save them over it (here wins). When it
        // cannot be read, nothing is written.
        if let Some(root) = self.state.project.as_ref().filter(|_| self.places_file.is_none()).map(|p| p.root().to_owned()) {
            let (file, from) = crate::places::files_for(&self.places_dir, &root);
            if let Ok(saved) = crate::places::read(&from, &root) {
                self.apply_places(file, &from, saved);
                self.places_changed();
            }
        }
        self.checkpoint_workspace();
        // The shown file is left at exit; the saver (a field) drops after this body and writes it.
        if let Some(id) = self.state.active {
            self.release(id);
        }
        // Before the job pool joins its workers: a worker blocked sending into a full channel returns, and running
        // scans and rankings stop.
        self.layouts.cancel_all();
        self.projections.cancel_all();
        self.rx.close();
        self.navigation_latest.fetch_add(1, Ordering::Relaxed);
        self.root_generation.fetch_add(1, Ordering::Relaxed);
        self.query_latest.fetch_add(1, Ordering::Relaxed);
        self.latest.fetch_add(1, Ordering::Relaxed);
        self.search_latest.fetch_add(1, Ordering::Relaxed);
        self.preview_parse.store(u64::MAX, Ordering::Relaxed);
        self.expand_latest.store(u64::MAX, Ordering::Relaxed);
        self.project_generation.fetch_add(1, Ordering::Relaxed);
    }
}

/// `$XDG_CACHE_HOME`, else `~/.cache`; relative values are ignored.
fn xdg_cache() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| PathBuf::from("/dev/null"));
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".cache"))
}

/// `$XDG_CONFIG_HOME`, else `~/.config`; relative values are ignored.
fn xdg_config() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| PathBuf::from("/dev/null"));
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"))
}

/// `$XDG_STATE_HOME`, else `~/.local/state`; relative values are ignored, as Lenz does.
fn xdg_state() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| PathBuf::from("/dev/null"));
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".local/state"))
}

/// Recent roots from `file`: a JSON array of paths, at most 64 KiB; anything else is no history.
fn read_recent(file: &Path) -> Vec<PathBuf> {
    let Ok(bytes) = lenz_content::theme::read_bounded(file) else { return Vec::new() };
    let paths: Vec<PathBuf> = serde_json::from_slice(&bytes).unwrap_or_default();
    paths.into_iter().filter(|p| p.is_absolute()).take(RECENT_ROOTS).collect()
}

/// The reader's settings as saved.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Settings {
    persistent_index: bool,
    folders_first: bool,
    documentation_on_hover: bool,
    /// File-type icons beside file names (Nerd Font glyphs in a terminal).
    file_icons: bool,
    /// Long lines wrap at the pane's width.
    soft_wrap: bool,
    /// The enclosing declarations stay pinned at the top of each pane.
    sticky_scroll: bool,
    /// A bar above each pane's text shows the path and the declarations at the caret.
    breadcrumbs: bool,
    /// Language-server inlay hints naming types.
    inlay_types: bool,
    /// Language-server inlay hints naming parameters.
    inlay_parameters: bool,
    /// Language-server token colours over the syntax colours.
    semantic_highlighting: bool,
    split: Option<Split>,
    layout: Layout,
}

impl Default for Settings {
    fn default() -> Self {
        Self { persistent_index: true, folders_first: true, documentation_on_hover: true, file_icons: true, soft_wrap: false, sticky_scroll: true, breadcrumbs: true, inlay_types: true, inlay_parameters: true, semantic_highlighting: true, split: None, layout: Layout::default() }
    }
}

/// Settings from `file` (at most 64 KiB of JSON); missing or unreadable ones are the defaults.
fn read_settings(file: &Path) -> Settings {
    lenz_content::theme::read_bounded(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Best effort, atomically: a failed save loses history or a setting, never a project file.
fn save_json(file: &Path, value: &impl serde::Serialize) {
    if let Ok(json) = serde_json::to_vec_pretty(value) {
        crate::places::write_atomic(file, &json);
    }
}

/// Whether `t` shows text lines from a file (not an image, a Markdown preview or a language server's source).
fn shows_text(t: &Tab) -> bool {
    t.doc.as_ref().is_some_and(|d| d.image().is_none() || t.image_source) && !matches!(t.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) && t.retrieved.is_none()
}

/// What tab content shown as `shown` (read as `stamp`) is now on disk at `path`.
fn check(path: &Path, stamp: Option<Stamp>, shown: Option<&Arc<Document>>, versions: &AtomicU64) -> Refresh {
    match (Stamp::of(path), shown) {
        (None, _) => Refresh::Gone("deleted or moved".into()),
        (Some(now), _) if Some(now) == stamp => Refresh::Same(now),
        (Some(_), None) => Refresh::Released,
        (Some(_), Some(old)) => match read(path) {
            // An image whose bytes are the same needs no decode to say so.
            Ok((now, bytes)) if old.image().is_some() && Document::fingerprint(&bytes) == old.content_fingerprint() => Refresh::Same(now),
            Ok((now, bytes)) => {
                let version = versions.fetch_add(1, Ordering::Relaxed) + 1;
                let doc = Document::decode(path.to_owned(), version, &bytes);
                if doc.same_content(old) { Refresh::Same(now) } else { Refresh::Changed(now, Arc::new(doc)) }
            }
            Err(ReadError::Unstable) => Refresh::Unstable,
            Err(ReadError::Failed(e)) => Refresh::Gone(e),
        },
    }
}

enum ReadError {
    /// The file changed during every attempt.
    Unstable,
    Failed(String),
}

/// Read a regular file whose stamp is the same before and after the read: a read that races a replacement is
/// retried, never presented as the confirmed current content.
fn read(path: &Path) -> Result<(Stamp, Vec<u8>), ReadError> {
    // A language server's opaque URI is never reinterpreted as a path.
    if is_virtual(path) {
        return Err(ReadError::Failed("it is source a language server returned, not a file".into()));
    }
    for attempt in 0..READ_ATTEMPTS {
        let meta = std::fs::metadata(path).map_err(|e| ReadError::Failed(e.to_string()))?;
        if !meta.is_file() {
            return Err(ReadError::Failed("not a regular file".into()));
        }
        let before = Stamp::from(&meta);
        let bytes = std::fs::read(path).map_err(|e| ReadError::Failed(e.to_string()))?;
        if Stamp::of(path) == Some(before) {
            return Ok((before, bytes));
        }
        std::thread::sleep(std::time::Duration::from_millis(10 << attempt));
    }
    Err(ReadError::Unstable)
}

/// Whether `path` is a language server's opaque URI rather than a file path: `scheme://…` with a URI scheme of
/// at least two characters (not a drive letter) other than `file`.
fn is_virtual(path: &Path) -> bool {
    path.to_str().and_then(|s| s.split_once("://")).is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme != "file"
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
}

/// An absolute path without `.` and `..` (lexically: the requested path stays the displayed one; symlinks are
/// resolved only where identity is checked).
/// What a new window opens at launch: the argument (a file or a folder, relative to `cwd`), else the folder it was
/// started in, unless that is the home folder (where desktop launchers start apps) or the filesystem root.
pub fn launch_target(arg: Option<PathBuf>, cwd: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(arg) = arg {
        return Some(match &cwd {
            Some(cwd) if arg.is_relative() => normalized(&cwd.join(arg)),
            _ => arg,
        });
    }
    let cwd = normalized(&cwd?);
    (cwd.parent().is_some() && home.is_none_or(|h| normalized(&h) != cwd)).then_some(cwd)
}

fn normalized(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut out = PathBuf::new();
    for c in absolute.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// Where `head` goes by `motion` in `doc`.
fn layout_intent_current(state: &State, intent: &LayoutIntent) -> bool {
    layout_current(state, &intent.request) && state.view(intent.request.tab).is_some_and(|view| {
        view.tab().selection() == intent.selection && (view.tab().top_line(), view.tab().top_row()) == intent.top
            && Arc::ptr_eq(&view.visible(), &intent.visible)
            && (!matches!(intent.purpose, LayoutPurpose::Command(_) | LayoutPurpose::Selection) || state.active().is_some_and(|tab| tab.id() == intent.request.tab))
    })
}

fn layout_current(state: &State, request: &crate::layout::Request) -> bool {
    state.shown(request.pane).is_some_and(|tab| tab.id() == request.tab)
        && state.view(request.tab).is_some_and(|view| request.work.matches(view.snapshot(), state.wrap_cols(view.tab()), view.inlay_hints()))
}

fn projection_current(state: &State, request: &crate::projection::Request) -> bool {
    state.shown(request.pane).is_some_and(|tab| tab.id() == request.tab)
        && state.view(request.tab).is_some_and(|view| request.cursor.matches(view.snapshot(), view.inlay_hints()))
}

fn moved(doc: &crate::text::TextSnapshot, v: &crate::folds::Visible, head: usize, motion: Motion) -> usize {
    let line = doc.line_of(head);
    let range = doc.reader_line_range(line);
    let column = |l: usize| {
        let r = doc.reader_line_range(l);
        doc.floor_char_boundary((r.start + (head - range.start)).min(r.end))
    };
    match motion {
        Motion::Left => doc.previous_char_boundary(head),
        Motion::Right => doc.next_char_boundary(head),
        // Up and Down skip collapsed lines.
        Motion::Up if line > 0 => column(v.step(line, -1)),
        Motion::Up => range.start,
        Motion::Down if v.step(line, 1) > line => column(v.step(line, 1)),
        Motion::Down => range.end,
        Motion::LineStart => range.start,
        Motion::LineEnd => range.end,
    }
}

/// The head moved one visual row up or down, keeping its display column (collapsed lines skipped).
fn layout_moved(view: &View<'_>, head: usize, down: bool, cols: usize) -> crate::layout::Query<usize> {
    use crate::layout::Query;
    let position = match view.layout_position(head, Some(cols)) {
        Query::Ready(Some(position)) => position, Query::Ready(None) => return Query::Ready(head), Query::Pending(work) => return Query::Pending(work),
    };
    let (line, index) = match view.layout_step((position.line, position.row), if down { 1 } else { -1 }, cols) {
        Query::Ready(target) => target, Query::Pending(work) => return Query::Pending(work),
    };
    let row = match view.layout_row(line, index, Some(cols)) {
        Query::Ready(Some(row)) => row, Query::Ready(None) => return Query::Ready(head), Query::Pending(work) => return Query::Pending(work),
    };
    match view.layout_byte_at(&row, position.column, Some(cols)) {
        Query::Ready(Some(at)) if at == head && (line, index) != (position.line, position.row) => Query::Ready(if down { view.snapshot().next_char_boundary(head) } else { view.snapshot().previous_char_boundary(head) }),
        Query::Ready(at) => Query::Ready(at.unwrap_or(head)), Query::Pending(work) => Query::Pending(work),
    }
}

/// A syntax-built level: each usage that is one side of a relation in its file gives the other side (subtypes: the
/// usage is a supertype name; supertypes: it is a subtype name). Files over the large-file limit have no relations
/// read; files without a relation query give nothing.
/// Work for a level another has replaced since (`latest` moved past `request`) stops.
fn syntax_level(usages: Vec<semantics::Target>, direction: Direction, encoding: semantics::lsp::Encoding, latest: Arc<AtomicU64>, request: u64) -> Vec<(Call, semantics::Target)> {
    let current = move || latest.load(Ordering::Relaxed) == request;
    if !current() {
        return Vec::new();
    }
    let mut by_file: std::collections::BTreeMap<PathBuf, Vec<semantics::Target>> = Default::default();
    for t in usages {
        by_file.entry(t.path.clone()).or_default().push(t);
    }
    let mut out = Vec::new();
    for (path, hits) in by_file {
        let doc = hits[0].doc.clone();
        let Some(lang) = doc.language().filter(|_| !doc.is_large()) else { continue };
        let Some(relations) = lenz_content::highlight::type_relations_while(lang, doc.text().clone(), current.clone()) else {
            if current() {
                continue;
            }
            return Vec::new();
        };
        for t in &hits {
            let at = t.at;
            for r in &relations {
                let (on, other, decl) = match direction {
                    Direction::Subtypes => (&r.sup, &r.sub, r.sub_kind),
                    Direction::Supertypes => (&r.sub, &r.sup, r.sup_kind),
                    Direction::Callers | Direction::Callees => continue,
                };
                if !on.contains(&at) {
                    continue;
                }
                let uri = semantics::lsp::file_uri(&path);
                let range = serde_json::json!({ "start": semantics::lsp::position(&doc, other.start, encoding), "end": semantics::lsp::position(&doc, other.end, encoding) });
                let name = doc.text()[other.clone()].to_owned();
                let call = Call { item: serde_json::json!({ "uri": uri, "selectionRange": range, "name": name }), name, kind: 0, decl: Some(decl), calls: 1, at: serde_json::json!({ "uri": uri, "range": range }) };
                // The row goes to the child's name, on the relation's header.
                let hit = semantics::hit_at(&path, &doc, other.start, other.end);
                out.push((call, semantics::Target { path: path.clone(), stamp: t.stamp, doc: doc.clone(), hit, at: other.start }));
            }
        }
    }
    out
}

fn floor_char_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// "<server> does not provide type hierarchy", as a disabled reason (`&'static str` like every other): made once per
/// provider name and kept for the process (at most one short string per provider).
fn no_type_hierarchy(name: &'static str) -> &'static str {
    static MADE: std::sync::Mutex<Vec<(&'static str, &'static str)>> = std::sync::Mutex::new(Vec::new());
    let mut made = MADE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(_, why)) = made.iter().find(|(n, _)| *n == name) {
        return why;
    }
    let why: &'static str = format!("{name} does not provide type hierarchy").leak();
    made.push((name, why));
    why
}

/// The kind of the declaration whose name holds byte `at`: the narrowest (an impl's name is its whole header, so
/// impls are skipped).
fn root_kind(found: &highlight::Declarations, at: usize) -> Option<highlight::DeclKind> {
    found
        .items
        .iter()
        .filter(|d| d.kind != highlight::DeclKind::Impl && (d.name_range.start as usize..d.name_range.end as usize).contains(&at))
        .min_by_key(|d| d.name_range.end - d.name_range.start)
        .map(|d| d.kind)
}

#[cfg(test)]
mod tests;
