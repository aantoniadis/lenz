//! Terminal input translates to the shared reader session; only focus and cell geometry live here.
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind,
};
use lenz_content::{
    goto::parse_line_column,
    theme::{ThemeCommand, ThemeSelection},
};
use reader_core::{
    actions::{CatalogAction, CommandGroup, InterfaceAction},
    breadcrumbs::SiblingTarget,
    chrome::MenuTarget,
    navigation::{Chosen, DoubleShift, Scope},
    search::{Group, Row},
    session::{
        Command, CommandId, Completion, Config, MenuEntry, Motion, Place, PlaceList, Rendering,
        SearchIn, Selection, Session,
    },
    tabs::TabId,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Tree,
    Text,
    Results,
    Breadcrumbs,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Prompt {
    Commands,
    /// The application menu under the toolbar's button: its groups (`None`) or one group's commands.
    Menu(Option<CommandGroup>),
    LanguageServer,
    Context {
        x: u16,
        y: u16,
        entries: Vec<MenuEntry>,
    },
    Open,
    OpenFolder,
    /// Compare With: the other file or folder for this one.
    Compare(PathBuf),
    WorkspaceRoot,
    ServerRuntime,
    RecentRoots,
    Navigation,
    Search,
    Find,
    Declarations,
    Symbols,
    GoTo,
    Themes,
    Places(PlaceList),
    Mnemonic,
    Siblings {
        pane: u8,
        crumb: usize,
    },
    Help,
    /// Read-only text (About adds its half-block logo when there is room).
    Details { title: &'static str, body: String, logo: bool },
}
pub(crate) struct Modal {
    pub kind: Prompt,
    pub input: String,
    pub secondary: String,
    pub secondary_focus: bool,
    pub at: usize,
    pub top: usize,
    pub reveal_selection: bool,
    pub error: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Choice {
    pub label: String,
    pub detail: Option<String>,
    pub disabled: Option<&'static str>,
    pub target: ChoiceTarget,
}
#[derive(Clone)]
pub(crate) enum ChoiceTarget {
    Action(CommandId),
    Information,
    ServerRuntime(Option<PathBuf>),
    Path(PathBuf),
    Theme(String),
    Place(Place),
    Sibling {
        pane: u8,
        crumb: usize,
        index: usize,
    },
}
#[derive(Clone, Copy)]
struct Hover {
    tab: TabId,
    version: u64,
    offset: usize,
    top: usize,
    due: Instant,
}
pub(crate) struct App {
    /// `LENZ_ICONS=nerd|plain` for this run, over the File Icons setting until that is toggled here.
    pub icons_override: Option<bool>,
    pub session: Session,
    pub modal: Option<Modal>,
    pub focus: Focus,
    pub tree_at: usize,
    pub tree_top: usize,
    pub results_at: usize,
    pub results_top: usize,
    pub tree_shown: bool,
    pending_explorer: Option<reader_core::workspace::Explorer>,
    pub notice: Option<String>,
    pub independent_view: Option<(usize, usize)>,
    pub quit: bool,
    pub new_view: bool,
    pub copy: Option<String>,
    /// Lenz Diff's terminal app, for the runtime to run in this terminal until it quits.
    pub foreground: Option<lenz_content::open::Launch>,
    /// What Open in Editor started; tests record here instead of starting anything.
    #[cfg(test)]
    pub launched: Vec<lenz_content::open::Launch>,
    pub layout: crate::render::Geometry,
    pub crumb_at: usize,
    drag: Option<(u8, TabId, u64, usize)>,
    /// The last text click (when, where) and how many came in a row there: two take the word, three its group.
    clicks: Option<(Instant, (u8, TabId, usize), u8)>,
    hover: Option<Hover>,
    double_shift: DoubleShift,
    pub(crate) preview_selection: Option<(TabId, u64, Selection)>,
    pub(crate) preview_scroll: HashMap<TabId, (u64, usize)>,
}
impl App {
    pub fn new(config: Config, path: Option<PathBuf>) -> Self {
        let mut app = Self {
            // Unit tests read plain labels unless they choose glyphs (the default has its own test).
            icons_override: if cfg!(test) { Some(false) } else { std::env::var("LENZ_ICONS").ok().map(|v| v == "nerd") },
            session: Session::new(config),
            modal: None,
            focus: Focus::Text,
            tree_at: 0,
            tree_top: 0,
            results_at: 0,
            results_top: 0,
            tree_shown: true,
            pending_explorer: None,
            notice: None,
            independent_view: None,
            quit: false,
            new_view: false,
            copy: None,
            foreground: None,
            #[cfg(test)]
            launched: Vec::new(),
            layout: Default::default(),
            crumb_at: 0,
            drag: None,
            clicks: None,
            hover: None,
            double_shift: DoubleShift::default(),
            preview_selection: None,
            preview_scroll: HashMap::new(),
        };
        if let Some(path) = path {
            if path.is_dir() {
                app.open_root(path);
            } else {
                app.dispatch(Command::Open(path));
            }
        }
        app
    }
    fn remember_explorer(&mut self) {
        if self.pending_explorer.is_some() {
            return;
        }
        if !self.session.state().has_tree() { return; }
        let explorer = reader_core::workspace::Explorer {
            visible: self.tree_shown,
            selected: self.session.state().tree_rows().nth(self.tree_at).map(|r| r.path.to_string()),
            top: self.session.state().tree_rows().nth(self.tree_top).map(|r| r.path.to_string()),
        };
        if self.session.state().explorer() != &explorer {
            self.session.dispatch(Command::SetExplorer(explorer));
        }
    }

    fn sync_workspace(&mut self, previous: u64) {
        if self.session.state().workspace_revision() != previous {
            let explorer = self.session.state().explorer().clone();
            self.tree_shown = explorer.visible;
            self.tree_at = 0;
            self.tree_top = 0;
            self.results_at = 0;
            self.results_top = 0;
            self.pending_explorer = Some(explorer);
        }
        let (Some(explorer), Some(project)) =
            (&self.pending_explorer, self.session.state().project())
        else {
            return;
        };
        let at = |path: &Option<String>| {
            path.as_ref()
                .and_then(|path| project.rows().iter().position(|r| r.path.as_ref() == path))
        };
        let selected = at(&explorer.selected);
        let top = at(&explorer.top);
        let done = matches!(
            project.coverage(),
            reader_core::project::Coverage::Complete(_)
                | reader_core::project::Coverage::Unavailable(_)
        );
        if (explorer.selected.is_none() || selected.is_some() || done)
            && (explorer.top.is_none() || top.is_some() || done)
        {
            self.tree_at = selected.unwrap_or(0);
            self.tree_top = top.unwrap_or(0);
            self.pending_explorer = None;
        }
    }

    pub fn dispatch(&mut self, command: Command) {
        if !matches!(command, Command::ColourResults(_)) {
            self.hover = None;
        }
        match &command {
            Command::ScrollRows(delta) if self.scroll_preview(Some(*delta), 0) => return,
            Command::Scroll(row) if self.scroll_preview(None, *row) => return,
            _ => {}
        }
        let preview_positions = self.preview_positions();
        let reset_preview_position = matches!(
            command,
            Command::Back
                | Command::Forward
                | Command::GoToLine(_)
                | Command::ToggleMarkdownSource
                | Command::FindStep { .. }
                | Command::StepBookmark { .. }
                | Command::GoToPlace(_)
                | Command::OpenAt { .. }
                | Command::JumpMnemonic(_)
        );
        if let Command::OpenResult(index) = &command
            && let Some(path) = self
                .session
                .state()
                .results()
                .and_then(|r| r.target(*index))
        {
            for tab in self
                .session
                .state()
                .tabs()
                .iter()
                .filter(|tab| tab.source() == path)
            {
                self.preview_scroll.remove(&tab.id());
            }
        }
        let navigates = matches!(
            command,
            Command::Open(_)
                | Command::OpenRetained(_)
                | Command::ActivateTree { .. }
                | Command::OpenAt { .. }
                | Command::ActivateTab(_)
                | Command::Close
                | Command::CloseTab(_)
                | Command::OpenRoot(_)
        );
        let results = matches!(
            command,
            Command::Outline
                | Command::ListMatches
                | Command::FindDeclarations(_)
                | Command::Symbols(_)
                | Command::FileSymbols
                | Command::Search { .. }
                | Command::RerunSearch
                | Command::Usages
                | Command::CallHierarchy(_)
                | Command::TypeHierarchy(_)
        );
        let close_results = matches!(command, Command::CloseResults);
        if matches!(&command, Command::OpenRoot(_)) {
            self.remember_explorer();
        }
        let workspace = self.session.state().workspace_revision();
        let tree_revision = self.session.state().tree_revision();
        let effect = self.session.dispatch(command);
        self.sync_workspace(workspace);
        if self.session.state().tree_revision() != tree_revision {
            self.pending_explorer = None;
            self.tree_shown = true;
            self.tree_at = self.session.state().active_in_tree().and_then(|path| self.session.state().tree_rows().position(|r| r.path == path)).unwrap_or(0);
        }
        if let Some(copy) = effect.copy {
            self.copy = Some(copy);
            self.notice = Some("Copied to terminal clipboard".into());
        }
        if navigates {
            self.drag = None;
            self.crumb_at = 0;
        }
        if reset_preview_position && let Some(tab) = self.session.state().active() {
            self.preview_scroll.remove(&tab.id());
        }
        self.invalidate_preview_positions(preview_positions);
        self.sync_preview_selection();
        if results {
            self.results_at = 0;
            self.results_top = 0;
            self.focus = Focus::Results;
        }
        if close_results && self.focus == Focus::Results {
            self.focus = Focus::Text;
        }
        self.clamp();
    }
    pub fn next_deadline(&self) -> Option<Instant> {
        self.hover.map(|h| h.due)
    }
    fn hover_current(&self, hover: Hover) -> bool {
        self.modal.is_none()
            && self.session.state().documentation_on_hover()
            && (0..=1)
                .filter_map(|pane| self.session.state().shown(pane))
                .any(|tab| {
                    tab.id() == hover.tab
                        && tab.version() == hover.version
                        && tab.top_line() == hover.top
                })
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        let Some(hover) = self.hover else {
            return false;
        };
        if !self.hover_current(hover) {
            self.hover = None;
            return false;
        }
        if now < hover.due {
            return false;
        }
        self.hover = None;
        self.dispatch(Command::DocumentationAt {
            tab: hover.tab,
            offset: hover.offset,
        });
        true
    }
    pub fn apply(&mut self, completion: Completion) -> bool {
        let theme = self.highlighted_theme();
        let tree_path = self.session.state().tree_rows().nth(self.tree_at).map(|r| r.path.clone());
        let tree_revision = self.session.state().tree_revision();
        let restoring = self.pending_explorer.is_some();
        let results_id = self.session.state().results().map(|r| r.id());
        let preview_positions = self.preview_positions();
        let workspace = self.session.state().workspace_revision();
        let effect = self.session.dispatch(completion);
        if let Some(id) = theme {
            self.select_theme_row(&id);
        }
        self.sync_workspace(workspace);
        if self.session.state().tree_revision() != tree_revision {
            self.pending_explorer = None;
            self.tree_shown = true;
            self.tree_at = self.session.state().active_in_tree().and_then(|path| self.session.state().tree_rows().position(|r| r.path == path)).unwrap_or(0);
        } else if !restoring && let Some(path) = tree_path
            && self.session.state().tree_rows().nth(self.tree_at).is_none_or(|r| r.path != path) {
            self.tree_at = self.session.state().tree_rows().position(|r| r.path == path).unwrap_or(0);
        }
        if self.session.state().results().map(|r| r.id()) != results_id {
            self.results_at = 0;
            self.results_top = 0;
        }
        self.invalidate_preview_positions(preview_positions);
        if self.hover.is_some_and(|h| !self.hover_current(h)) {
            self.hover = None;
        }
        if let Some(copy) = effect.copy {
            self.copy = Some(copy);
        }
        self.sync_preview_selection();
        self.clamp();
        self.show_documentation();
        effect.repaint
    }
    pub(crate) fn show_documentation(&mut self) {
        if self.modal.is_none()
            && let Some(doc) = self.session.state().documentation()
        {
            self.prompt(Prompt::Details { title: "Documentation", body: doc.preview.doc.text.clone(), logo: false });
        }
    }
    fn open_root(&mut self, path: PathBuf) {
        self.dispatch(Command::OpenRoot(path));
    }
    /// Whether file names show Nerd Font file-type glyphs (the File Icons setting, on by default).
    pub fn nerd_icons(&self) -> bool {
        self.icons_override.unwrap_or_else(|| self.session.state().file_icons())
    }
    pub fn run(&mut self, id: CommandId) {
        // Toggling flips what is shown, then the setting alone decides.
        if id == CommandId::FileIcons {
            let on = !self.nerd_icons();
            self.icons_override = None;
            return self.dispatch(Command::SetFileIcons(on));
        }
        match self.session.action(id) {
            CatalogAction::Disabled(why) => self.notice = Some(why.into()),
            CatalogAction::Command(command) => {
                if self.preview_command(&command) {
                    return;
                }
                self.dispatch(command);
                if id == CommandId::LocateFile {
                    self.tree_shown = true;
                    self.focus = Focus::Tree;
                    self.tree_at = self.session.state().active_in_tree().and_then(|path| self.session.state().tree_rows().position(|r| r.path == path)).unwrap_or(0);
                    self.clamp();
                }
            }
            CatalogAction::Interface(action) => self.interface(action),
        }
    }
    fn preview_positions(&self) -> Vec<(TabId, usize)> {
        self.session
            .state()
            .tabs()
            .iter()
            .filter(|tab| self.preview_scroll.contains_key(&tab.id()))
            .map(|tab| (tab.id(), tab.top_line()))
            .collect()
    }
    fn invalidate_preview_positions(&mut self, before: Vec<(TabId, usize)>) {
        for (id, top) in before {
            if self
                .session
                .state()
                .tabs()
                .iter()
                .find(|tab| tab.id() == id)
                .is_none_or(|tab| tab.top_line() != top)
            {
                self.preview_scroll.remove(&id);
            }
        }
    }
    fn scroll_preview(&mut self, delta: Option<isize>, absolute: usize) -> bool {
        let state = self.session.state();
        let Some(pane) = self
            .layout
            .panes
            .iter()
            .find(|p| p.pane == state.focused_pane() && p.preview)
        else {
            return false;
        };
        let Some(tab) = state.active() else {
            return false;
        };
        if !matches!(tab.rendering(), Some(Rendering::Preview(_))) {
            return false;
        }
        let current = self
            .preview_scroll
            .get(&tab.id())
            .filter(|(version, _)| *version == tab.version())
            .map_or(pane.preview_top, |(_, top)| *top);
        let top = match delta {
            Some(delta) => current.saturating_add_signed(delta),
            None => absolute,
        }
        .min(
            pane.preview_total
                .saturating_sub(pane.text.height.max(1) as usize),
        );
        let source = pane.preview_source_lines.get(top).copied().unwrap_or(0);
        self.preview_scroll.insert(tab.id(), (tab.version(), top));
        self.session.dispatch(Command::Scroll(source));
        true
    }
    fn sync_preview_selection(&mut self) {
        self.preview_scroll.retain(|id, (version, _)| {
            self.session
                .state()
                .tabs()
                .iter()
                .any(|tab| tab.id() == *id && (tab.version() == 0 || tab.version() == *version))
        });
        if let Some((id, version, _)) = self.preview_selection {
            let current = self.session.state().active().is_some_and(|tab| {
                tab.id() == id
                    && tab.document().is_some_and(|doc| doc.version() == version)
                    && matches!(tab.rendering(), Some(Rendering::Preview(_)))
            });
            if !current {
                self.preview_selection = None;
                self.drag = None;
            }
        }
    }
    fn preview_command(&mut self, command: &Command) -> bool {
        if !matches!(command, Command::SelectAll | Command::Copy) {
            return false;
        }
        let Some(tab) = self.session.state().active() else {
            return false;
        };
        let (Some(doc), Some(Rendering::Preview(preview))) = (tab.document(), tab.rendering())
        else {
            return false;
        };
        let (id, version) = (tab.id(), doc.version());
        if matches!(command, Command::SelectAll) {
            self.preview_selection = Some((
                id,
                version,
                Selection {
                    anchor: 0,
                    head: preview.doc.text.len(),
                },
            ));
        } else {
            let text = self
                .preview_selection
                .filter(|(tab, v, _)| *tab == id && *v == version)
                .and_then(|(_, _, s)| preview.doc.text.get(s.range()))
                .unwrap_or_default()
                .to_owned();
            self.dispatch(Command::CopyText(text));
        }
        true
    }
    /// Starts `launch` detached (a terminal editor gets its own window, as in Lenz Diff); a failure, or else
    /// `toast`, shows with the core's messages.
    fn start(&mut self, launch: lenz_content::open::Launch, toast: Option<String>) {
        #[cfg(test)]
        let started = { self.launched.push(launch.clone()); Ok::<(), std::io::Error>(()) };
        #[cfg(not(test))]
        let started = lenz_content::open::spawn(&launch);
        if let Some(message) = started.err().map(|e| format!("Cannot start {}: {e}", launch.program)).or(toast) {
            self.dispatch(Command::Notice(message));
        }
    }
    fn interface(&mut self, action: InterfaceAction) {
        let kind = match action {
            InterfaceAction::Open => Prompt::Open,
            InterfaceAction::OpenFolder => Prompt::OpenFolder,
            InterfaceAction::OpenInEditor | InterfaceAction::OpenWithDefaultApp => {
                let default_app = action == InterfaceAction::OpenWithDefaultApp;
                match self.session.editor_plan(&lenz_content::open::Host::current(), lenz_content::appearance::editor().as_deref(), default_app) {
                    Ok(plan) => self.start(plan.launch, plan.toast),
                    Err(why) => self.dispatch(Command::Notice(why.into())),
                }
                return;
            }
            InterfaceAction::CompareWith => {
                let row = (self.focus == Focus::Tree).then(|| self.session.state().tree_rows().nth(self.tree_at).map(|r| r.path.clone())).flatten();
                match self.session.compare_source(row.as_deref()) {
                    Ok(left) => Prompt::Compare(left),
                    Err(why) => return self.dispatch(Command::Notice(why.into())),
                }
            }
            InterfaceAction::OpenRecent => Prompt::RecentRoots,
            InterfaceAction::FindFile => return self.open_navigation(Scope::Files),
            InterfaceAction::SearchEverywhere => return self.open_navigation(Scope::All),
            InterfaceAction::FindSymbol => return self.open_navigation(Scope::Symbols),
            InterfaceAction::FindAction => return self.open_navigation(Scope::Actions),
            InterfaceAction::SearchProject => Prompt::Search,
            InterfaceAction::FindInFile => Prompt::Find,
            InterfaceAction::GoToLine => Prompt::GoTo,
            InterfaceAction::FindDeclarations => Prompt::Declarations,
            InterfaceAction::Symbols => Prompt::Symbols,
            InterfaceAction::SetBookmarkMnemonic => Prompt::Mnemonic,
            InterfaceAction::ShowBookmarks => Prompt::Places(PlaceList::Bookmarks),
            InterfaceAction::RecentFiles => return self.open_navigation(Scope::Recent),
            InterfaceAction::RecentLocations => Prompt::Places(PlaceList::RecentLocations),
            InterfaceAction::ChooseTheme => {
                self.dispatch(Command::Theme(ThemeCommand::OpenChooser));
                Prompt::Themes
            }
            InterfaceAction::ServerRuntime => {
                self.dispatch(Command::OpenServerRuntime);
                Prompt::ServerRuntime
            }
            InterfaceAction::WorkspaceRoot => Prompt::WorkspaceRoot,
            InterfaceAction::LanguageServer => Prompt::LanguageServer,
            InterfaceAction::ServerDetails | InterfaceAction::StatusDetails => {
                let (title, mut details) = if action == InterfaceAction::ServerDetails {
                    ("Server details", self.session.language_server_details())
                } else {
                    ("Status details", self.session.status_details())
                };
                if let Some(notice) = &self.notice {
                    details.push_str("\n\nTerminal notice\n");
                    details.push_str(notice);
                }
                Prompt::Details { title, body: details, logo: false }
            },
            InterfaceAction::About => Prompt::Details { title: reader_core::chrome::ABOUT_TITLE, body: reader_core::chrome::about(), logo: true },
            InterfaceAction::NewWindow => {
                self.new_view = true;
                return;
            }
            InterfaceAction::ToggleExplorer => {
                self.tree_shown = !self.tree_shown;
                self.remember_explorer();
                if !self.tree_shown && self.focus == Focus::Tree {
                    self.focus = Focus::Text;
                }
                return;
            }
            InterfaceAction::FocusBreadcrumbs => {
                self.focus = Focus::Breadcrumbs;
                self.crumb_at = self
                    .session
                    .state()
                    .breadcrumbs_for(self.session.state().focused_pane())
                    .map_or(0, |b| b.crumbs.len().saturating_sub(1));
                return;
            }
        };
        self.prompt(kind);
    }
    pub(crate) fn prompt(&mut self, kind: Prompt) {
        if !matches!(kind, Prompt::Details { .. }) {
            self.dispatch(Command::CloseDocumentation);
        }
        let mut input = String::new();
        let mut secondary = String::new();
        match kind {
            Prompt::Find => {
                input = self.session.find_seed();
                self.dispatch(Command::Find(input.clone()));
            }
            Prompt::Search => {
                if let Some(results) = self.session.state().results() {
                    input = results.query().into();
                    secondary = results.pattern().into();
                }
            }
            Prompt::ServerRuntime => {
                input = self
                    .session
                    .state()
                    .server_runtime()
                    .and_then(|m| m.configured.as_ref())
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
            }
            Prompt::WorkspaceRoot => {
                input = self
                    .session
                    .state()
                    .provider()
                    .and_then(|v| v.root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            }
            Prompt::Places(_) => self.dispatch(Command::CheckPlaces),
            _ => {}
        }
        self.modal = Some(Modal {
            kind,
            input,
            secondary,
            secondary_focus: false,
            at: 0,
            top: 0,
            reveal_selection: true,
            error: None,
        });
        if self
            .modal
            .as_ref()
            .is_some_and(|m| m.kind == Prompt::Themes)
        {
            self.select_theme_row(&self.session.theme_id());
        }
        self.drag = None;
        self.hover = None;
    }
    pub(crate) fn open_menu(&mut self, group: Option<CommandGroup>, at: usize) {
        self.prompt(Prompt::Menu(group));
        if let Some(modal) = &mut self.modal {
            modal.at = at;
        }
    }
    fn open_navigation(&mut self, scope: Scope) {
        self.close_modal();
        self.prompt(Prompt::Navigation);
        self.dispatch(Command::SearchEverywhere {
            scope,
            query: String::new(),
        });
    }
    fn navigation_scope(&mut self, scope: Scope) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        modal.at = 0;
        modal.top = 0;
        modal.reveal_selection = true;
        let query = modal.input.clone();
        self.dispatch(Command::SearchEverywhere { scope, query });
    }
    fn modal_visible_rows(&self) -> usize {
        let height = self.layout.modal_rows.height as usize;
        if self
            .modal
            .as_ref()
            .is_some_and(|m| m.kind == Prompt::Navigation)
            && height >= 4
        {
            height / 2
        } else {
            height.max(1)
        }
    }
    fn modal_len(&self) -> usize {
        if self
            .modal
            .as_ref()
            .is_some_and(|m| m.kind == Prompt::Navigation)
        {
            self.session
                .state()
                .navigation()
                .map_or(0, |s| s.rows.len())
        } else {
            self.choices().len()
        }
    }
    fn accept_navigation(&mut self, revision: u64, index: usize) {
        let Some(chosen) = self.session.navigation_choice(revision, index) else {
            return;
        };
        self.close_modal();
        self.focus = Focus::Text;
        match chosen {
            Chosen::Command(command) => self.dispatch(command),
            Chosen::Action(id) => self.run(id),
        }
    }
    fn close_modal(&mut self) {
        if let Some(modal) = self.modal.take() {
            match modal.kind {
                Prompt::Themes => self.dispatch(Command::PreviewTheme(None)),
                Prompt::Navigation => self.dispatch(Command::CloseSearchEverywhere),
                Prompt::ServerRuntime => self.dispatch(Command::CloseServerRuntime),
                Prompt::Details { .. } if self.session.state().documentation().is_some() => {
                    self.dispatch(Command::CloseDocumentation)
                }
                _ => {}
            }
        }
    }
    fn highlighted_theme(&self) -> Option<String> {
        let modal = self.modal.as_ref().filter(|m| m.kind == Prompt::Themes)?;
        if modal.at == 0 {
            Some("system".into())
        } else {
            self.session
                .theme_entries()
                .get(modal.at - 1)
                .map(|e| e.id.clone())
        }
    }
    fn select_theme_row(&mut self, id: &str) {
        let at = self
            .session
            .theme_entries()
            .iter()
            .position(|e| e.id == id)
            .map_or(0, |at| at + 1);
        if let Some(modal) = &mut self.modal
            && modal.at != at
        {
            modal.at = at;
            modal.reveal_selection = true;
        }
    }
    pub fn choices(&self) -> Vec<Choice> {
        self.choices_for(self.layout.modal_rows.width.saturating_sub(2).max(1) as usize)
    }
    pub(crate) fn choices_for(&self, width: usize) -> Vec<Choice> {
        let Some(modal) = &self.modal else {
            return Vec::new();
        };
        let query = modal.input.to_lowercase();
        let simple = |label: String, target| Choice {
            label,
            detail: None,
            disabled: None,
            target,
        };
        let choices: Vec<Choice> = match modal.kind {
            Prompt::Context { ref entries, .. } => entries
                .iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Command(id) => Some(Choice {
                        label: id.label().into(),
                        detail: None,
                        disabled: self.session.disabled(*id),
                        target: ChoiceTarget::Action(*id),
                    }),
                    MenuEntry::Separator => None,
                })
                .collect(),
            Prompt::ServerRuntime => {
                self.session
                    .state()
                    .server_runtime()
                    .map_or_else(Vec::new, |model| {
                        model
                            .choices(&modal.input)
                            .into_iter()
                            .map(|(label, path)| Choice {
                                label,
                                detail: None,
                                disabled: self.session.disabled(CommandId::ServerRuntime),
                                target: ChoiceTarget::ServerRuntime(path),
                            })
                            .chain(
                                crate::render::wrap_text(&model.message, width)
                                    .into_iter()
                                    .map(|label| simple(label, ChoiceTarget::Information)),
                            )
                            .collect()
                    })
            }
            Prompt::LanguageServer => self.session.language_server_rows().into_iter()
                .filter_map(|(label, action)| action.map(|id| Choice {
                    label,
                    detail: None,
                    disabled: self.session.disabled(id),
                    target: ChoiceTarget::Action(id),
                })).collect(),
            Prompt::Menu(group) => self
                .session
                .menu(group)
                .into_iter()
                .map(|r| Choice {
                    label: r.label,
                    detail: None,
                    disabled: r.disabled,
                    target: match r.target {
                        MenuTarget::Run(id) => ChoiceTarget::Action(id),
                        _ => ChoiceTarget::Information,
                    },
                })
                .collect(),
            Prompt::Commands => self
                .session
                .actions()
                .into_iter()
                .map(|a| Choice {
                    label: format!(
                        "{}: {}{}",
                        a.id.group().label(),
                        a.on.map_or("", |on| if on { "✓ " } else { "○ " }),
                        a.label
                    ),
                    detail: a.disabled.map(str::to_owned),
                    disabled: a.disabled,
                    target: ChoiceTarget::Action(a.id),
                })
                .collect(),
            Prompt::RecentRoots => self
                .session
                .state()
                .recent()
                .iter()
                .map(|p| simple(p.display().to_string(), ChoiceTarget::Path(p.clone())))
                .collect(),
            Prompt::Themes => std::iter::once(simple(
                "Follow system".into(),
                ChoiceTarget::Theme("system".into()),
            ))
            .chain(
                self.session
                    .theme_entries()
                    .iter()
                    .map(|e| simple(e.label.clone(), ChoiceTarget::Theme(e.id.clone()))),
            )
            .collect(),
            Prompt::Places(list) => self
                .session
                .state()
                .place_rows(list, &modal.input)
                .into_iter()
                .map(|r| Choice {
                    label: r.label,
                    detail: Some(r.detail),
                    disabled: r.missing.then_some("File is missing"),
                    target: ChoiceTarget::Place(r.place),
                })
                .collect(),
            Prompt::Siblings { pane, crumb } => self
                .session
                .state()
                .siblings(pane, crumb)
                .into_iter()
                .enumerate()
                .map(|(index, s)| {
                    simple(
                        format!("{}{}", s.label, if s.dir { "/" } else { "" }),
                        ChoiceTarget::Sibling { pane, crumb, index },
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        if matches!(
            modal.kind,
            Prompt::Places(_) | Prompt::ServerRuntime | Prompt::Themes
        ) || query.is_empty()
        {
            choices
        } else {
            choices
                .into_iter()
                .filter(|c| c.label.to_lowercase().contains(&query))
                .collect()
        }
    }
    pub fn modal_title(&self) -> String {
        self.modal.as_ref().map_or_else(String::new, |m| {
            match &m.kind {
                Prompt::LanguageServer => "Language server status · ↑↓ scroll · Enter runs action",
                Prompt::Commands => "Commands · type to filter",
                Prompt::Context { .. } | Prompt::Menu(_) => "",
                Prompt::Open => "Open file · enter path",
                Prompt::OpenFolder => "Open folder · enter path",
                Prompt::Compare(_) => "Compare with · enter a file or folder path",
                Prompt::WorkspaceRoot => "Workspace root · empty restores discovery",
                Prompt::ServerRuntime => {
                    "Server runtime executable / install folder · Enter saves and restarts"
                }
                Prompt::RecentRoots => "Recent folders",
                Prompt::Navigation => "Search Everywhere · Tab scope · ↑↓ select · Enter open",
                Prompt::Search => "Search in Project",
                Prompt::Find => "Find in file · F3 next · Alt+C/W/R options",
                Prompt::Declarations => "Find declaration · Tab: workspace symbols",
                Prompt::Symbols => "Workspace symbols · Tab: declarations",
                Prompt::GoTo => "Go to line · line:column",
                Prompt::Themes => "Theme",
                Prompt::Places(PlaceList::Bookmarks) => "Bookmarks · Delete forgets",
                Prompt::Places(PlaceList::RecentFiles) => "Recent files · Delete forgets",
                Prompt::Places(PlaceList::RecentLocations) => "Recent locations · Delete forgets",
                Prompt::Mnemonic => "Bookmark key · press 0–9 or A–Z",
                Prompt::Siblings { .. } => "Breadcrumb entries",
                Prompt::Help => "Reader help",
                Prompt::Details { title, .. } => title,
            }
            .to_owned()
        })
    }
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => {
                if let KeyCode::Modifier(modifier) = key.code {
                    if matches!(
                        modifier,
                        ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift
                    ) {
                        let other = key.modifiers.intersects(
                            KeyModifiers::CONTROL
                                | KeyModifiers::ALT
                                | KeyModifiers::SUPER
                                | KeyModifiers::HYPER
                                | KeyModifiers::META,
                        );
                        if key.kind != KeyEventKind::Repeat
                            && self.double_shift.update(
                                key.kind == KeyEventKind::Press,
                                other,
                                Instant::now(),
                            )
                            && self.modal.is_none()
                        {
                            self.open_navigation(Scope::All);
                            return true;
                        }
                    } else {
                        self.double_shift.interrupt();
                    }
                    return false;
                }
                self.double_shift.interrupt();
                if key.kind == KeyEventKind::Release {
                    return false;
                }
                self.hover = None;
                // Ctrl+Q is the one action supported through every modal.
                if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.quit = true;
                    return true;
                }
                if self.modal.is_some() {
                    self.modal_key(key);
                } else {
                    self.key(key);
                }
            }
            Event::Paste(text) if self.modal.is_some() => {
                self.double_shift.interrupt();
                self.edit_prompt(&text, false);
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Moved => {
                self.double_shift.interrupt();
                let before = self.modal.as_ref().map(|m| m.at);
                self.mouse(mouse);
                return before != self.modal.as_ref().map(|m| m.at);
            }
            Event::Mouse(mouse) => {
                self.double_shift.interrupt();
                self.mouse(mouse);
            }
            Event::FocusGained => self.dispatch(Command::Recheck),
            Event::Resize(_, _) => {
                self.hover = None;
                self.clamp();
            }
            Event::FocusLost => {
                self.double_shift.reset();
                self.hover = None;
                self.drag = None;
            }
            _ => {}
        }
        true
    }
    fn edit_prompt(&mut self, text: &str, backspace: bool) {
        let Some(modal) = &mut self.modal else { return };
        if matches!(
            modal.kind,
            Prompt::Help
                | Prompt::Details { .. }
                | Prompt::Mnemonic
                | Prompt::Context { .. }
                | Prompt::Menu(_)
                | Prompt::LanguageServer
                | Prompt::Themes
        ) {
            return;
        }
        let target = if modal.secondary_focus {
            &mut modal.secondary
        } else {
            &mut modal.input
        };
        if backspace {
            target.pop();
        } else {
            target.extend(text.chars().filter(|c| !c.is_control()));
        }
        modal.at = 0;
        modal.top = 0;
        modal.reveal_selection = true;
        modal.error = None;
        let (kind, query) = (modal.kind.clone(), modal.input.clone());
        match kind {
            Prompt::Navigation => {
                if let Some(search) = self.session.state().navigation() {
                    self.dispatch(Command::SearchEverywhere {
                        scope: search.scope,
                        query,
                    });
                }
            }
            Prompt::Find => self.dispatch(Command::Find(query)),
            Prompt::Declarations => self.dispatch(Command::FindDeclarations(query)),
            Prompt::Symbols => self.dispatch(Command::Symbols(query)),
            _ => {}
        }
    }
    // Callers require an open modal; branches that dismiss or replace it return before accessing it again.
    fn modal_key(&mut self, key: KeyEvent) {
        let kind = self.modal.as_ref().unwrap().kind.clone();
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if kind == Prompt::LanguageServer
            && control
            && !alt
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.run(CommandId::CopyServerDetails);
            return;
        }
        if let Prompt::Details { body, .. } = &kind
            && control && !alt && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.copy = Some(body.clone());
            return;
        }
        // As in the GUI: Left or Escape goes back from a group to the groups, Right opens a group.
        match (&kind, key.code) {
            (Prompt::Menu(Some(group)), KeyCode::Left | KeyCode::Esc) => {
                return self.open_menu(None, CommandGroup::ALL.iter().position(|g| g == group).unwrap_or(0));
            }
            (Prompt::Menu(None), KeyCode::Right) => return self.accept(),
            _ => {}
        }
        if key.code == KeyCode::Esc {
            self.close_modal();
            if kind == Prompt::Find {
                self.dispatch(Command::CloseFind);
            }
            return;
        }
        if kind == Prompt::Mnemonic {
            if let KeyCode::Char(c) = key.code
                && c.is_ascii_alphanumeric()
                && !control
                && !alt
            {
                self.close_modal();
                self.dispatch(Command::SetMnemonic(c.to_ascii_uppercase()));
            }
            return;
        }
        if kind == Prompt::Navigation && control {
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            let scope = match (key.code, shift, alt) {
                (KeyCode::Char('p' | 'P'), false, false)
                | (KeyCode::Char('n' | 'N'), true, false) => Some(Scope::Files),
                (KeyCode::Char('n' | 'N'), true, true) => Some(Scope::Symbols),
                (KeyCode::Char('a' | 'A' | 'p' | 'P'), true, false) => Some(Scope::Actions),
                (KeyCode::Char('k' | 'K'), false, false) => Some(Scope::All),
                (KeyCode::Char('e' | 'E'), false, false) => Some(Scope::Recent),
                _ => None,
            };
            if let Some(scope) = scope {
                self.navigation_scope(scope);
                return;
            }
            if !shift && !alt && matches!(key.code, KeyCode::Char('a' | 'A')) {
                if let Some(modal) = &mut self.modal {
                    modal.input.clear();
                }
                self.edit_prompt("", false);
                return;
            }
        }
        if alt {
            let command = match key.code {
                KeyCode::Char('c') => Some(CommandId::MatchCase),
                KeyCode::Char('w') => Some(CommandId::WholeWord),
                KeyCode::Char('r') => Some(CommandId::RegularExpression),
                _ => None,
            };
            if matches!(kind, Prompt::Find | Prompt::Search)
                && let Some(id) = command
            {
                self.run(id);
            }
            return;
        }
        match key.code {
            KeyCode::Enter => self.accept(),
            KeyCode::F(3) if kind == Prompt::Find => {
                self.run(if key.modifiers.contains(KeyModifiers::SHIFT) {
                    CommandId::PreviousMatch
                } else {
                    CommandId::NextMatch
                })
            }
            KeyCode::Tab | KeyCode::BackTab if kind == Prompt::Navigation => {
                if let Some(search) = self.session.state().navigation() {
                    if search.scope == Scope::Recent {
                        return;
                    }
                    let at = Scope::ALL
                        .iter()
                        .position(|s| *s == search.scope)
                        .unwrap_or(0);
                    let previous =
                        key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                    self.navigation_scope(Scope::ALL[(at + if previous { 3 } else { 1 }) % 4]);
                }
            }
            KeyCode::Tab if kind == Prompt::Search => {
                let m = self.modal.as_mut().unwrap();
                m.secondary_focus = !m.secondary_focus;
            }
            KeyCode::Tab if matches!(kind, Prompt::Declarations | Prompt::Symbols) => {
                let m = self.modal.as_mut().unwrap();
                m.kind = if kind == Prompt::Declarations {
                    Prompt::Symbols
                } else {
                    Prompt::Declarations
                };
                self.edit_prompt("", false);
            }
            KeyCode::Delete if kind == Prompt::Navigation => {
                if self
                    .session
                    .state()
                    .navigation()
                    .is_some_and(|s| s.scope == Scope::Recent)
                    && let Some(revision) = self.layout.navigation_revision
                    && let Some(index) = self.modal.as_ref().map(|m| m.at)
                {
                    self.dispatch(Command::ForgetNavigation { revision, index });
                }
            }
            KeyCode::Delete if matches!(kind, Prompt::Places(_)) => {
                if let Some(choice) = self.choices().get(self.modal.as_ref().unwrap().at)
                    && let ChoiceTarget::Place(place) = &choice.target
                {
                    self.dispatch(Command::ForgetPlace(place.clone()));
                }
                self.clamp();
            }
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Home
            | KeyCode::End => {
                let count = if matches!(kind, Prompt::Help | Prompt::Details { .. }) {
                    self.layout.modal_content_rows
                } else {
                    self.modal_len()
                };
                let height = self.layout.modal_rows.height;
                let page = if kind == Prompt::Themes {
                    8
                } else if kind == Prompt::Navigation && height >= 4 {
                    height / 2
                } else {
                    height
                };
                let m = self.modal.as_mut().unwrap();
                if matches!(kind, Prompt::Help | Prompt::Details { .. }) {
                    m.top = stepped(m.top, count, key.code, page.max(1) as usize)
                        .min(count.saturating_sub(height as usize));
                } else {
                    m.at = stepped(m.at, count, key.code, page.max(1) as usize);
                    m.reveal_selection = true;
                }
                self.clamp();
                if kind == Prompt::Themes {
                    self.dispatch(Command::PreviewTheme(self.highlighted_theme()));
                }
            }
            KeyCode::Backspace if !control => self.edit_prompt("", true),
            KeyCode::Char('u') if control => {
                let m = self.modal.as_mut().unwrap();
                if m.secondary_focus {
                    m.secondary.clear();
                } else {
                    m.input.clear();
                }
                self.edit_prompt("", false);
            }
            KeyCode::Char(c) if !control => self.edit_prompt(&c.to_string(), false),
            _ => {}
        }
    }
    // The initial Some guard keeps the modal present in error branches; successful dismissals do not reuse it.
    fn accept(&mut self) {
        let Some(m) = self.modal.as_ref() else { return };
        let (kind, input, secondary, at) =
            (m.kind.clone(), m.input.clone(), m.secondary.clone(), m.at);
        match kind {
            Prompt::Navigation => {
                if let Some(search) = self.session.state().navigation() {
                    self.accept_navigation(search.revision, at);
                }
            }
            Prompt::Menu(group) => match self.session.menu(group).get(at).map(|r| r.target) {
                Some(MenuTarget::Group(group)) => self.open_menu(Some(group), 1),
                Some(MenuTarget::Back) => self.open_menu(
                    None,
                    CommandGroup::ALL.iter().position(|g| Some(*g) == group).unwrap_or(0),
                ),
                Some(MenuTarget::Run(id)) => {
                    self.close_modal();
                    self.run(id);
                }
                Some(MenuTarget::AllCommands) => self.open_navigation(Scope::Actions),
                None => {}
            },
            Prompt::Context { .. } => {
                let choice = self.choices().get(at).cloned();
                self.close_modal();
                if let Some(Choice {
                    target: ChoiceTarget::Action(id),
                    ..
                }) = choice
                {
                    self.run(id);
                }
            }
            Prompt::GoTo => match parse_line_column(&input) {
                Ok(at) => {
                    self.close_modal();
                    self.dispatch(Command::GoToLine(at));
                }
                Err(_) => {
                    self.modal.as_mut().unwrap().error =
                        Some("Enter a positive line or line:column".into())
                }
            },
            Prompt::Compare(left) => {
                let exe = std::env::current_exe().and_then(std::fs::canonicalize).unwrap_or_default();
                let launch = match input.trim().is_empty() {
                    true => Err("Enter a file or folder path".to_string()),
                    false => self.session.compare_launch(&left, &self.resolve_path(&input), &exe, true),
                };
                match launch {
                    Ok(launch) => {
                        self.close_modal();
                        self.foreground = Some(launch);
                    }
                    Err(e) => {
                        if let Some(modal) = self.modal.as_mut() {
                            modal.error = Some(e);
                        }
                    }
                }
            }
            Prompt::Open | Prompt::OpenFolder => {
                let path = self.resolve_path(&input);
                if input.trim().is_empty()
                    || !path.exists()
                    || (kind == Prompt::OpenFolder && !path.is_dir())
                    || (kind == Prompt::Open && !path.is_file())
                {
                    self.modal.as_mut().unwrap().error =
                        Some("Enter an existing file or folder path".into());
                } else {
                    self.close_modal();
                    if kind == Prompt::OpenFolder {
                        self.open_root(path);
                    } else {
                        self.dispatch(Command::Open(path));
                    }
                }
            }
            Prompt::ServerRuntime => {
                if let Some(Choice {
                    target: ChoiceTarget::ServerRuntime(path),
                    ..
                }) = self.choices().get(at).cloned()
                {
                    if let Some(modal) = &mut self.modal {
                        modal.input = path
                            .as_ref()
                            .map_or_else(String::new, |p| p.display().to_string());
                    }
                    self.dispatch(Command::SetServerRuntime(path));
                }
            }
            Prompt::WorkspaceRoot => {
                let path = (!input.trim().is_empty()).then(|| self.resolve_path(&input));
                if path.as_ref().is_some_and(|p| !p.is_dir()) {
                    self.modal.as_mut().unwrap().error =
                        Some("Workspace root must be a folder".into());
                } else {
                    self.close_modal();
                    self.dispatch(Command::SetWorkspaceRoot(path));
                }
            }
            Prompt::Search => {
                if self.session.search_refusal(&input).is_some() {
                    return;
                }
                self.close_modal();
                self.dispatch(Command::Search {
                    query: input,
                    scope: if secondary.trim().is_empty() {
                        SearchIn::Project
                    } else {
                        SearchIn::Matching(secondary)
                    },
                });
            }
            Prompt::Find => {
                self.close_modal();
                self.run(CommandId::NextMatch);
            }
            Prompt::Declarations | Prompt::Symbols => {
                self.close_modal();
                self.focus = Focus::Results;
            }
            Prompt::Help | Prompt::Details { .. } => self.close_modal(),
            Prompt::Mnemonic => {}
            _ => {
                let Some(choice) = self.choices().get(at).cloned() else {
                    return;
                };
                if matches!(choice.target, ChoiceTarget::Information) {
                    return;
                }
                if let Some(why) = choice.disabled {
                    self.modal.as_mut().unwrap().error = Some(why.into());
                    return;
                }
                self.close_modal();
                match choice.target {
                    ChoiceTarget::Information | ChoiceTarget::ServerRuntime(_) => {}
                    ChoiceTarget::Action(id) => self.run(id),
                    ChoiceTarget::Path(path) if kind == Prompt::RecentRoots => self.open_root(path),
                    ChoiceTarget::Path(path) => self.dispatch(Command::Open(path)),
                    ChoiceTarget::Theme(id) => {
                        if let Some(theme) = ThemeSelection::from_id(&id) {
                            self.dispatch(Command::Theme(ThemeCommand::Select(theme)));
                        }
                    }
                    ChoiceTarget::Place(place) => self.dispatch(Command::GoToPlace(place)),
                    ChoiceTarget::Sibling { pane, crumb, index } => {
                        self.dispatch(Command::FocusPane(pane));
                        if let Some(s) = self.session.state().siblings(pane, crumb).get(index) {
                            match s.target.clone() {
                                SiblingTarget::Folder(path) => {
                                    self.dispatch(Command::RevealInTree(path));
                                    self.tree_shown = true;
                                    self.focus = Focus::Tree;
                                }
                                SiblingTarget::File(path) => {
                                    if let Some(project) = self.session.state().project() {
                                        self.dispatch(Command::Open(project.root().join(&*path)));
                                    }
                                }
                                SiblingTarget::Line(line) => self.dispatch(Command::GoToLine(
                                    lenz_content::goto::LineColumn {
                                        line: (line + 1).try_into().unwrap_or(u32::MAX),
                                        column: Some(1),
                                    },
                                )),
                            }
                        }
                    }
                }
            }
        }
    }
    fn resolve_path(&self, input: &str) -> PathBuf {
        let input = input.trim();
        if let Some(rest) = input.strip_prefix("~/")
            && let Some(home) = std::env::var_os("HOME")
        {
            return PathBuf::from(home).join(rest);
        }
        let path = PathBuf::from(input);
        if path.is_relative() {
            self.session.state().project().map_or_else(
                || std::env::current_dir().unwrap_or_default().join(&path),
                |p| p.root().join(&path),
            )
        } else {
            path
        }
    }
    fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if alt && let KeyCode::Char(c @ '0'..='9') = key.code {
            self.dispatch(Command::JumpMnemonic(c));
            return;
        }
        if ctrl
            && shift
            && let KeyCode::Char(c @ '0'..='9') = key.code
        {
            self.dispatch(Command::SetMnemonic(c));
            return;
        }
        if alt && shift && !ctrl && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            let target = {
                let state = self.session.state();
                state.active().and_then(|tab| {
                    let pane = state.focused_pane();
                    state
                        .pane_tabs(pane)
                        .position(|t| t.id() == tab.id())
                        .map(|index| (tab.id(), pane, index))
                })
            };
            if let Some((tab, pane, index)) = target {
                let index = if key.code == KeyCode::Left {
                    index.saturating_sub(1)
                } else {
                    index.saturating_add(1)
                };
                self.dispatch(Command::MoveTab { tab, pane, index });
            }
            return;
        }
        let action = match (key.code, ctrl, shift, alt) {
            (KeyCode::F(10), _, _, _) | (KeyCode::Char(':'), false, _, _) => {
                self.prompt(Prompt::Commands);
                return;
            }
            (KeyCode::F(1), _, _, _) | (KeyCode::Char('?'), false, _, _) => {
                self.prompt(Prompt::Help);
                return;
            }
            (KeyCode::F(2), false, true, false) => {
                self.run(CommandId::StatusDetails);
                return;
            }
            (KeyCode::F(2), _, _, _) => {
                self.run(CommandId::LanguageServer);
                return;
            }
            (KeyCode::Char('z'), false, _, true) => Some(CommandId::SoftWrap),
            (KeyCode::Char('v' | 'V'), true, true, _) => Some(CommandId::MarkdownSource),
            (KeyCode::Char('-' | '_'), true, true, _) | (KeyCode::Char('_'), true, false, _) => {
                Some(CommandId::CollapseAll)
            }
            (KeyCode::Char('=' | '+'), true, true, _) | (KeyCode::Char('+'), true, false, _) => {
                Some(CommandId::ExpandAll)
            }
            (KeyCode::Char('-'), true, false, _) => Some(CommandId::Collapse),
            (KeyCode::Char('='), true, false, _) => Some(CommandId::Expand),
            (KeyCode::Char('o' | 'O'), true, true, _) => Some(CommandId::Outline),
            (KeyCode::Char('o'), true, false, _) => Some(CommandId::Open),
            (KeyCode::Char('p' | 'P'), true, true, false)
            | (KeyCode::Char('a' | 'A'), true, true, false) => Some(CommandId::FindAction),
            (KeyCode::Char('n' | 'N'), true, true, true) => Some(CommandId::FindSymbol),
            (KeyCode::Char('n' | 'N'), true, true, false)
            | (KeyCode::Char('p'), true, false, false) => Some(CommandId::FindFile),
            (KeyCode::Char('k'), true, false, false) => Some(CommandId::SearchEverywhere),
            (KeyCode::Char('f' | 'F'), true, true, _) => Some(CommandId::SearchProject),
            (KeyCode::Char('f'), true, false, _) | (KeyCode::Char('/'), false, _, _) => {
                Some(CommandId::FindInFile)
            }
            (KeyCode::Char('g'), true, false, _) => Some(CommandId::GoToLine),
            (KeyCode::Char('r'), true, _, _) | (KeyCode::F(5), _, _, _) => Some(CommandId::Reload),
            (KeyCode::Char('a'), true, _, _) => Some(CommandId::SelectAll),
            (KeyCode::Char('c'), true, _, _) => Some(CommandId::Copy),
            (KeyCode::Char('w' | 'W'), true, true, _) => Some(CommandId::ShrinkSelection),
            (KeyCode::Char('w'), true, false, _) => Some(CommandId::ExtendSelection),
            (KeyCode::F(4), true, _, _) => Some(CommandId::Close),
            (KeyCode::Char('t'), true, _, _) => Some(CommandId::FindDeclarations),
            (KeyCode::Char('e' | 'E'), true, true, _) => {
                self.tree_shown = true;
                self.focus = Focus::Tree;
                return;
            }
            (KeyCode::Char('e'), true, false, true) => Some(CommandId::RecentLocations),
            (KeyCode::Char('e'), true, false, false) => Some(CommandId::RecentFiles),
            (KeyCode::Char('b'), true, false, true) => Some(CommandId::Implementation),
            (KeyCode::Char('b' | 'B'), true, true, _) => Some(CommandId::TypeDefinition),
            (KeyCode::Char('b'), true, false, false) => Some(CommandId::ToggleExplorer),
            (KeyCode::Char('h'), true, _, true) => Some(CommandId::CallHierarchy),
            (KeyCode::Char('h'), true, _, false) => Some(CommandId::TypeHierarchy),
            (KeyCode::Char('k' | 'K'), true, true, _) => Some(CommandId::PinTab),
            (KeyCode::Char('\\'), true, false, _) => Some(CommandId::SplitRight),
            (KeyCode::Char('\\' | '|'), true, true, _) => Some(CommandId::SplitDown),
            (KeyCode::Char('1'), true, _, _) => {
                self.dispatch(Command::FocusPane(0));
                return;
            }
            (KeyCode::Char('2'), true, _, _) => {
                self.dispatch(Command::FocusPane(1));
                return;
            }
            (KeyCode::Char('.'), true, true, _) => Some(CommandId::FocusBreadcrumbs),
            (KeyCode::Tab, true, true, _) | (KeyCode::BackTab, true, _, _) => {
                Some(CommandId::PreviousTab)
            }
            (KeyCode::Tab, true, false, _) => Some(CommandId::NextTab),
            (KeyCode::F(3), _, true, _) => Some(CommandId::PreviousMatch),
            (KeyCode::F(3), _, false, _) => Some(CommandId::NextMatch),
            (KeyCode::F(11), true, _, _) => Some(CommandId::SetBookmarkMnemonic),
            (KeyCode::F(11), false, true, _) => Some(CommandId::ShowBookmarks),
            (KeyCode::F(11), false, false, _) => Some(CommandId::ToggleBookmark),
            (KeyCode::F(12), _, _, true) => Some(CommandId::PeekDefinition),
            (KeyCode::F(12), _, true, _) => Some(CommandId::Usages),
            (KeyCode::F(12), _, false, _) => Some(CommandId::Definition),
            (KeyCode::Left, true, _, true) => {
                self.dispatch(Command::ScrollColumn(
                    self.session.state().column().saturating_sub(4),
                ));
                return;
            }
            (KeyCode::Right, true, _, true) => {
                self.dispatch(Command::ScrollColumn(
                    self.session.state().column().saturating_add(4),
                ));
                return;
            }
            (KeyCode::Left, _, _, true) => Some(CommandId::Back),
            (KeyCode::Right, _, _, true) => Some(CommandId::Forward),
            _ => None,
        };
        if let Some(id) = action {
            if key.kind != KeyEventKind::Repeat
                || !matches!(
                    id,
                    CommandId::SearchEverywhere
                        | CommandId::FindFile
                        | CommandId::FindSymbol
                        | CommandId::FindAction
                        | CommandId::RecentFiles
                )
            {
                self.run(id);
            }
            return;
        }
        if key.code == KeyCode::Esc {
            if self.session.state().documentation().is_some() {
                self.dispatch(Command::CloseDocumentation);
            } else if self.session.state().asking().is_some() {
                self.dispatch(Command::CancelSemantic);
            } else {
                self.focus = Focus::Text;
                self.notice = None;
            }
            return;
        }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            let mut available = vec![Focus::Text];
            if self.tree_shown && self.session.state().has_tree() {
                available.push(Focus::Tree);
            }
            if self.session.state().results().is_some() {
                available.push(Focus::Results);
            }
            let at = available.iter().position(|f| *f == self.focus).unwrap_or(0);
            let delta = if shift || key.code == KeyCode::BackTab {
                available.len() - 1
            } else {
                1
            };
            self.focus = available[(at + delta) % available.len()];
            return;
        }
        if ctrl {
            return;
        }
        match self.focus {
            Focus::Tree => {
                let len = self.session.state().tree_rows().count();
                if matches!(
                    key.code,
                    KeyCode::Enter | KeyCode::Right | KeyCode::Left | KeyCode::Char(' ')
                ) {
                    self.tree_activate(key.code);
                } else {
                    self.tree_at = stepped(
                        self.tree_at,
                        len,
                        key.code,
                        self.layout.tree.height.max(1) as usize,
                    );
                }
            }
            Focus::Results => {
                let len = self.session.state().results().map_or(0, |r| r.rows().len());
                if matches!(
                    key.code,
                    KeyCode::Enter | KeyCode::Right | KeyCode::Left | KeyCode::Char(' ')
                ) {
                    self.result_activate(key.code == KeyCode::Enter);
                } else {
                    self.results_at = stepped(
                        self.results_at,
                        len,
                        key.code,
                        self.layout.results.height.max(1) as usize,
                    );
                    if let Some(Row::Hit(index)) = self
                        .session
                        .state()
                        .results()
                        .and_then(|r| r.rows().get(self.results_at))
                    {
                        self.dispatch(Command::SelectResult(*index));
                    }
                }
            }
            Focus::Breadcrumbs => {
                let pane = self.session.state().focused_pane();
                let len = self
                    .session
                    .state()
                    .breadcrumbs_for(pane)
                    .map_or(0, |b| b.crumbs.len());
                match key.code {
                    KeyCode::Left => self.crumb_at = self.crumb_at.saturating_sub(1),
                    KeyCode::Right => {
                        self.crumb_at = (self.crumb_at + 1).min(len.saturating_sub(1))
                    }
                    KeyCode::Enter | KeyCode::Down | KeyCode::Char(' ') => {
                        self.prompt(Prompt::Siblings {
                            pane,
                            crumb: self.crumb_at,
                        })
                    }
                    _ => {}
                }
            }
            Focus::Text => {
                let motion = match key.code {
                    KeyCode::Left => Some(Motion::Left),
                    KeyCode::Right => Some(Motion::Right),
                    KeyCode::Up => Some(Motion::Up),
                    KeyCode::Down => Some(Motion::Down),
                    KeyCode::Home => Some(Motion::LineStart),
                    KeyCode::End => Some(Motion::LineEnd),
                    _ => None,
                };
                if shift && let Some(motion) = motion {
                    self.dispatch(Command::Extend(motion));
                    return;
                }
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
                ) && self.session.state().active().is_some_and(|tab| {
                    tab.document().is_some()
                        && !matches!(
                            tab.rendering(),
                            Some(Rendering::Preview(_) | Rendering::Pending)
                        )
                }) && let Some(motion) = motion
                {
                    self.dispatch(Command::MoveCaret(motion));
                    return;
                }
                let page = self
                    .layout
                    .panes
                    .iter()
                    .find(|p| p.pane == self.session.state().focused_pane())
                    .map_or(20, |p| p.text.height.max(1) as isize);
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => self.dispatch(Command::ScrollRows(1)),
                    KeyCode::Up | KeyCode::Char('k') => self.dispatch(Command::ScrollRows(-1)),
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        self.dispatch(Command::ScrollRows(page))
                    }
                    KeyCode::PageUp => self.dispatch(Command::ScrollRows(-page)),
                    KeyCode::Home => {
                        self.dispatch(Command::ScrollColumn(0));
                        self.dispatch(Command::Scroll(0));
                    }
                    KeyCode::End => self.dispatch(Command::Scroll(usize::MAX)),
                    KeyCode::Left => self.dispatch(Command::ScrollColumn(
                        self.session.state().column().saturating_sub(4),
                    )),
                    KeyCode::Right => self.dispatch(Command::ScrollColumn(
                        self.session.state().column().saturating_add(4),
                    )),
                    KeyCode::Char('q') => self.quit = true,
                    _ => {}
                }
            }
        }
        self.clamp();
    }
    fn tree_activate(&mut self, key: KeyCode) {
        let Some(row) = self.session.state().tree_rows().nth(self.tree_at).cloned() else { return };
        if let Some(open) = row.dir {
            if !(key == KeyCode::Left && !open || key == KeyCode::Right && open) {
                self.dispatch(Command::ToggleDir(row.path.to_string()));
            }
        } else if key != KeyCode::Left {
            self.dispatch(Command::ActivateTree { path: row.path, retain: false });
            self.focus = Focus::Text;
        }
    }
    fn result_activate(&mut self, open: bool) {
        let Some(row) = self
            .session
            .state()
            .results()
            .and_then(|r| r.rows().get(self.results_at))
            .cloned()
        else {
            return;
        };
        match row {
            Row::Dir { dir, .. } => self.dispatch(Command::ToggleResultGroup(Group::Dir(dir))),
            Row::File { path, .. } => self.dispatch(Command::ToggleResultGroup(Group::File(path))),
            Row::Hit(index) if open => {
                self.dispatch(Command::OpenResult(index));
                self.focus = Focus::Text;
            }
            Row::Hit(index) => {
                let hierarchy = self.session.state().results().is_some_and(|r| {
                    r.results_kind() == reader_core::search::ResultsKind::Hierarchy
                });
                self.dispatch(if hierarchy {
                    Command::ToggleCall(index)
                } else {
                    Command::SelectResult(index)
                });
            }
        }
    }
    fn hover_at(&mut self, x: u16, y: u16) {
        let target = if self.session.state().documentation_on_hover() && self.modal.is_none() {
            self.layout
                .panes
                .iter()
                .filter(|p| !p.preview)
                .find_map(|pane| {
                    if !pane.text.contains(ratatui::layout::Position::new(x, y)) {
                        return None;
                    }
                    let hit = pane
                        .hits
                        .iter()
                        .find(|h| h.y == y && h.x <= x && x < h.x.saturating_add(h.width.max(1)))?;
                    let tab = self.session.state().shown(pane.pane)?;
                    let doc = tab.document()?;
                    if hit.line >= doc.line_count() || tab.linkable(hit.line, hit.byte).is_none() {
                        return None;
                    }
                    Some((
                        tab.id(),
                        doc.version(),
                        doc.line_range(hit.line).start + hit.byte,
                        tab.top_line(),
                    ))
                })
        } else {
            None
        };
        self.hover = target.map(|(tab, version, offset, top)| {
            let due = self
                .hover
                .filter(|h| {
                    h.tab == tab && h.version == version && h.offset == offset && h.top == top
                })
                .map_or_else(|| Instant::now() + Duration::from_millis(600), |h| h.due);
            Hover {
                tab,
                version,
                offset,
                top,
                due,
            }
        });
    }
    fn mouse(&mut self, event: MouseEvent) {
        let (x, y) = (event.column, event.row);
        let point = ratatui::layout::Position::new(x, y);
        if event.kind == MouseEventKind::Moved {
            if matches!(
                self.modal.as_ref().map(|m| &m.kind),
                Some(Prompt::Context { .. } | Prompt::Menu(_))
            ) {
                if self.layout.modal_rows.contains(point)
                    && let Some(&at) = self
                        .layout
                        .modal_items
                        .get(usize::from(y - self.layout.modal_rows.y))
                    && at < self.choices().len()
                    && let Some(modal) = &mut self.modal
                {
                    modal.at = at;
                }
                return;
            }
            self.hover_at(x, y);
            return;
        }
        self.hover = None;
        if self.modal.is_some() {
            // Project search: a toggle flips its option and a field takes the keys; a click outside closes it.
            if self.modal.as_ref().is_some_and(|m| m.kind == Prompt::Search)
                && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            {
                if let Some(&(id, _)) = self
                    .layout
                    .search_toggles
                    .iter()
                    .find(|(_, r)| r.contains(point))
                {
                    self.run(id);
                } else if let Some(i) = self
                    .layout
                    .search_fields
                    .iter()
                    .position(|r| r.contains(point))
                    && let Some(modal) = &mut self.modal
                {
                    modal.secondary_focus = i == 1;
                } else if !self.layout.modal.contains(point) {
                    self.close_modal();
                }
                return;
            }
            if self
                .modal
                .as_ref()
                .is_some_and(|m| m.kind == Prompt::Navigation)
                && matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            {
                if let Some((scope, _)) = self
                    .layout
                    .navigation_scopes
                    .iter()
                    .find(|(_, r)| r.contains(point))
                {
                    self.navigation_scope(*scope);
                    return;
                }
                if self.layout.modal_rows.contains(point) {
                    if let (Some(revision), Some(&index)) = (
                        self.layout.navigation_revision,
                        self.layout
                            .modal_items
                            .get(usize::from(y - self.layout.modal_rows.y)),
                    ) {
                        self.accept_navigation(revision, index);
                    }
                    return;
                }
            }
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(Modal { kind: Prompt::Details { body, .. }, .. }) = &self.modal
            {
                if self.layout.details_copy.contains(point) {
                    self.copy = Some(body.clone());
                    return;
                }
                if self.layout.details_close.contains(point) {
                    self.close_modal();
                    return;
                }
            }
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                && self.layout.modal_rows.contains(point)
            {
                let screen_row = y.saturating_sub(self.layout.modal_rows.y) as usize;
                let Some(&row) = self.layout.modal_items.get(screen_row) else {
                    return;
                };
                if row < self.choices().len() {
                    self.modal.as_mut().unwrap().at = row;
                    self.accept();
                }
            } else if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                && !self.layout.modal.contains(point)
            {
                self.close_modal();
            } else if matches!(
                event.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            ) {
                let visible = self.modal_visible_rows();
                let count = self.layout.modal_content_rows;
                if let Some(modal) = &mut self.modal {
                    modal.reveal_selection = false;
                    modal.top = if event.kind == MouseEventKind::ScrollDown {
                        modal.top.saturating_add(1)
                    } else {
                        modal.top.saturating_sub(1)
                    }
                    .min(count.saturating_sub(visible));
                }
            }
            return;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) && self.layout.app_menu.contains(point) {
            self.open_menu(None, 0);
            return;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && self.layout.command_menus.iter().any(|rect| rect.contains(point))
        {
            self.prompt(Prompt::Commands);
            return;
        }
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Right)) {
            self.context_menu(x, y);
            return;
        }
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some((id, _)) = self
                .layout
                .controls
                .iter()
                .find(|(_, rect)| rect.contains(point))
                .copied()
        {
            if let Some(pane) = self.layout.panes.iter().find(|p| p.controls.iter().any(|(_, r)| r.contains(point))).map(|p| p.pane) {
                self.focus = Focus::Text;
                self.dispatch(Command::FocusPane(pane));
            }
            self.run(id);
            return;
        }
        if matches!(event.kind, MouseEventKind::Up(MouseButton::Left)) {
            self.drag = None;
            return;
        }
        if matches!(
            event.kind,
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
        ) {
            let drag = matches!(event.kind, MouseEventKind::Drag(_));
            if !drag && self.layout.tree.contains(point) {
                let row = self.tree_top + y.saturating_sub(self.layout.tree.y) as usize;
                if row < self.session.state().tree_rows().count()
                {
                    self.focus = Focus::Tree;
                    self.tree_at = row;
                    self.tree_activate(KeyCode::Enter);
                }
                return;
            }
            if !drag && self.layout.results.contains(point) {
                let row = self.results_top + y.saturating_sub(self.layout.results.y) as usize;
                if self
                    .session
                    .state()
                    .results()
                    .is_some_and(|r| row < r.rows().len())
                {
                    self.focus = Focus::Results;
                    self.results_at = row;
                    self.result_activate(false);
                }
                return;
            }
            for pane in &self.layout.panes {
                if !drag && let Some((tab, _)) = pane.tabs.iter().find(|(_, r)| r.contains(point)) {
                    let id = *tab;
                    self.dispatch(Command::FocusPane(pane.pane));
                    self.dispatch(Command::ActivateTab(id));
                    self.focus = Focus::Text;
                    return;
                }
                if !drag
                    && let Some((crumb, _)) = pane.crumbs.iter().find(|(_, r)| r.contains(point))
                {
                    let (pane, crumb) = (pane.pane, *crumb);
                    self.dispatch(Command::FocusPane(pane));
                    self.crumb_at = crumb;
                    self.prompt(Prompt::Siblings { pane, crumb });
                    return;
                }
                if pane.text.contains(point)
                    || (!drag && x + 1 == pane.text.x && y >= pane.text.y && y < pane.text.bottom())
                {
                    let pane_id = pane.pane;
                    let Some((_, line, _)) = pane.rows.iter().find(|(row, _, _)| *row == y) else {
                        return;
                    };
                    let line = *line;
                    if x < pane.text.x {
                        self.dispatch(Command::FocusPane(pane_id));
                        self.dispatch(Command::ToggleFold(line));
                        return;
                    }
                    let hits = || pane.hits.iter().filter(|h| h.y == y && h.line == line);
                    let byte = hits()
                        .find(|h| h.x <= x && x < h.x.saturating_add(h.width.max(1)))
                        .or_else(|| hits().filter(|h| h.x <= x).max_by_key(|h| h.x))
                        .map_or(0, |h| h.byte);
                    if pane.preview {
                        let Some(tab) = self.session.state().shown(pane_id) else {
                            return;
                        };
                        let (Some(doc), Some(Rendering::Preview(preview))) =
                            (tab.document(), tab.rendering())
                        else {
                            return;
                        };
                        let (id, version, len) = (tab.id(), doc.version(), preview.doc.text.len());
                        if drag
                            && self
                                .drag
                                .is_none_or(|(p, t, v, _)| p != pane_id || t != id || v != version)
                        {
                            self.drag = None;
                            return;
                        }
                        let head = byte.min(len);
                        let anchor = if drag {
                            self.drag
                                .filter(|(p, t, v, _)| *p == pane_id && *t == id && *v == version)
                                .map(|(_, _, _, a)| a)
                        } else {
                            None
                        }
                        .unwrap_or(head);
                        self.dispatch(Command::FocusPane(pane_id));
                        self.preview_selection = Some((id, version, Selection { anchor, head }));
                        self.drag = Some((pane_id, id, version, anchor));
                        self.focus = Focus::Text;
                        return;
                    }
                    let Some(tab) = self.session.state().shown(pane_id) else {
                        return;
                    };
                    let Some(doc) = tab.document() else { return };
                    let (id, version) = (tab.id(), doc.version());
                    if drag
                        && self
                            .drag
                            .is_none_or(|(p, t, v, _)| p != pane_id || t != id || v != version)
                    {
                        self.drag = None;
                        return;
                    }
                    let offset = (doc.line_range(line).start + byte).min(doc.text().len());
                    if !drag && !event.modifiers.contains(KeyModifiers::CONTROL) {
                        // Clicks on one cell within 400 ms count up: caret, word, group, then caret again.
                        let now = Instant::now();
                        let at = (pane_id, id, offset);
                        let count = self
                            .clicks
                            .filter(|(t, a, _)| *a == at && now.duration_since(*t) < Duration::from_millis(400))
                            .map_or(1, |(_, _, n)| n % 3 + 1);
                        self.clicks = Some((now, at, count));
                        if count > 1 {
                            self.dispatch(Command::FocusPane(pane_id));
                            self.dispatch(Command::ClickSelect { at: offset, clicks: count });
                            self.focus = Focus::Text;
                            self.drag = None;
                            return;
                        }
                    }
                    let anchor = if drag {
                        self.drag
                            .filter(|(p, t, v, _)| *p == pane_id && *t == id && *v == version)
                            .map(|(_, _, _, a)| a)
                    } else {
                        None
                    }
                    .unwrap_or(offset);
                    self.dispatch(Command::FocusPane(pane_id));
                    self.dispatch(Command::Select(Selection {
                        anchor,
                        head: offset,
                    }));
                    self.focus = Focus::Text;
                    self.drag = Some((pane_id, id, version, anchor));
                    if !drag && event.modifiers.contains(KeyModifiers::CONTROL) {
                        self.drag = None;
                        self.run(CommandId::Definition);
                    }
                    return;
                }
            }
        }
        if matches!(
            event.kind,
            MouseEventKind::ScrollDown
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ) {
            let down = matches!(
                event.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollRight
            );
            if self.layout.tree.contains(point) {
                self.tree_at = if down {
                    self.tree_at.saturating_add(3)
                } else {
                    self.tree_at.saturating_sub(3)
                };
            } else if self.layout.results.contains(point) {
                self.results_at = if down {
                    self.results_at.saturating_add(3)
                } else {
                    self.results_at.saturating_sub(3)
                };
            } else {
                if let Some(pane) = self
                    .layout
                    .panes
                    .iter()
                    .find(|p| p.text.contains(point))
                    .map(|p| p.pane)
                {
                    self.dispatch(Command::FocusPane(pane));
                }
                if matches!(
                    event.kind,
                    MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
                ) {
                    self.dispatch(Command::ScrollColumn(if down {
                        self.session.state().column().saturating_add(4)
                    } else {
                        self.session.state().column().saturating_sub(4)
                    }));
                } else {
                    self.dispatch(Command::ScrollRows(if down { 3 } else { -3 }));
                }
            }
        }
        self.clamp();
    }

    fn context_menu(&mut self, x: u16, y: u16) {
        let Some(pane) = self
            .layout
            .panes
            .iter()
            .find(|p| p.text.contains((x, y).into()))
        else {
            return;
        };
        let pane_id = pane.pane;
        let preview = pane.preview;
        let offset = (!preview)
            .then(|| {
                let line = pane.rows.iter().find(|(row, _, _)| *row == y)?.1;
                let byte = pane
                    .hits
                    .iter()
                    .filter(|h| h.y == y && h.line == line && h.x <= x)
                    .max_by_key(|h| h.x)
                    .map_or(0, |h| h.byte);
                let doc = self.session.state().shown(pane_id)?.document()?;
                Some((doc.line_range(line).start + byte).min(doc.text().len()))
            })
            .flatten();
        if self.session.state().shown(pane_id).is_none() {
            return;
        }
        self.dispatch(Command::FocusPane(pane_id));
        if let Some(at) = offset
            && !self
                .session
                .state()
                .selection()
                .is_some_and(|s| s.range().contains(&at))
        {
            self.dispatch(Command::Select(Selection {
                anchor: at,
                head: at,
            }));
        }
        self.focus = Focus::Text;
        let entries = self.session.context_menu();
        self.prompt(Prompt::Context { x, y, entries });
    }
    pub(crate) fn clamp(&mut self) {
        let tree = self.session.state().tree_rows().count();
        clamp_list(
            &mut self.tree_at,
            &mut self.tree_top,
            tree,
            self.layout.tree.height as usize,
        );
        let results = self.session.state().results().map_or(0, |r| r.rows().len());
        clamp_list(
            &mut self.results_at,
            &mut self.results_top,
            results,
            self.layout.results.height as usize,
        );
        if self.focus == Focus::Results && self.session.state().results().is_none() {
            self.focus = Focus::Text;
        }
        if self.focus == Focus::Tree
            && (!self.tree_shown || !self.session.state().has_tree())
        {
            self.focus = Focus::Text;
        }
        let visible = self.modal_visible_rows();
        if let Some(modal) = &mut self.modal {
            modal.top = modal
                .top
                .min(self.layout.modal_content_rows.saturating_sub(visible));
        }
        if let Some(modal) = &self.modal
            && !matches!(modal.kind, Prompt::Help | Prompt::Details { .. })
        {
            let len = self.modal_len();
            if let Some(modal) = &mut self.modal {
                modal.at = modal.at.min(len.saturating_sub(1));
            }
        }
    }
}
fn stepped(at: usize, len: usize, key: KeyCode, page: usize) -> usize {
    let last = len.saturating_sub(1);
    match key {
        KeyCode::Down | KeyCode::Char('j') => at.saturating_add(1).min(last),
        KeyCode::Up | KeyCode::Char('k') => at.saturating_sub(1).min(last),
        KeyCode::PageDown => at.saturating_add(page).min(last),
        KeyCode::PageUp => at.saturating_sub(page).min(last),
        KeyCode::Home => 0,
        KeyCode::End => last,
        _ => at.min(last),
    }
}
fn clamp_list(at: &mut usize, top: &mut usize, len: usize, rows: usize) {
    *at = (*at).min(len.saturating_sub(1));
    *top = (*top).min(len.saturating_sub(rows.max(1)));
    if *at < *top {
        *top = *at;
    }
    if *at >= top.saturating_add(rows.max(1)) {
        *top = at.saturating_add(1).saturating_sub(rows.max(1));
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;

impl Drop for App {
    fn drop(&mut self) {
        self.remember_explorer();
    }
}
