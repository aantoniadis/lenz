//! Resolve catalog entries once for every frontend; adapters own only interface presentation.
use lenz_content::find::FindOption;

use crate::{
    semantics::hierarchy::Direction,
    session::{Action, Command, CommandId, Session, Split},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogAction {
    Disabled(&'static str),
    Command(Command),
    Interface(InterfaceAction),
}

/// A panel the keys and menus resize; the step is the adapter's (cells or pixels), the result a shared
/// [`crate::session::Command::SetLayout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Explorer,
    Results,
    /// The first (left or top) of the split file panes.
    FirstPane,
}

/// Interface requests whose inputs and layout belong to the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceAction {
    /// Grow or shrink a panel by one step, within the limits dragging its edge keeps.
    Resize { panel: Panel, grow: bool },
    RenameTerminal(crate::terminal::Id),
    PasteTerminalPath,
    SearchEverywhere,
    FindAction,
    FindSymbol,
    Open,
    /// Ask for a new path, then dispatch [`crate::session::Command::SaveAs`] for the active tab.
    SaveAs,
    NewScratch,
    ScratchFiles,
    /// Ask for a name in the explorer's folder, then dispatch [`crate::session::Command::CreateEntry`].
    Create(crate::create::Kind),
    /// Ask for a new name or folder for the explorer's entry, then dispatch [`crate::session::Command::RelocateEntry`].
    Relocate(crate::relocate::Kind),
    /// Confirm deleting the explorer's entry, then dispatch [`crate::session::Command::DeleteEntry`].
    ConfirmDelete(crate::trash::Removal),
    PasteExplorerEntry,
    ExplorerHistory,
    OpenFolder,
    OpenRecent,
    FindFile,
    SearchProject,
    FindInFile,
    GoToLine,
    FindDeclarations,
    Symbols,
    SetBookmarkMnemonic,
    ShowBookmarks,
    RecentFiles,
    RecentLocations,
    ChooseTheme,
    About,
    OpenInEditor,
    OpenWithDefaultApp,
    CompareWith,
    Paste,
    ShowCommitDiff,
    /// Ask for a new branch's name (from HEAD).
    NewBranch,
    /// Ask for a tag or revision to check out.
    CheckoutRevision,
    /// Start Lenz Diff on the conflicts a Smart Checkout left.
    ResolveConflicts,
    /// Start Lenz Diff on the working tree against HEAD (`Session::git_diff_launch`).
    ShowLocalChanges,
    /// Start Lenz Diff on the working tree against the base branch's tip.
    CompareWithBase,
    /// Start Lenz Diff on the two explorer entries marked (with the highlighted one).
    CompareSelected,
    /// Start Lenz Diff on the current branch's changes since its base.
    ShowBranchChanges,
    NewWindow,
    CloseWindow,
    QuitAll,
    FocusBreadcrumbs,
    ToggleExplorer,
    WorkspaceRoot,
    LanguageServer,
    ServerDetails,
    ServerRuntime,
    StatusDetails,
    /// The diagnostics of the version shown (`Session::problems_details`).
    Problems,
    EditingPreferences,
    /// The active file's SQL dialect, its project default and what can assist it.
    SqlDialect,
    ConfigureSqlMetadata,
    /// Every command's chord, its preset and any displaced binding: rebinding captures the next key.
    Keymap,
}

impl Session {
    /// Window navigation controls shared by the GUI and terminal toolbars.
    pub fn navigation(&self) -> Vec<(CommandId, &'static str)> {
        let state = self.state();
        let explorer = state
            .has_tree()
            .then_some((CommandId::ToggleExplorer, ""));
        let mut buttons: Vec<_> = explorer
            .into_iter()
            .chain([(CommandId::Back, "←"), (CommandId::Forward, "→")])
            .collect();
        if self.disabled(CommandId::ImageSource).is_none() {
            buttons.push((
                CommandId::ImageSource,
                if state.active().is_some_and(|t| t.image_source()) {
                    "Image"
                } else {
                    "Source"
                },
            ));
        }
        buttons
    }

    /// The Markdown toggle belongs to the file shown in this pane, including an unfocused pane.
    pub fn preview_control(&self, pane: u8) -> Option<Action> {
        let tab = self.state().shown(pane).filter(|t| t.comparison_side().is_none()).filter(|t| self.state().view(t.id()).is_some_and(|v| v.rendering().is_some()))?;
        Some(Action {
            id: CommandId::MarkdownSource,
            label: if tab.markdown_source() { "Preview" } else { "Source" },
            disabled: matches!(tab.preview, Some(Err(_))).then_some("This document cannot be previewed (the notice says why)"),
            on: None,
            shortcut: None,
        })
    }

    /// Resolve against the current state, including availability and toggle values. Dispatch a returned command
    /// immediately: do not retain it across state changes.
    pub fn action(&self, id: CommandId) -> CatalogAction {
        if let Some(why) = self.disabled(id) {
            return CatalogAction::Disabled(why);
        }
        let state = self.state();
        let command = match id {
            CommandId::ShowTerminal | CommandId::NewTerminal | CommandId::FocusTerminal | CommandId::FocusEditor | CommandId::SendSelectionToTerminal | CommandId::PasteTerminalPath | CommandId::CloseTerminal | CommandId::StopTerminal | CommandId::RenameTerminal | CommandId::RestartTerminal | CommandId::NextTerminal | CommandId::PreviousTerminal | CommandId::CopyTerminalSelection | CommandId::FindTerminal => return self.terminal_action(id),
            CommandId::CsvTable => Command::Csv(crate::csv::Command::Toggle),
            CommandId::CsvHeader => Command::Csv(crate::csv::Command::ToggleHeader),
            CommandId::CsvFilter => Command::Csv(crate::csv::Command::BeginFilter),
            CommandId::CsvSort => Command::Csv(crate::csv::Command::Sort),
            CommandId::CsvClearFilter => Command::Csv(crate::csv::Command::ClearFilter),
            CommandId::CsvMoveLeft => Command::Csv(crate::csv::Command::MoveColumn { right: false }),
            CommandId::CsvMoveRight => Command::Csv(crate::csv::Command::MoveColumn { right: true }),
            CommandId::CsvDropColumn => Command::Csv(crate::csv::Command::DropColumn),
            CommandId::CsvEditCell => Command::Csv(crate::csv::Command::BeginEdit),
            CommandId::StatusDetails => return CatalogAction::Interface(InterfaceAction::StatusDetails),
            CommandId::ToggleActivityLog => Command::ToggleActivityLog,
            CommandId::CopyActivityLog => Command::CopyActivityLog,
            CommandId::ClearActivityLog => Command::ClearActivityLog,
            CommandId::ShowProblems => Command::ShowProblems,
            CommandId::CheckProjectSyntax => Command::CheckProjectSyntax,
            CommandId::CancelProjectSyntax => Command::CancelProjectSyntax,
            CommandId::Keymap => return CatalogAction::Interface(InterfaceAction::Keymap),
            CommandId::RelativeLineNumbers => Command::SetRelativeLineNumbers(!state.relative_line_numbers()),
            CommandId::VimMode => Command::SetVimEnabled(!state.vim_enabled()),
            CommandId::KeymapIntelliJ => Command::SetKeymapPreset(crate::keymap::KeymapPreset::IntelliJ),
            CommandId::KeymapVsCode => Command::SetKeymapPreset(crate::keymap::KeymapPreset::VsCode),
            CommandId::ResetKeymap => Command::ResetKeymap,
            CommandId::EditingPreferences => return CatalogAction::Interface(InterfaceAction::EditingPreferences),
            CommandId::InspectChange => Command::InspectChange,
            CommandId::ShowChangeDiff => Command::ShowChangeDiff,
            CommandId::RevertChange => Command::RevertChange,
            CommandId::CloseChange => Command::CloseChange,
            CommandId::ConfigureSqlMetadata => return CatalogAction::Interface(InterfaceAction::ConfigureSqlMetadata),
            CommandId::ConnectSqlMetadata => Command::ConnectSqlMetadata,
            CommandId::RefreshSqlMetadata => Command::RefreshSqlMetadata,
            CommandId::DisconnectSqlMetadata => Command::DisconnectSqlMetadata,
            CommandId::SqlDialect => return CatalogAction::Interface(InterfaceAction::SqlDialect),
            CommandId::SqlDialectPostgreSql => Command::SetSqlDialect(Some(crate::sql_dialect::Dialect::PostgreSql)),
            CommandId::SqlDialectDuckDb => Command::SetSqlDialect(Some(crate::sql_dialect::Dialect::DuckDb)),
            CommandId::SqlDialectFollowProject => Command::SetSqlDialect(None),
            CommandId::ProjectSqlDialectPostgreSql => Command::SetProjectSqlDialect(Some(crate::sql_dialect::Dialect::PostgreSql)),
            CommandId::ProjectSqlDialectDuckDb => Command::SetProjectSqlDialect(Some(crate::sql_dialect::Dialect::DuckDb)),
            CommandId::ProjectSqlDialectUnset => Command::SetProjectSqlDialect(None),
            CommandId::SearchEverywhere => return CatalogAction::Interface(InterfaceAction::SearchEverywhere),
            CommandId::FindAction => return CatalogAction::Interface(InterfaceAction::FindAction),
            CommandId::FindSymbol => return CatalogAction::Interface(InterfaceAction::FindSymbol),
            CommandId::ServerRuntime => return CatalogAction::Interface(InterfaceAction::ServerRuntime),
            CommandId::ServerDetails => return CatalogAction::Interface(InterfaceAction::ServerDetails),
            CommandId::LanguageServer => return CatalogAction::Interface(InterfaceAction::LanguageServer),
            CommandId::Open => return CatalogAction::Interface(InterfaceAction::Open),
            CommandId::OpenInEditor => return CatalogAction::Interface(InterfaceAction::OpenInEditor),
            CommandId::OpenWithDefaultApp => return CatalogAction::Interface(InterfaceAction::OpenWithDefaultApp),
            CommandId::CompareSaved => Command::CompareSaved,
            CommandId::ResolveExternalChange => Command::ResolveExternalChange,
            CommandId::CloseSavedComparison => Command::CloseSavedComparison,
            CommandId::CompareWith => return CatalogAction::Interface(InterfaceAction::CompareWith),
            CommandId::ShowCommitDiff => return CatalogAction::Interface(InterfaceAction::ShowCommitDiff),
            CommandId::NewBranch => return CatalogAction::Interface(InterfaceAction::NewBranch),
            CommandId::CheckoutRevision => return CatalogAction::Interface(InterfaceAction::CheckoutRevision),
            CommandId::ResolveConflicts => return CatalogAction::Interface(InterfaceAction::ResolveConflicts),
            CommandId::ShowLocalChanges => return CatalogAction::Interface(InterfaceAction::ShowLocalChanges),
            CommandId::CompareWithRevision => Command::OpenRevisions,
            CommandId::CompareWithBase => return CatalogAction::Interface(InterfaceAction::CompareWithBase),
            CommandId::CompareSelected => return CatalogAction::Interface(InterfaceAction::CompareSelected),
            CommandId::ShowBranchChanges => return CatalogAction::Interface(InterfaceAction::ShowBranchChanges),
            CommandId::CompareWithWorktree => Command::OpenWorktrees,
            CommandId::GitBranches => Command::OpenBranches,
            CommandId::OpenFolder => return CatalogAction::Interface(InterfaceAction::OpenFolder),
            CommandId::OpenRecent => return CatalogAction::Interface(InterfaceAction::OpenRecent),
            CommandId::FindFile => return CatalogAction::Interface(InterfaceAction::FindFile),
            CommandId::SearchProject => return CatalogAction::Interface(InterfaceAction::SearchProject),
            CommandId::FindInFile if state.active().and_then(|tab| state.csv(tab.id())).is_some() => Command::Csv(crate::csv::Command::BeginFilter),
            CommandId::FindInFile => return CatalogAction::Interface(InterfaceAction::FindInFile),
            CommandId::GoToLine => return CatalogAction::Interface(InterfaceAction::GoToLine),
            CommandId::FindDeclarations => return CatalogAction::Interface(InterfaceAction::FindDeclarations),
            CommandId::Symbols => return CatalogAction::Interface(InterfaceAction::Symbols),
            CommandId::SetBookmarkMnemonic => return CatalogAction::Interface(InterfaceAction::SetBookmarkMnemonic),
            CommandId::ShowBookmarks => return CatalogAction::Interface(InterfaceAction::ShowBookmarks),
            CommandId::RecentFiles => return CatalogAction::Interface(InterfaceAction::RecentFiles),
            CommandId::RecentLocations => return CatalogAction::Interface(InterfaceAction::RecentLocations),
            CommandId::ChooseTheme => return CatalogAction::Interface(InterfaceAction::ChooseTheme),
            CommandId::About => return CatalogAction::Interface(InterfaceAction::About),
            CommandId::CloseWindow => return CatalogAction::Interface(InterfaceAction::CloseWindow),
            CommandId::QuitAll => return CatalogAction::Interface(InterfaceAction::QuitAll),
            CommandId::NewWindow => return CatalogAction::Interface(InterfaceAction::NewWindow),
            CommandId::FocusBreadcrumbs => return CatalogAction::Interface(InterfaceAction::FocusBreadcrumbs),
            CommandId::ToggleExplorer => return CatalogAction::Interface(InterfaceAction::ToggleExplorer),
            CommandId::WorkspaceRoot => return CatalogAction::Interface(InterfaceAction::WorkspaceRoot),
            CommandId::NextMatch => Command::FindStep { forward: true },
            CommandId::PreviousMatch => Command::FindStep { forward: false },
            CommandId::CloseFind => Command::CloseFind,
            CommandId::MatchCase => Command::ToggleFindOption(FindOption::Case),
            CommandId::WholeWord => Command::ToggleFindOption(FindOption::Word),
            CommandId::RegularExpression => Command::ToggleFindOption(FindOption::Regex),
            CommandId::ReplaceMatch => Command::ReplaceFind { all: false },
            CommandId::ReplaceAll => Command::ReplaceFind { all: true },
            CommandId::ListMatches => Command::ListMatches,
            CommandId::SoftWrap => Command::SetSoftWrap(!state.soft_wrap()),
            CommandId::StickyScroll => Command::SetStickyScroll(!state.sticky_scroll()),
            CommandId::Breadcrumbs => Command::SetBreadcrumbs(!state.breadcrumbs()),
            CommandId::Outline => Command::Outline,
            CommandId::RerunSearch => Command::RerunSearch,
            CommandId::CloseResults => Command::CloseResults,
            CommandId::ToggleResults => Command::ToggleResults,
            CommandId::StopSearch => Command::StopSearch,
            CommandId::FoldAllResults => Command::FoldAllResults,
            CommandId::NextResult => Command::StepResult { forward: true },
            CommandId::PreviousResult => Command::StepResult { forward: false },
            CommandId::PreviousResults => Command::ResultsHistory { back: true },
            CommandId::NextResults => Command::ResultsHistory { back: false },
            CommandId::Back => Command::Back,
            CommandId::Forward => Command::Forward,
            CommandId::ToggleBookmark => Command::ToggleBookmark,
            CommandId::NextBookmark => Command::StepBookmark { forward: true },
            CommandId::PreviousBookmark => Command::StepBookmark { forward: false },
            CommandId::PinTab => match state.active() {
                Some(tab) => Command::Pin(tab.id()),
                None => return CatalogAction::Disabled("No tab is open"),
            },
            CommandId::NextTab | CommandId::PreviousTab => {
                let tabs: Vec<_> = state.pane_tabs(state.focused_pane()).map(|t| t.id()).collect();
                let Some(at) = state.active().and_then(|a| tabs.iter().position(|&id| id == a.id())) else {
                    return CatalogAction::Disabled("No tab is open");
                };
                let by = if id == CommandId::NextTab { 1 } else { tabs.len() - 1 };
                Command::ActivateTab(tabs[(at + by) % tabs.len()])
            }
            CommandId::Reload => Command::Reload,
            CommandId::Close => Command::Close,
            CommandId::SelectAll => Command::SelectAll,
            CommandId::Copy => Command::Copy,
            CommandId::Cut => Command::Cut,
            CommandId::Paste => return CatalogAction::Interface(InterfaceAction::Paste),
            CommandId::Delete => Command::Delete { backward: false, word: false },
            CommandId::ReadOnlyMode => Command::SetReadOnly(!state.active().and_then(|t| state.view(t.id())).is_some_and(|view| view.read_only())),
            CommandId::SaveAs => return CatalogAction::Interface(InterfaceAction::SaveAs),
            // An untitled document has no file yet: Save chooses one.
            CommandId::Save if state.active().is_some_and(crate::tabs::Tab::untitled) => return CatalogAction::Interface(InterfaceAction::SaveAs),
            CommandId::NewDocument if state.project().is_some() => return CatalogAction::Interface(InterfaceAction::Create(crate::create::Kind::File)),
            CommandId::NewDocument => Command::NewDocument,
            CommandId::NewScratch => return CatalogAction::Interface(InterfaceAction::NewScratch),
            CommandId::ScratchFiles => return CatalogAction::Interface(InterfaceAction::ScratchFiles),
            CommandId::CloseAllTabs | CommandId::CloseOtherTabs | CommandId::CloseTabsLeft | CommandId::CloseTabsRight => {
                let (Some(tab), Some(scope)) = (state.active(), id.close_scope()) else { return CatalogAction::Disabled("No tab is open") };
                Command::CloseTabs { anchor: tab.id(), scope }
            },
            CommandId::RestoreDeleted => Command::RestoreDeleted,
            CommandId::NewExplorerFile => return CatalogAction::Interface(InterfaceAction::Create(crate::create::Kind::File)),
            CommandId::NewExplorerFolder => return CatalogAction::Interface(InterfaceAction::Create(crate::create::Kind::Folder)),
            CommandId::RenameExplorerEntry => return CatalogAction::Interface(InterfaceAction::Relocate(crate::relocate::Kind::Rename)),
            CommandId::MoveExplorerEntry => return CatalogAction::Interface(InterfaceAction::Relocate(crate::relocate::Kind::Move)),
            CommandId::DeleteExplorerEntry => return CatalogAction::Interface(InterfaceAction::ConfirmDelete(crate::trash::Removal::Trash)),
            CommandId::DeleteExplorerPermanently => return CatalogAction::Interface(InterfaceAction::ConfirmDelete(crate::trash::Removal::Permanent)),
            CommandId::RecoverDrafts => Command::ShowRecoveredDrafts,
            CommandId::Save | CommandId::RecheckSave | CommandId::ConfirmRecheckedSave | CommandId::SaveWithoutFormatting => match state.active() {
                Some(tab) => match id { CommandId::Save => Command::Save { tab: tab.id() }, CommandId::SaveWithoutFormatting => Command::SaveWithoutFormatting { tab: tab.id() }, CommandId::RecheckSave => Command::RecheckSave { tab: tab.id() }, _ => Command::ConfirmRecheckedSave { tab: tab.id() } },
                None => return CatalogAction::Disabled("No file is open"),
            },
            CommandId::Undo | CommandId::Redo => match state.active() {
                Some(tab) if id == CommandId::Undo => Command::Undo { tab: tab.id() },
                Some(tab) => Command::Redo { tab: tab.id() },
                None => return CatalogAction::Disabled("No tab is open"),
            },
            CommandId::Collapse => Command::Collapse,
            CommandId::Expand => Command::Expand,
            CommandId::CollapseAll => Command::CollapseAll,
            CommandId::ExpandAll => Command::ExpandAll,
            CommandId::CollapseCommentsImports => Command::CollapseCommentsImports,
            CommandId::ExtendSelection => Command::ExtendSelection,
            CommandId::ShrinkSelection => Command::ShrinkSelection,
            CommandId::AddCursorBelow => Command::AddCursorBelow,
            CommandId::AddCursorAbove => Command::AddCursorAbove,
            CommandId::SelectNextOccurrence => Command::SelectNextOccurrence,
            CommandId::CollapseSelections => Command::CollapseSelections,
            CommandId::IndentSelection => Command::Indent { outdent: false },
            CommandId::OutdentSelection => Command::Indent { outdent: true },
            CommandId::ToggleComment => Command::ToggleComment,
            CommandId::MoveLinesUp => Command::MoveLines { down: false },
            CommandId::MoveLinesDown => Command::MoveLines { down: true },
            CommandId::DuplicateLinesUp => Command::DuplicateLines { down: false },
            CommandId::DuplicateLinesDown => Command::DuplicateLines { down: true },
            CommandId::Definition => Command::Definition { peek: false },
            CommandId::PeekDefinition => Command::Definition { peek: true },
            CommandId::TypeDefinition => Command::TypeDefinition,
            CommandId::Implementation => Command::Implementation,
            CommandId::Usages => Command::Usages,
            CommandId::CallHierarchy => Command::CallHierarchy(Direction::Callers),
            CommandId::CallHierarchyCallers => Command::HierarchyDirection(Direction::Callers),
            CommandId::CallHierarchyCallees => Command::HierarchyDirection(Direction::Callees),
            CommandId::TypeHierarchy => Command::TypeHierarchy(Direction::Subtypes),
            CommandId::TypeHierarchySupertypes => Command::HierarchyDirection(Direction::Supertypes),
            CommandId::TypeHierarchySubtypes => Command::HierarchyDirection(Direction::Subtypes),
            CommandId::Documentation => Command::Documentation,
            CommandId::DocumentationOnHover => Command::SetDocumentationOnHover(!state.documentation_on_hover()),
            CommandId::Completion => Command::Complete,
            CommandId::ParameterInfo => Command::ParameterInfo,
            CommandId::QuickFix => Command::QuickFix,
            CommandId::FormatDocument => Command::Format(crate::session::FormatKind::Format),
            CommandId::OrganizeImports => Command::Format(crate::session::FormatKind::OrganizeImports),
            CommandId::NextProblem => Command::StepProblem(true),
            CommandId::PreviousProblem => Command::StepProblem(false),
            CommandId::AutoCompletion => Command::SetAutoCompletion(!state.auto_completion()),
            CommandId::FileIcons => Command::SetFileIcons(!state.file_icons()),
            CommandId::AnnotateBlame => Command::SetAnnotate(state.active().is_none_or(|t| t.blame.is_none())),
            CommandId::NarrowAnnotations => Command::ResizeAnnotations { width: state.layout().annotation_cols().saturating_sub(2), commit: true },
            CommandId::WidenAnnotations => Command::ResizeAnnotations { width: state.layout().annotation_cols() + 2, commit: true },
            CommandId::CloseAnnotations => Command::SetAnnotate(false),
            CommandId::CopyRevision => Command::CopyRevision,
            CommandId::ShowCommit => match state.active().and_then(|t| Some((t.id(), state.view(t.id())?.snapshot().line_of(t.selection()?.head)))) {
                Some((tab, line)) => Command::ShowCommitAt { tab, line },
                None => return CatalogAction::Disabled("No line is selected"),
            },
            CommandId::InlayTypes => Command::SetInlayTypes(!state.inlay_types()),
            CommandId::InlayParameters => Command::SetInlayParameters(!state.inlay_parameters()),
            CommandId::SemanticHighlighting => Command::SetSemanticHighlighting(!state.semantic_highlighting()),
            CommandId::CopyExplorerName => Command::CopyExplorerPath { kind: crate::session::PathKind::Name },
            CommandId::CopyExplorerFullPath => Command::CopyExplorerPath { kind: crate::session::PathKind::Full },
            CommandId::CopyExplorerRelativePath => Command::CopyExplorerPath { kind: crate::session::PathKind::Relative },
            CommandId::CopyExplorerEntry => Command::CopyExplorerEntry,
            CommandId::CancelExplorerCopy => Command::CancelExplorerCopy,
            CommandId::PasteExplorerEntry => return CatalogAction::Interface(InterfaceAction::PasteExplorerEntry),
            CommandId::ExplorerHistory => return CatalogAction::Interface(InterfaceAction::ExplorerHistory),
            CommandId::CopyPath => Command::CopyPath { line: false },
            CommandId::CopyPathLine => Command::CopyPath { line: true },
            CommandId::FileSymbols => Command::FileSymbols,
            CommandId::CopyServerDetails => Command::CopyText(self.language_server_details()),
            CommandId::InstallServer => Command::OpenServerInstall,
            CommandId::StartServerInstall => Command::StartServerInstall,
            CommandId::CancelServerInstall => Command::CancelServerInstall,
            CommandId::RestartServer => Command::RestartServer,
            CommandId::ExcludeRubyTooling => Command::SetRubyToolingExclusions(!state.setting(id).unwrap_or(false)),
            CommandId::PersistentIndex => Command::SetPersistentIndex(!state.setting(id).unwrap_or(false)),
            CommandId::IndentTabs => Command::SetProjectIndent(Some(crate::preferences::Indent::Tabs)),
            CommandId::IndentTwoSpaces => Command::SetProjectIndent(Some(crate::preferences::Indent::Spaces(2))),
            CommandId::IndentFourSpaces => Command::SetProjectIndent(Some(crate::preferences::Indent::Spaces(4))),
            CommandId::IndentInherit => Command::SetProjectIndent(None),
            CommandId::FormatOnSave => Command::SetFormatOnSave(!state.setting(id).unwrap_or(false)),
            CommandId::RebuildIndex => Command::RebuildIndex,
            CommandId::ResetProjectServerSettings => Command::ResetProjectServerSettings,
            CommandId::DisableServer => Command::SetServerEnabled(false),
            CommandId::EnableServer => Command::SetServerEnabled(true),
            CommandId::ImageSource => Command::ToggleImageSource,
            CommandId::MarkdownSource => Command::ToggleMarkdownSource,
            CommandId::PreviewBeside => Command::PreviewBeside,
            CommandId::ShowInSource => Command::ShowInSource,
            CommandId::FollowLink => Command::FollowLink,
            CommandId::LocateFile => Command::Locate,
            CommandId::FoldersFirst => Command::SetFoldersFirst(true),
            CommandId::AllByName => Command::SetFoldersFirst(false),
            CommandId::SplitRight => Command::Split(Split::Columns),
            CommandId::SplitDown => Command::Split(Split::Rows),
            CommandId::Unsplit => Command::Unsplit,
            CommandId::WidenExplorer | CommandId::NarrowExplorer => return CatalogAction::Interface(InterfaceAction::Resize { panel: Panel::Explorer, grow: id == CommandId::WidenExplorer }),
            CommandId::TallerResults | CommandId::ShorterResults => return CatalogAction::Interface(InterfaceAction::Resize { panel: Panel::Results, grow: id == CommandId::TallerResults }),
            CommandId::EnlargeFirstPane | CommandId::EnlargeSecondPane => return CatalogAction::Interface(InterfaceAction::Resize { panel: Panel::FirstPane, grow: id == CommandId::EnlargeFirstPane }),
            CommandId::FocusOtherPane => Command::FocusPane(1 - state.focused_pane()),
            CommandId::MoveTabToOtherPane => match state.active() {
                Some(tab) => Command::MoveTab { tab: tab.id(), pane: 1 - state.focused_pane(), index: usize::MAX },
                None => return CatalogAction::Disabled("No tab is open"),
            },
        };
        CatalogAction::Command(command)
    }

    /// Seed find from a single selected line, then the focused tab's previous query.
    pub fn find_seed(&self) -> String {
        let state = self.state();
        state.active().and_then(|tab| state.view(tab.id())).and_then(|view| {
            let range = view.selected_range()?;
            // Check line identity before copying: a multiline selection must not be assembled for Find.
            (view.snapshot().line_of(range.start) == view.snapshot().line_of(range.end)).then(|| view.copy_selection()).flatten()
        })
            .or_else(|| state.active().and_then(|t| t.find()).map(|f| f.query().to_owned()))
            .unwrap_or_default()
    }
}

/// The same command groups in the app menu and command catalogs of both adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandGroup {
    Terminal,
    File,
    View,
    Navigate,
    Find,
    Selection,
    Git,
    LanguageServers,
    Sql,
    Application,
    Recovery,
    FileLocation,
    New,
    Tabs,
    Compare,
    Layout,
    Appearance,
    Folding,
    Hints,
    Explorer,
    Csv,
    Annotations,
    Branches,
    GitCompare,
    Changes,
    Bookmarks,
    CodeNavigation,
    Problems,
    FindOptions,
    Selections,
    Lines,
    EditingSettings,
    ServerSetup,
    Metadata,
    Dialect,
    Keybindings,
}

impl CommandGroup {
    pub const ALL: [Self; 36] = [Self::File, Self::View, Self::Navigate, Self::Find, Self::Selection, Self::Git, Self::LanguageServers, Self::Sql, Self::Application, Self::Recovery, Self::FileLocation, Self::New, Self::Tabs, Self::Compare, Self::Layout, Self::Appearance, Self::Folding, Self::Hints, Self::Explorer, Self::Csv, Self::Annotations, Self::Branches, Self::GitCompare, Self::Changes, Self::Bookmarks, Self::CodeNavigation, Self::Problems, Self::FindOptions, Self::Selections, Self::Lines, Self::EditingSettings, Self::ServerSetup, Self::Metadata, Self::Dialect, Self::Keybindings, Self::Terminal];

    /// The menu a group's Back row returns to: the application's commands sit in the root menu, so its submenus
    /// return there.
    pub fn menu_parent(self) -> Option<Self> {
        self.parent().filter(|p| *p != Self::Application)
    }

    pub fn parent(self) -> Option<Self> {
        match self {
            Self::Terminal => Some(Self::View),
            Self::Recovery | Self::FileLocation => Some(Self::File),
            Self::New => Some(Self::File),
            Self::Tabs => Some(Self::File),
            Self::Compare => Some(Self::File),
            Self::Layout => Some(Self::View),
            Self::Appearance => Some(Self::View),
            Self::Folding => Some(Self::View),
            Self::Hints => Some(Self::View),
            Self::Explorer => Some(Self::File),
            Self::Csv => Some(Self::View),
            Self::Annotations => Some(Self::Git),
            Self::Branches => Some(Self::Git),
            Self::GitCompare => Some(Self::Git),
            Self::Changes => Some(Self::Git),
            Self::Bookmarks => Some(Self::Navigate),
            Self::CodeNavigation => Some(Self::Navigate),
            Self::Problems => Some(Self::Navigate),
            Self::FindOptions => Some(Self::Find),
            Self::Selections => Some(Self::Selection),
            Self::Lines => Some(Self::Selection),
            Self::EditingSettings => Some(Self::Selection),
            Self::ServerSetup => Some(Self::LanguageServers),
            Self::Metadata => Some(Self::Sql),
            Self::Dialect => Some(Self::Sql),
            Self::Keybindings => Some(Self::Application),
            Self::File | Self::View | Self::Navigate | Self::Find | Self::Selection | Self::Git | Self::LanguageServers | Self::Sql | Self::Application => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::File => "File",
            Self::View => "View",
            Self::Navigate => "Navigate",
            Self::Find => "Find",
            Self::Selection => "Edit",
            Self::Git => "Git",
            Self::LanguageServers => "Language Servers",
            Self::Sql => "SQL",
            Self::Application => "Application",
            Self::Recovery => "Save Recovery",
            Self::FileLocation => "File Location",
            Self::New => "New",
            Self::Tabs => "Tabs",
            Self::Compare => "Compare",
            Self::Layout => "Layout",
            Self::Appearance => "Editor Appearance",
            Self::Folding => "Folding",
            Self::Hints => "Code Hints",
            Self::Explorer => "Explorer",
            Self::Csv => "CSV Table",
            Self::Annotations => "Annotations",
            Self::Branches => "Branches",
            Self::GitCompare => "Compare Revisions",
            Self::Changes => "Marked Changes",
            Self::Bookmarks => "Bookmarks",
            Self::CodeNavigation => "Code Navigation",
            Self::Problems => "Problems",
            Self::FindOptions => "Find Options",
            Self::Selections => "Selections and Cursors",
            Self::Lines => "Lines and Indentation",
            Self::EditingSettings => "Editing Preferences",
            Self::ServerSetup => "Setup and Indexing",
            Self::Metadata => "Metadata Connection",
            Self::Dialect => "Dialect",
            Self::Keybindings => "Keyboard Shortcuts",

        }
    }
}

impl CommandId {
    pub fn group(self) -> CommandGroup {
        use CommandGroup as G;
        match self {
            Self::ShowTerminal | Self::NewTerminal | Self::FocusTerminal | Self::FocusEditor | Self::SendSelectionToTerminal | Self::PasteTerminalPath | Self::CloseTerminal | Self::StopTerminal | Self::RenameTerminal | Self::RestartTerminal | Self::NextTerminal | Self::PreviousTerminal | Self::CopyTerminalSelection | Self::FindTerminal => return G::Terminal,
            Self::RecoverDrafts | Self::RecheckSave | Self::ConfirmRecheckedSave | Self::SaveWithoutFormatting | Self::ResolveExternalChange => return G::Recovery,
            Self::CopyPath | Self::CopyPathLine | Self::OpenInEditor | Self::OpenWithDefaultApp => return G::FileLocation,
            Self::NewWindow => return G::New,
            Self::ReadOnlyMode => return G::EditingSettings,
            Self::NewDocument | Self::NewExplorerFile | Self::NewExplorerFolder | Self::NewScratch | Self::ScratchFiles => return G::New,
            Self::CloseAllTabs | Self::CloseOtherTabs | Self::CloseTabsLeft | Self::CloseTabsRight | Self::PinTab => return G::Tabs,
            Self::CompareWith | Self::CompareSelected | Self::CompareSaved | Self::CloseSavedComparison => return G::Compare,
            Self::SplitRight | Self::SplitDown | Self::Unsplit | Self::FocusOtherPane | Self::MoveTabToOtherPane | Self::ToggleExplorer | Self::WidenExplorer | Self::NarrowExplorer | Self::TallerResults | Self::ShorterResults | Self::EnlargeFirstPane | Self::EnlargeSecondPane | Self::ToggleActivityLog | Self::CopyActivityLog | Self::ClearActivityLog => return G::Layout,
            Self::RelativeLineNumbers | Self::SoftWrap | Self::StickyScroll | Self::Breadcrumbs | Self::FileIcons | Self::ImageSource | Self::MarkdownSource | Self::PreviewBeside => return G::Appearance,
            Self::Collapse | Self::Expand | Self::CollapseAll | Self::ExpandAll | Self::CollapseCommentsImports => return G::Folding,
            Self::DocumentationOnHover | Self::AutoCompletion | Self::InlayTypes | Self::InlayParameters | Self::SemanticHighlighting => return G::Hints,
            Self::LocateFile | Self::CopyExplorerName | Self::CopyExplorerFullPath | Self::CopyExplorerRelativePath | Self::CopyExplorerEntry | Self::PasteExplorerEntry | Self::CancelExplorerCopy | Self::ExplorerHistory | Self::RenameExplorerEntry | Self::MoveExplorerEntry | Self::DeleteExplorerEntry | Self::DeleteExplorerPermanently | Self::RestoreDeleted | Self::FoldersFirst | Self::AllByName => return G::Explorer,
            Self::CsvTable | Self::CsvHeader | Self::CsvFilter | Self::CsvSort | Self::CsvClearFilter | Self::CsvMoveLeft | Self::CsvMoveRight | Self::CsvDropColumn | Self::CsvEditCell => return G::Csv,
            Self::AnnotateBlame | Self::CloseAnnotations | Self::NarrowAnnotations | Self::WidenAnnotations | Self::ShowCommit | Self::ShowCommitDiff | Self::CopyRevision => return G::Annotations,
            Self::GitBranches | Self::NewBranch | Self::CheckoutRevision => return G::Branches,
            Self::CompareWithRevision | Self::CompareWithBase | Self::ShowBranchChanges | Self::CompareWithWorktree => return G::GitCompare,
            Self::InspectChange | Self::ShowChangeDiff | Self::RevertChange | Self::CloseChange => return G::Changes,
            Self::ToggleBookmark | Self::SetBookmarkMnemonic | Self::ShowBookmarks | Self::NextBookmark | Self::PreviousBookmark => return G::Bookmarks,
            Self::Definition | Self::PeekDefinition | Self::TypeDefinition | Self::Implementation | Self::CallHierarchy | Self::CallHierarchyCallers | Self::CallHierarchyCallees | Self::TypeHierarchy | Self::TypeHierarchySupertypes | Self::TypeHierarchySubtypes | Self::Documentation => return G::CodeNavigation,
            Self::NextProblem | Self::PreviousProblem | Self::ShowProblems | Self::CheckProjectSyntax | Self::CancelProjectSyntax => return G::Problems,
            Self::MatchCase | Self::WholeWord | Self::RegularExpression => return G::FindOptions,
            Self::ExtendSelection | Self::ShrinkSelection | Self::AddCursorBelow | Self::AddCursorAbove | Self::SelectNextOccurrence | Self::CollapseSelections => return G::Selections,
            Self::IndentSelection | Self::OutdentSelection | Self::ToggleComment | Self::MoveLinesUp | Self::MoveLinesDown | Self::DuplicateLinesUp | Self::DuplicateLinesDown => return G::Lines,
            Self::EditingPreferences | Self::IndentTabs | Self::IndentTwoSpaces | Self::IndentFourSpaces | Self::IndentInherit | Self::FormatOnSave | Self::VimMode => return G::EditingSettings,
            Self::WorkspaceRoot | Self::ServerRuntime | Self::ExcludeRubyTooling | Self::ResetProjectServerSettings | Self::PersistentIndex | Self::RebuildIndex => return G::ServerSetup,
            Self::ConfigureSqlMetadata | Self::ConnectSqlMetadata | Self::RefreshSqlMetadata | Self::DisconnectSqlMetadata => return G::Metadata,
            Self::SqlDialect | Self::SqlDialectPostgreSql | Self::SqlDialectDuckDb | Self::SqlDialectFollowProject | Self::ProjectSqlDialectPostgreSql | Self::ProjectSqlDialectDuckDb | Self::ProjectSqlDialectUnset => return G::Dialect,
            Self::Keymap | Self::KeymapIntelliJ | Self::KeymapVsCode | Self::ResetKeymap => return G::Keybindings,
            _ => {}
        }
        match self {
            Self::Open | Self::OpenFolder | Self::OpenRecent | Self::NewWindow | Self::Reload | Self::Save | Self::SaveWithoutFormatting | Self::SaveAs | Self::NewDocument | Self::RecoverDrafts | Self::RecheckSave | Self::ConfirmRecheckedSave
            | Self::NewScratch | Self::ScratchFiles | Self::CloseAllTabs | Self::CloseOtherTabs | Self::CloseTabsLeft | Self::CloseTabsRight | Self::Close | Self::PinTab | Self::CopyPath | Self::CopyPathLine | Self::OpenInEditor | Self::OpenWithDefaultApp
            | Self::ReadOnlyMode | Self::CompareWith | Self::CompareSelected | Self::CompareSaved | Self::CloseSavedComparison
            | Self::ResolveExternalChange => G::File,
            Self::ShowTerminal | Self::NewTerminal | Self::FocusTerminal | Self::FocusEditor | Self::SendSelectionToTerminal | Self::PasteTerminalPath | Self::CloseTerminal | Self::StopTerminal | Self::RenameTerminal | Self::RestartTerminal | Self::NextTerminal | Self::PreviousTerminal | Self::CopyTerminalSelection | Self::FindTerminal | Self::ToggleActivityLog | Self::CopyActivityLog | Self::ClearActivityLog | Self::StatusDetails | Self::RelativeLineNumbers | Self::SoftWrap | Self::StickyScroll | Self::Breadcrumbs | Self::Outline
            | Self::Collapse | Self::Expand | Self::CollapseAll | Self::ExpandAll | Self::CollapseCommentsImports
            | Self::DocumentationOnHover | Self::AutoCompletion | Self::FileIcons | Self::InlayTypes | Self::InlayParameters | Self::SemanticHighlighting
            | Self::CsvTable | Self::CsvHeader | Self::CsvFilter | Self::CsvSort | Self::CsvClearFilter | Self::CsvMoveLeft | Self::CsvMoveRight | Self::CsvDropColumn | Self::CsvEditCell
            | Self::ImageSource | Self::MarkdownSource | Self::PreviewBeside | Self::ToggleExplorer | Self::LocateFile | Self::NewExplorerFile | Self::NewExplorerFolder
            | Self::CopyExplorerName | Self::CopyExplorerFullPath | Self::CopyExplorerRelativePath | Self::CopyExplorerEntry | Self::PasteExplorerEntry | Self::CancelExplorerCopy | Self::ExplorerHistory
            | Self::RenameExplorerEntry | Self::MoveExplorerEntry | Self::DeleteExplorerEntry | Self::DeleteExplorerPermanently | Self::RestoreDeleted
            | Self::FoldersFirst | Self::AllByName | Self::AnnotateBlame | Self::CloseAnnotations | Self::NarrowAnnotations | Self::WidenAnnotations | Self::ShowCommit | Self::ShowCommitDiff | Self::CopyRevision | Self::SplitRight | Self::SplitDown | Self::Unsplit
            | Self::FocusOtherPane | Self::MoveTabToOtherPane | Self::WidenExplorer | Self::NarrowExplorer | Self::TallerResults | Self::ShorterResults | Self::EnlargeFirstPane | Self::EnlargeSecondPane => G::View,
            Self::GitBranches | Self::NewBranch | Self::CheckoutRevision | Self::ShowLocalChanges | Self::CompareWithRevision | Self::CompareWithBase
            | Self::ShowBranchChanges | Self::CompareWithWorktree | Self::ResolveConflicts => G::Git,
            Self::GoToLine | Self::FocusBreadcrumbs | Self::Back | Self::Forward | Self::ToggleBookmark
            | Self::SetBookmarkMnemonic | Self::ShowBookmarks | Self::NextBookmark | Self::PreviousBookmark
            | Self::RecentFiles | Self::RecentLocations | Self::NextTab | Self::PreviousTab
            | Self::Definition | Self::PeekDefinition | Self::TypeDefinition | Self::Implementation
            | Self::CallHierarchy | Self::CallHierarchyCallers | Self::CallHierarchyCallees
            | Self::TypeHierarchy | Self::TypeHierarchySupertypes | Self::TypeHierarchySubtypes
            | Self::Documentation | Self::NextProblem | Self::PreviousProblem | Self::ShowProblems | Self::CheckProjectSyntax | Self::CancelProjectSyntax | Self::ShowInSource | Self::FollowLink => G::Navigate,
            Self::SearchEverywhere | Self::FindSymbol | Self::FindAction | Self::FindFile | Self::SearchProject | Self::FindInFile | Self::NextMatch | Self::PreviousMatch
            | Self::CloseFind | Self::MatchCase | Self::WholeWord | Self::RegularExpression | Self::ReplaceMatch | Self::ReplaceAll | Self::ListMatches
            | Self::FindDeclarations | Self::RerunSearch | Self::CloseResults | Self::ToggleResults | Self::StopSearch | Self::FoldAllResults | Self::NextResult | Self::PreviousResult | Self::PreviousResults | Self::NextResults | Self::Usages | Self::Symbols
            | Self::FileSymbols => G::Find,
            Self::SelectAll | Self::Copy | Self::Cut | Self::Paste | Self::Delete | Self::Undo | Self::Redo | Self::ExtendSelection | Self::ShrinkSelection
            | Self::AddCursorBelow | Self::AddCursorAbove | Self::SelectNextOccurrence | Self::CollapseSelections
            | Self::IndentSelection | Self::OutdentSelection | Self::ToggleComment | Self::MoveLinesUp | Self::MoveLinesDown
            | Self::DuplicateLinesUp | Self::DuplicateLinesDown
            | Self::VimMode | Self::EditingPreferences | Self::IndentTabs | Self::IndentTwoSpaces | Self::IndentFourSpaces | Self::IndentInherit | Self::FormatOnSave | Self::Completion | Self::ParameterInfo | Self::QuickFix | Self::FormatDocument | Self::OrganizeImports => G::Selection,
            Self::InstallServer | Self::StartServerInstall | Self::CancelServerInstall | Self::WorkspaceRoot | Self::LanguageServer | Self::ServerDetails | Self::CopyServerDetails | Self::ServerRuntime
            | Self::RestartServer | Self::ExcludeRubyTooling | Self::ResetProjectServerSettings | Self::PersistentIndex | Self::RebuildIndex
            | Self::DisableServer | Self::EnableServer => G::LanguageServers,
            Self::InspectChange | Self::ShowChangeDiff | Self::RevertChange | Self::CloseChange => G::Git,
            Self::ConfigureSqlMetadata | Self::ConnectSqlMetadata | Self::RefreshSqlMetadata | Self::DisconnectSqlMetadata | Self::SqlDialect | Self::SqlDialectPostgreSql | Self::SqlDialectDuckDb | Self::SqlDialectFollowProject
            | Self::ProjectSqlDialectPostgreSql | Self::ProjectSqlDialectDuckDb | Self::ProjectSqlDialectUnset => G::Sql,
            Self::ChooseTheme | Self::About | Self::CloseWindow | Self::QuitAll
            | Self::Keymap | Self::KeymapIntelliJ | Self::KeymapVsCode | Self::ResetKeymap => G::Application,
        }
    }
}


impl CommandId {
    /// Lenz commands whose chords still run while the terminal has the keys; every other key goes to its program.
    pub fn passes_terminal_focus(self) -> bool {
        matches!(self, Self::FocusEditor | Self::ShowTerminal | Self::NewTerminal | Self::FocusTerminal
            | Self::NextTerminal | Self::PreviousTerminal | Self::FindAction | Self::CloseTerminal | Self::StopTerminal
            | Self::RenameTerminal | Self::RestartTerminal | Self::CopyTerminalSelection | Self::FindTerminal
            | Self::PasteTerminalPath | Self::SendSelectionToTerminal)
    }

    /// The fixed editing keys adapters map as text input themselves. Semantic commands take their keys from the
    /// session's keymap instead, so a preset or an override changes what menus show and what keys run.
    pub fn input_hint(self) -> Option<&'static str> {
        match self {
            Self::SelectAll => Some("Ctrl+A"),
            Self::Copy => Some("Ctrl+C"),
            Self::Cut => Some("Ctrl+X"),
            Self::Paste => Some("Ctrl+V"),
            Self::Delete => Some("Delete"),
            Self::Undo => Some("Ctrl+Z"),
            Self::Redo => Some("Ctrl+Shift+Z"),
            // Escape is an editing key, not a chord: the adapters give it to this before their own meanings.
            Self::CollapseSelections => Some("Esc"),
            Self::IndentSelection => Some("Tab"),
            Self::OutdentSelection => Some("Shift+Tab"),
            _ => None,
        }
    }
}


impl CommandId {
    /// Explorer commands must capture the adapter's highlighted row before availability is resolved.
    pub fn targets_explorer(self) -> bool {
        matches!(self, Self::NewDocument | Self::NewExplorerFile | Self::NewExplorerFolder | Self::RenameExplorerEntry | Self::MoveExplorerEntry
            | Self::DeleteExplorerEntry | Self::DeleteExplorerPermanently | Self::CopyExplorerName | Self::CopyExplorerFullPath
            | Self::CopyExplorerRelativePath | Self::CopyExplorerEntry | Self::PasteExplorerEntry | Self::ExplorerHistory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_in_one_nonempty_menu_group() {
        let grouped: Vec<_> = CommandGroup::ALL.into_iter().flat_map(|group| {
            let ids: Vec<_> = CommandId::ALL.iter().copied().filter(|id| id.group() == group).collect();
            assert!(!ids.is_empty() || CommandGroup::ALL.iter().any(|child| child.parent() == Some(group)), "{}", group.label());
            ids
        }).collect();
        assert_eq!(grouped.len(), CommandId::ALL.len());
        assert!(CommandId::ALL.iter().all(|id| grouped.contains(id)));
    }
}
