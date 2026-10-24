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
    FocusBreadcrumbs,
    ToggleExplorer,
    WorkspaceRoot,
    LanguageServer,
    ServerDetails,
    ServerRuntime,
    StatusDetails,
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
        let tab = self.state().shown(pane).filter(|t| self.state().view(t.id()).is_some_and(|v| v.rendering().is_some()))?;
        Some(Action {
            id: CommandId::MarkdownSource,
            label: if tab.markdown_source() { "Preview" } else { "Source" },
            disabled: matches!(tab.preview, Some(Err(_))).then_some("This document cannot be previewed (the notice says why)"),
            on: None,
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
            CommandId::SearchEverywhere => return CatalogAction::Interface(InterfaceAction::SearchEverywhere),
            CommandId::FindAction => return CatalogAction::Interface(InterfaceAction::FindAction),
            CommandId::FindSymbol => return CatalogAction::Interface(InterfaceAction::FindSymbol),
            CommandId::ServerRuntime => return CatalogAction::Interface(InterfaceAction::ServerRuntime),
            CommandId::ServerDetails => return CatalogAction::Interface(InterfaceAction::ServerDetails),
            CommandId::LanguageServer => return CatalogAction::Interface(InterfaceAction::LanguageServer),
            CommandId::Open => return CatalogAction::Interface(InterfaceAction::Open),
            CommandId::OpenInEditor => return CatalogAction::Interface(InterfaceAction::OpenInEditor),
            CommandId::OpenWithDefaultApp => return CatalogAction::Interface(InterfaceAction::OpenWithDefaultApp),
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
            Self::Open | Self::OpenFolder | Self::OpenRecent | Self::NewWindow | Self::Reload | Self::Save | Self::RecheckSave | Self::ConfirmRecheckedSave
            | Self::Close | Self::PinTab | Self::CopyPath | Self::CopyPathLine | Self::OpenInEditor | Self::OpenWithDefaultApp
            | Self::CompareWith | Self::CompareSelected => G::File,
            Self::StatusDetails | Self::SoftWrap | Self::StickyScroll | Self::Breadcrumbs | Self::Outline
            | Self::Collapse | Self::Expand | Self::CollapseAll | Self::ExpandAll | Self::CollapseCommentsImports
            | Self::DocumentationOnHover | Self::FileIcons | Self::InlayTypes | Self::InlayParameters | Self::SemanticHighlighting
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
            Self::SelectAll | Self::Copy | Self::Undo | Self::Redo | Self::ExtendSelection | Self::ShrinkSelection => G::Selection,
            Self::WorkspaceRoot | Self::LanguageServer | Self::ServerDetails | Self::CopyServerDetails | Self::ServerRuntime
            | Self::RestartServer | Self::ExcludeRubyTooling | Self::ResetProjectServerSettings | Self::PersistentIndex | Self::RebuildIndex
            | Self::DisableServer | Self::EnableServer => G::LanguageServers,
            Self::ChooseTheme | Self::About => G::Application,
        }
    }
}


impl CommandId {
    /// Key labels shared by name-search results and pointer menus.
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::SearchEverywhere => Some("Double Shift / Ctrl+K"),
            Self::FindFile => Some("Ctrl+P / Ctrl+Shift+N"),
            Self::FindSymbol => Some("Ctrl+Alt+Shift+N"),
            Self::FindAction => Some("Ctrl+Shift+A"),
            Self::Open => Some("Ctrl+O"),
            Self::SplitRight => Some("Ctrl+\\"),
            Self::SplitDown => Some("Ctrl+Shift+\\"),
            Self::Outline => Some("Ctrl+Shift+O"),
            Self::FindInFile => Some("Ctrl+F"),
            Self::CompareSelected => Some("Ctrl+D"),
            Self::SearchProject => Some("Ctrl+Shift+F"),
            Self::GoToLine => Some("Ctrl+G"),
            Self::Reload => Some("F5"),
            Self::SoftWrap => Some("Alt+Z"),
            Self::LanguageServer => Some("F2"),
            Self::StatusDetails => Some("Shift+F2"),
            Self::Definition => Some("F12"),
            Self::PeekDefinition => Some("Alt+F12"),
            Self::TypeDefinition => Some("Ctrl+Shift+B"),
            Self::Implementation => Some("Ctrl+Alt+B"),
            Self::AnnotateBlame | Self::CloseAnnotations => Some("Alt+B"),
            Self::Usages => Some("Shift+F12"),
            Self::CallHierarchy => Some("Ctrl+Alt+H"),
            Self::TypeHierarchy => Some("Ctrl+H"),
            Self::Documentation => Some("Ctrl+Q"),
            Self::Copy => Some("Ctrl+C"),
            Self::Save => Some("Ctrl+S"),
            Self::Undo => Some("Ctrl+Z"),
            Self::Redo => Some("Ctrl+Shift+Z"),
            Self::Close => Some("Ctrl+F4"),
            Self::ExtendSelection => Some("Ctrl+W"),
            Self::ShrinkSelection => Some("Ctrl+Shift+W"),
            Self::FocusBreadcrumbs => Some("Ctrl+Shift+."),
            Self::ToggleBookmark => Some("F11"),
            Self::SetBookmarkMnemonic => Some("Ctrl+F11"),
            Self::ShowBookmarks => Some("Shift+F11"),
            Self::RecentFiles => Some("Ctrl+E"),
            Self::RecentLocations => Some("Ctrl+Alt+E"),
            Self::Collapse => Some("Ctrl+-"),
            Self::Expand => Some("Ctrl+="),
            Self::CollapseAll => Some("Ctrl+Shift+-"),
            Self::ExpandAll => Some("Ctrl+Shift+="),
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
