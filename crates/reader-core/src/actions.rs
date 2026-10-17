//! Resolve catalog entries once for every frontend; adapters own only interface presentation.
use lenz_content::find::FindOption;

use crate::{
    semantics::hierarchy::Direction,
    session::{Command, CommandId, Session, Split},
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
    NewWindow,
    FocusBreadcrumbs,
    ToggleExplorer,
    WorkspaceRoot,
}

impl Session {
    /// Resolve against the current state, including availability and toggle values. Dispatch a returned command
    /// immediately: do not retain it across state changes.
    pub fn action(&self, id: CommandId) -> CatalogAction {
        if let Some(why) = self.disabled(id) {
            return CatalogAction::Disabled(why);
        }
        let state = self.state();
        let command = match id {
            CommandId::Open => return CatalogAction::Interface(InterfaceAction::Open),
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
            CommandId::InlayTypes => Command::SetInlayTypes(!state.inlay_types()),
            CommandId::InlayParameters => Command::SetInlayParameters(!state.inlay_parameters()),
            CommandId::SemanticHighlighting => Command::SetSemanticHighlighting(!state.semantic_highlighting()),
            CommandId::CopyPath => Command::CopyPath { line: false },
            CommandId::CopyPathLine => Command::CopyPath { line: true },
            CommandId::FileSymbols => Command::FileSymbols,
            CommandId::RestartServer => Command::RestartServer,
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
        state.selected_text().filter(|s| !s.contains('\n')).map(str::to_owned)
            .or_else(|| state.active().and_then(|t| t.find()).map(|f| f.query().to_owned()))
            .unwrap_or_default()
    }
}
