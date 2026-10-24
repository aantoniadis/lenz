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

/// Interface requests whose inputs and layout belong to the frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceAction {
    SearchEverywhere,
    FindAction,
    FindSymbol,
    Open,
    /// Ask for a new path, then dispatch [`crate::session::Command::SaveAs`] for the active tab.
    SaveAs,
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
    EditingPreferences,
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
            CommandId::StatusDetails => return CatalogAction::Interface(InterfaceAction::StatusDetails),
            CommandId::Keymap => return CatalogAction::Interface(InterfaceAction::Keymap),
            CommandId::KeymapIntelliJ => Command::SetKeymapPreset(crate::keymap::KeymapPreset::IntelliJ),
            CommandId::KeymapVsCode => Command::SetKeymapPreset(crate::keymap::KeymapPreset::VsCode),
            CommandId::ResetKeymap => Command::ResetKeymap,
            CommandId::EditingPreferences => return CatalogAction::Interface(InterfaceAction::EditingPreferences),
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
            CommandId::ListMatches => Command::ListMatches,
            CommandId::SoftWrap => Command::SetSoftWrap(!state.soft_wrap()),
            CommandId::StickyScroll => Command::SetStickyScroll(!state.sticky_scroll()),
            CommandId::Breadcrumbs => Command::SetBreadcrumbs(!state.breadcrumbs()),
            CommandId::Outline => Command::Outline,
            CommandId::RerunSearch => Command::RerunSearch,
            CommandId::CloseResults => Command::CloseResults,
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
            CommandId::Delete => Command::Delete { backward: false },
            CommandId::ReadOnlyMode => Command::SetReadOnly(!state.active().and_then(|t| state.view(t.id())).is_some_and(|view| view.read_only())),
            CommandId::SaveAs => return CatalogAction::Interface(InterfaceAction::SaveAs),
            // An untitled document has no file yet: Save chooses one.
            CommandId::Save if state.active().is_some_and(crate::tabs::Tab::untitled) => return CatalogAction::Interface(InterfaceAction::SaveAs),
            CommandId::NewDocument => Command::NewDocument,
            CommandId::RecoverDrafts => Command::ShowRecoveredDrafts,
            CommandId::Save | CommandId::RecheckSave | CommandId::ConfirmRecheckedSave => match state.active() {
                Some(tab) => match id { CommandId::Save => Command::Save { tab: tab.id() }, CommandId::RecheckSave => Command::RecheckSave { tab: tab.id() }, _ => Command::ConfirmRecheckedSave { tab: tab.id() } },
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
            CommandId::AutoCompletion => Command::SetAutoCompletion(!state.auto_completion()),
            CommandId::FileIcons => Command::SetFileIcons(!state.file_icons()),
            CommandId::AnnotateBlame => Command::SetAnnotate(state.active().is_none_or(|t| t.blame.is_none())),
            CommandId::CloseAnnotations => Command::SetAnnotate(false),
            CommandId::CopyRevision => Command::CopyRevision,
            CommandId::ShowCommit => match state.active().and_then(|t| Some((t.id(), state.view(t.id())?.snapshot().line_of(t.selection()?.head)))) {
                Some((tab, line)) => Command::ShowCommitAt { tab, line },
                None => return CatalogAction::Disabled("No line is selected"),
            },
            CommandId::InlayTypes => Command::SetInlayTypes(!state.inlay_types()),
            CommandId::InlayParameters => Command::SetInlayParameters(!state.inlay_parameters()),
            CommandId::SemanticHighlighting => Command::SetSemanticHighlighting(!state.semantic_highlighting()),
            CommandId::CopyPath => Command::CopyPath { line: false },
            CommandId::CopyPathLine => Command::CopyPath { line: true },
            CommandId::FileSymbols => Command::FileSymbols,
            CommandId::CopyServerDetails => Command::CopyText(self.language_server_details()),
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
            CommandId::LocateFile => Command::Locate,
            CommandId::FoldersFirst => Command::SetFoldersFirst(true),
            CommandId::AllByName => Command::SetFoldersFirst(false),
            CommandId::SplitRight => Command::Split(Split::Columns),
            CommandId::SplitDown => Command::Split(Split::Rows),
            CommandId::Unsplit => Command::Unsplit,
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
    File,
    View,
    Navigate,
    Find,
    Selection,
    Git,
    LanguageServers,
    Application,
}

impl CommandGroup {
    pub const ALL: [Self; 8] = [Self::File, Self::View, Self::Navigate, Self::Find, Self::Selection, Self::Git, Self::LanguageServers, Self::Application];

    pub fn label(self) -> &'static str {
        match self {
            Self::File => "File",
            Self::View => "View",
            Self::Navigate => "Navigate",
            Self::Find => "Find",
            Self::Selection => "Selection",
            Self::Git => "Git",
            Self::LanguageServers => "Language Servers",
            Self::Application => "Application",
        }
    }
}

impl CommandId {
    pub fn group(self) -> CommandGroup {
        use CommandGroup as G;
        match self {
            Self::Open | Self::OpenFolder | Self::OpenRecent | Self::NewWindow | Self::Reload | Self::Save | Self::SaveAs | Self::NewDocument | Self::RecoverDrafts | Self::RecheckSave | Self::ConfirmRecheckedSave
            | Self::Close | Self::PinTab | Self::CopyPath | Self::CopyPathLine | Self::OpenInEditor | Self::OpenWithDefaultApp
            | Self::ReadOnlyMode | Self::CompareWith | Self::CompareSelected | Self::CompareSaved | Self::CloseSavedComparison => G::File,
            Self::StatusDetails | Self::SoftWrap | Self::StickyScroll | Self::Breadcrumbs | Self::Outline
            | Self::Collapse | Self::Expand | Self::CollapseAll | Self::ExpandAll | Self::CollapseCommentsImports
            | Self::DocumentationOnHover | Self::AutoCompletion | Self::FileIcons | Self::InlayTypes | Self::InlayParameters | Self::SemanticHighlighting
            | Self::ImageSource | Self::MarkdownSource | Self::ToggleExplorer | Self::LocateFile
            | Self::FoldersFirst | Self::AllByName | Self::AnnotateBlame | Self::CloseAnnotations | Self::ShowCommit | Self::ShowCommitDiff | Self::CopyRevision | Self::SplitRight | Self::SplitDown | Self::Unsplit
            | Self::FocusOtherPane | Self::MoveTabToOtherPane => G::View,
            Self::GitBranches | Self::NewBranch | Self::CheckoutRevision | Self::ShowLocalChanges | Self::CompareWithRevision | Self::CompareWithBase
            | Self::ShowBranchChanges | Self::CompareWithWorktree | Self::ResolveConflicts => G::Git,
            Self::GoToLine | Self::FocusBreadcrumbs | Self::Back | Self::Forward | Self::ToggleBookmark
            | Self::SetBookmarkMnemonic | Self::ShowBookmarks | Self::NextBookmark | Self::PreviousBookmark
            | Self::RecentFiles | Self::RecentLocations | Self::NextTab | Self::PreviousTab
            | Self::Definition | Self::PeekDefinition | Self::TypeDefinition | Self::Implementation
            | Self::CallHierarchy | Self::CallHierarchyCallers | Self::CallHierarchyCallees
            | Self::TypeHierarchy | Self::TypeHierarchySupertypes | Self::TypeHierarchySubtypes
            | Self::Documentation => G::Navigate,
            Self::SearchEverywhere | Self::FindSymbol | Self::FindAction | Self::FindFile | Self::SearchProject | Self::FindInFile | Self::NextMatch | Self::PreviousMatch
            | Self::CloseFind | Self::MatchCase | Self::WholeWord | Self::RegularExpression | Self::ListMatches
            | Self::FindDeclarations | Self::RerunSearch | Self::CloseResults | Self::Usages | Self::Symbols
            | Self::FileSymbols => G::Find,
            Self::SelectAll | Self::Copy | Self::Cut | Self::Paste | Self::Delete | Self::Undo | Self::Redo | Self::ExtendSelection | Self::ShrinkSelection
            | Self::AddCursorBelow | Self::AddCursorAbove | Self::SelectNextOccurrence | Self::CollapseSelections
            | Self::EditingPreferences | Self::IndentTabs | Self::IndentTwoSpaces | Self::IndentFourSpaces | Self::IndentInherit | Self::FormatOnSave | Self::Completion => G::Selection,
            Self::WorkspaceRoot | Self::LanguageServer | Self::ServerDetails | Self::CopyServerDetails | Self::ServerRuntime
            | Self::RestartServer | Self::ExcludeRubyTooling | Self::ResetProjectServerSettings | Self::PersistentIndex | Self::RebuildIndex
            | Self::DisableServer | Self::EnableServer => G::LanguageServers,
            Self::ChooseTheme | Self::About | Self::CloseWindow | Self::QuitAll
            | Self::Keymap | Self::KeymapIntelliJ | Self::KeymapVsCode | Self::ResetKeymap => G::Application,
        }
    }
}


impl CommandId {
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
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_in_one_nonempty_menu_group() {
        let grouped: Vec<_> = CommandGroup::ALL.into_iter().flat_map(|group| {
            let ids: Vec<_> = CommandId::ALL.iter().copied().filter(|id| id.group() == group).collect();
            assert!(!ids.is_empty(), "{}", group.label());
            ids
        }).collect();
        assert_eq!(grouped.len(), CommandId::ALL.len());
        assert!(CommandId::ALL.iter().all(|id| grouped.contains(id)));
    }
}
