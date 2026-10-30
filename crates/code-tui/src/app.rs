//! Terminal input translates to the shared reader session; only focus and cell geometry live here.
#[path = "csv.rs"]
pub(crate) mod csv;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind,
};
use lenz_content::{
    goto::parse_line_column,
    theme::{ThemeCommand, ThemeSelection},
};
use code_core::{
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
use code_core::text::{EditKind, SnapshotKey};
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
    TerminalPath,
    RenameTerminal { id: code_core::terminal::Id },
    /// Save As: the destination path for this tab's document.
    SaveAs(code_core::tabs::TabId),
    NewScratch,
    ScratchFiles(Vec<PathBuf>),
    /// New File / New Folder: the name to create in the explorer's folder (`dir`, project-relative).
    Create { kind: code_core::create::Kind, dir: String },
    /// Rename / Move: the new name or folder for explorer entry `from` (project-relative).
    Relocate { kind: code_core::relocate::Kind, from: String },
    PasteEntry(code_core::session::PasteEntry),
    /// Delete: confirm removing explorer entry `from` (project-relative), into the trash or permanently.
    ConfirmDelete { removal: code_core::trash::Removal, from: String },
    /// Compare With: the other file or folder for this one.
    Compare(PathBuf),
    WorkspaceRoot,
    SqlMetadata,
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
    /// `kind`: plain text, or one of the core's dialogs whose text and buttons follow the core (the commit popup:
    /// Show Diff / Copy Revision / Close; the checkout problem: its choices).
    Details { title: &'static str, body: String, logo: bool, kind: Details },
    /// The Git Branches popup (the core's lists; `input` filters).
    Branches,
    /// A popup row's actions, and the popup's filter and row to go back to.
    BranchMenu { target: code_core::branches::BranchTarget, label: String, query: String, row: usize },
    /// A new branch's name (`input`), where it starts (`None`: HEAD) and whether to check it out (Tab).
    NewBranch { from: Option<String>, checkout: bool },
    /// Checkout Tag or Revision (`input`).
    Revision,
    /// Every command's chord: type to filter, Enter captures a new shortcut, Delete restores the preset.
    Keymap,
}
/// What a details prompt is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Details {
    Terminal(u64),
    Install,
    Sql,
    Change,
    Text,
    Commit,
    Checkout,
    Close,
    External,
    /// A dirty draft and a file that changed on disk: compare them, take disk, save over it or keep the draft.
    Reconcile,
    Replace,
    Recovered,
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
    /// The keymap modal takes the next key as this command's shortcut.
    pub capture: Option<CommandId>,
    /// The focused field's text is selected, so typing replaces it (a terminal's label, a reopened search's query).
    pub selected: bool,
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
    /// A keymap row: Enter captures the next key for this command.
    Keymap(CommandId),
    /// A Git Branches row (Enter runs its first action), or one of a row's actions.
    Branch(code_core::branches::BranchTarget),
    BranchAction(code_core::branches::BranchTarget, code_core::branches::BranchAction),
}
#[derive(Clone, Copy)]
struct Hover {
    tab: TabId,
    version: u64,
    offset: usize,
    top: usize,
    due: Instant,
}
/// Current preview coordinates retain metadata identity as well as text version.
#[derive(Clone, Debug)]
pub(crate) struct PreviewSource { key: code_core::text::SnapshotKey, metadata: std::sync::Arc<code_core::document::SourceMeta> }
impl PreviewSource {
    pub(crate) fn new(view: &code_core::session::View<'_>) -> Self { Self { key: view.snapshot().key(), metadata: view.snapshot().source_meta().clone() } }
    pub(crate) fn matches(&self, view: &code_core::session::View<'_>) -> bool { self.key == view.snapshot().key() && std::sync::Arc::ptr_eq(&self.metadata, view.snapshot().source_meta()) }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct TabStrip { pub first: usize, pub last: usize, pub shown: Option<TabId>, pub manual: bool }

pub(crate) struct App {
    /// `LENZ_ICONS=nerd|plain` for this run, over the File Icons setting until that is toggled here.
    pub icons_override: Option<bool>,
    pub session: Session,
    pub modal: Option<Modal>,
    pub focus: Focus,
    pub tree_at: usize,
    pub tree_top: usize,
    /// The highlighted rows the views last revealed: a moved highlight scrolls into view, the wheel does not move it.
    pub(crate) tree_seen: usize,
    pub results_at: usize,
    pub results_top: usize,
    pub(crate) results_seen: usize,
    pub tab_strips: std::cell::Cell<[TabStrip; 2]>,
    pub tree_shown: bool,
    pending_explorer: Option<code_core::workspace::Explorer>,
    /// The last explorer-reveal ticket applied, so a created entry is selected once.
    reveal_seen: u64,
    pub log_focus: bool,
    /// Capture the clicked path before background work can switch the active document.
    terminal_path_target: Option<Result<Command, String>>,
    pub(crate) terminal_drag: Option<code_core::terminal::Id>,
    pub log_scroll: usize,
    pub independent_view: Option<(usize, usize)>,
    pub quit: bool,
    pub quit_all: bool,
    pub new_view: bool,
    /// The folder the new view opens (Open in New Window on a worktree).
    pub new_view_root: Option<PathBuf>,
    pub copy: Option<String>,
    pub copy_ticket: Option<code_core::session::ClipboardTicket>,
    /// Lenz Diff's terminal app, for the runtime to run in this terminal until it quits.
    pub foreground: Option<lenz_content::open::Launch>,
    /// What Open in Editor started; tests record here instead of starting anything.
    #[cfg(test)]
    pub launched: Vec<lenz_content::open::Launch>,
    /// For tests: where Lenz Code is, so Lenz Diff is found beside it.
    #[cfg(test)]
    pub exe: Option<PathBuf>,
    pub layout: crate::render::Geometry,
    pub source_cache: std::cell::RefCell<crate::source::Cache>,
    pub(crate) source_redraw: bool,
    pub crumb_at: usize,
    drag: Option<(u8, TabId, u64, usize)>,
    /// The annotation edge being resized, fenced to its shown source.
    annotation_drag: Option<(u8, TabId, code_core::text::SnapshotKey)>,
    /// The explorer's width while its right edge is dragged; remembered in the layout on release.
    pub(crate) explorer_drag: Option<u16>,
    /// The first file pane's share (thousandths) while the panes' boundary is dragged; remembered on release.
    pub(crate) split_drag: Option<u32>,
    /// The results panel's share of the height while its heading is dragged.
    pub(crate) results_drag: Option<u32>,
    /// The dragged size when the button went down: a click that does not move it remembers nothing.
    divider_from: u32,
    /// The last text click (when, where) and how many came in a row there: two take the word, three its group.
    clicks: Option<(Instant, (u8, TabId, usize), u8)>,
    tree_click: Option<(Instant, PathBuf)>,
    hover: Option<Hover>,
    double_shift: DoubleShift,
    pub(crate) preview_selection: Option<(TabId, PreviewSource, Selection)>,
    pub(crate) preview_scroll: HashMap<TabId, (PreviewSource, usize)>,
}
impl App {
    pub fn new(config: Config, path: Option<PathBuf>) -> Self {
        Self::with_text_input(config, path, cfg!(any(target_os = "linux", target_os = "macos")))
    }
    fn with_text_input(config: Config, path: Option<PathBuf>, enabled: bool) -> Self {
        let mut app = Self {
            // Unit tests read plain labels unless they choose glyphs (the default has its own test).
            icons_override: if cfg!(test) { Some(false) } else { std::env::var("LENZ_ICONS").ok().map(|v| v == "nerd") },
            session: Session::with_text_input(config, enabled),
            modal: None,
            focus: Focus::Text,
            tree_at: 0,
            tree_top: 0,
            tree_seen: usize::MAX,
            results_at: 0,
            results_top: 0,
            results_seen: usize::MAX,
            tab_strips: Default::default(),
            tree_shown: true,
            pending_explorer: None,
            reveal_seen: 0,
            log_focus: false,
            terminal_path_target: None,
            terminal_drag: None,
            log_scroll: 0,
            independent_view: None,
            quit: false, quit_all: false,
            new_view: false,
            new_view_root: None,
            copy: None,
            copy_ticket: None,
            foreground: None,
            #[cfg(test)]
            launched: Vec::new(),
            #[cfg(test)]
            exe: None,
            layout: Default::default(),
            source_cache: Default::default(),
            source_redraw: false,
            crumb_at: 0,
            drag: None,
            annotation_drag: None,
            explorer_drag: None,
            split_drag: None,
            results_drag: None,
            divider_from: 0,
            clicks: None,
            tree_click: None,
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
    pub(crate) fn remember_explorer(&mut self) {
        if self.pending_explorer.is_some() {
            return;
        }
        if !self.session.state().has_tree() { return; }
        let explorer = code_core::workspace::Explorer {
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
            code_core::project::Coverage::Complete(_)
                | code_core::project::Coverage::Unavailable(_)
        );
        if (explorer.selected.is_none() || selected.is_some() || done)
            && (explorer.top.is_none() || top.is_some() || done)
        {
            self.tree_at = selected.unwrap_or(0);
            self.tree_top = top.unwrap_or(0);
            // A restored view keeps its saved scroll, even when its highlight is off screen.
            self.tree_seen = self.tree_at;
            self.pending_explorer = None;
        }
    }

    /// Show and select the row the session asked for, once per ticket.
    pub(crate) fn sync_reveal(&mut self) {
        let Some((path, ticket)) = self.session.state().tree_reveal() else { return };
        if ticket <= self.reveal_seen { return; }
        self.reveal_seen = ticket;
        if let Some(at) = self.session.state().tree_rows().position(|r| r.path == path) {
            self.tree_at = at;
            self.tree_seen = usize::MAX; // revealed even if it was already the highlight
            self.tree_shown = true;
            self.tree_top = self.tree_top.min(at);
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
                | Command::NewScratch(_)
                | Command::OpenScratch(_)
                | Command::ActivateTree { .. }
                | Command::OpenAt { .. }
                | Command::ActivateTab(_)
                | Command::Close
                | Command::CloseTab(_)
                | Command::CloseTabs { .. }
                | Command::OpenRoot(_)
        );
        let results = matches!(
            command,
            Command::CheckProjectSyntax
                | Command::ShowProblems
                | Command::Outline
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
        let show_results = matches!(command, Command::ToggleResults);
        // Another list, or another result of this one: the highlighted row follows its selection.
        let reselect = matches!(command, Command::ResultsHistory { .. } | Command::StepResult { .. } | Command::ToggleResults);
        if matches!(&command, Command::OpenRoot(_)) {
            self.remember_explorer();
        }
        let workspace = self.session.state().workspace_revision();
        let tree_revision = self.session.state().tree_revision();
        let terminal_focus = self.session.state().terminal().focused();
        let mut effect = self.session.dispatch(command);
        if terminal_focus != self.session.state().terminal().focused() && self.layout.terminal.area.bottom() < 7 { self.layout.invalidate_source(); }
        if let Some(ticket) = self.session.external_ready() {
            let delivery = self.session.dispatch(Command::TakeExternal { ticket });
            effect.repaint |= delivery.repaint;
            effect.external = delivery.external.or(effect.external);
        }
        self.sync_workspace(workspace);
        if self.session.state().tree_revision() != tree_revision {
            self.pending_explorer = None;
            self.tree_shown = true;
            self.tree_at = self.session.state().active_in_tree().and_then(|path| self.session.state().tree_rows().position(|r| r.path == path)).unwrap_or(0);
        }
        self.sync_reveal();
        if let Some(plan) = effect.external { self.start(plan.launch,plan.toast); }
        if let Some(copy) = effect.copy {
            self.copy = Some(copy);
            self.copy_ticket = effect.copy_ticket;
            self.session.dispatch(Command::Notice("Copied to terminal clipboard".into()));
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
        if reselect && let Some(r) = self.session.state().results() {
            self.results_at = r.selected().and_then(|i| r.row_of_hit(i)).unwrap_or(0);
        }
        // Shown results take the keys; hidden or closed ones give them back to the text.
        if self.focus == Focus::Results && self.session.state().results().is_none() {
            self.focus = Focus::Text;
        } else if show_results && self.session.state().results().is_some() {
            self.focus = Focus::Results;
        }
        self.clamp();
        self.sync_change_inspector();
    }
    pub fn next_deadline(&self) -> Option<Instant> {
        if self.source_redraw { Some(Instant::now()) } else { self.hover.map(|h| h.due) }
    }
    fn hover_current(&self, hover: Hover) -> bool {
        self.modal.is_none()
            && self.session.state().documentation_on_hover()
            && self.layout.panes.iter()
                .filter_map(|pane| pane.source_view(self))
                .any(|view| {
                    view.tab().id() == hover.tab
                        && view.snapshot().key().version == hover.version
                        && view.tab().top_line() == hover.top
                })
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        let repaint = std::mem::take(&mut self.source_redraw);
        let Some(hover) = self.hover else {
            return repaint;
        };
        if !self.hover_current(hover) {
            self.hover = None;
            return repaint;
        }
        if now < hover.due {
            return repaint;
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
        let mut effect = self.session.dispatch(completion);
        if let Some(ticket) = self.session.external_ready() {
            let delivery = self.session.dispatch(Command::TakeExternal { ticket });
            effect.repaint |= delivery.repaint;
            effect.external = delivery.external.or(effect.external);
        }
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
        self.sync_reveal();
        if self.session.state().results().map(|r| r.id()) != results_id {
            self.results_at = 0;
            self.results_top = 0;
        }
        self.invalidate_preview_positions(preview_positions);
        if self.hover.is_some_and(|h| !self.hover_current(h)) {
            self.hover = None;
        }
        if let Some(plan) = effect.external { self.start(plan.launch,plan.toast); }
        if let Some(copy) = effect.copy {
            self.copy = Some(copy);
            self.copy_ticket = effect.copy_ticket;
        }
        self.sync_preview_selection();
        self.clamp();
        self.show_documentation();
        effect.repaint
    }
    pub(crate) fn show_documentation(&mut self) {
        // The core's commit popup opens, follows and closes its dialog.
        for (kind, title, now) in [(Details::Commit, "Commit", self.session.commit_text()), (Details::Checkout, "Checkout", self.session.checkout_text())] {
            match (now, self.modal.as_mut()) {
                (Some(text), Some(Modal { kind: Prompt::Details { kind: k, body, .. }, .. })) if *k == kind => *body = text,
                (None, Some(Modal { kind: Prompt::Details { kind: k, .. }, .. })) if *k == kind => self.modal = None,
                (Some(body), None) => self.prompt(Prompt::Details { title, body, logo: false, kind }),
                _ => {}
            }
        }
        // The core's Git Branches popup opens and closes its list.
        match (self.session.state().branches().is_some(), self.modal.as_ref().map(|m| &m.kind)) {
            (true, None) => self.prompt(Prompt::Branches),
            (false, Some(Prompt::Branches | Prompt::BranchMenu { .. })) => self.modal = None,
            _ => {}
        }
        if self.modal.is_none()
            && let Some(doc) = self.session.state().documentation()
        {
            self.prompt(Prompt::Details { title: "Documentation", body: doc.preview.doc.text.clone(), logo: false, kind: Details::Text });
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
        if id == CommandId::PasteTerminalPath && let Some(command) = self.terminal_path_target.take() {
            if let Some(why) = self.session.disabled(id) { return self.dispatch(Command::Notice(why.into())); }
            match command {
                Ok(command) => self.dispatch(command),
                Err(why) => self.dispatch(Command::Notice(why)),
            }
            return;
        }
        if matches!(id, CommandId::ShowTerminal | CommandId::FocusTerminal | CommandId::NewTerminal | CommandId::NextTerminal | CommandId::PreviousTerminal) { self.log_focus = false; }
        if id == CommandId::ToggleActivityLog {
            self.log_focus = !self.session.state().activity_log_shown();
            self.log_scroll = 0;
        }
        // A command acts on the explorer's highlighted row, which only this adapter knows.
        if id.targets_explorer() {
            self.remember_explorer();
        }
        // Toggling flips what is shown, then the setting alone decides.
        if id == CommandId::FileIcons {
            let on = !self.nerd_icons();
            self.icons_override = None;
            return self.dispatch(Command::SetFileIcons(on));
        }
        match self.session.action(id) {
            CatalogAction::Disabled(why) => self.dispatch(Command::Notice(why.into())),
            CatalogAction::Command(command) => {
                if self.preview_command(&command) {
                    return;
                }
                self.dispatch(command);
                if id == CommandId::LocateFile {
                    self.tree_shown = true;
                    self.focus = Focus::Tree;
                    self.tree_at = self.session.state().active_in_tree().and_then(|path| self.session.state().tree_rows().position(|r| r.path == path)).unwrap_or(0);
                    self.tree_seen = usize::MAX; // revealed even if it was already the highlight
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
        if !matches!(self.session.state().view(tab.id()).and_then(|v| v.rendering()), Some(Rendering::Preview(_))) {
            return false;
        }
        let current = self
            .preview_scroll
            .get(&tab.id())
            .filter(|(source, _)| state.view(tab.id()).is_some_and(|v| source.matches(&v)))
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
        if let Some(view) = state.view(tab.id()) { self.preview_scroll.insert(tab.id(), (PreviewSource::new(&view), top)); }
        self.session.dispatch(Command::Scroll(source));
        true
    }
    fn sync_preview_selection(&mut self) {
        let state = self.session.state();
        self.preview_scroll.retain(|id, (source, _)| state.tabs().iter().any(|tab| tab.id() == *id
            && (!tab.loaded() || state.view(*id).is_some_and(|v| source.matches(&v)))));
        if let Some((id, source, _)) = &self.preview_selection {
            let current = state.active().is_some_and(|tab| tab.id() == *id)
                && state.view(*id).is_some_and(|view| source.matches(&view) && matches!(view.rendering(), Some(Rendering::Preview(_))));
            if !current { self.preview_selection = None; self.drag = None; }
        }
    }
    fn preview_command(&mut self, command: &Command) -> bool {
        if !matches!(command, Command::SelectAll | Command::Copy) {
            return false;
        }
        let Some(tab) = self.session.state().active() else {
            return false;
        };
        let (Some(view), Some(Rendering::Preview(preview))) = (self.session.state().view(tab.id()), self.session.state().view(tab.id()).and_then(|v| v.rendering()))
        else {
            return false;
        };
        let (id, source) = (tab.id(), PreviewSource::new(&view));
        if matches!(command, Command::SelectAll) {
            self.preview_selection = Some((
                id,
                source,
                Selection {
                    anchor: 0,
                    head: preview.doc.text.len(),
                },
            ));
        } else {
            let text = self
                .preview_selection.as_ref()
                .filter(|(tab, source, _)| *tab == id && source.matches(&view))
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
            InterfaceAction::CloseWindow => { self.quit = true; return; }
            InterfaceAction::QuitAll => { self.quit_all = true; return; }
            InterfaceAction::Open => Prompt::Open,
            InterfaceAction::PasteTerminalPath => Prompt::TerminalPath,
            InterfaceAction::RenameTerminal(id) => Prompt::RenameTerminal { id },
            InterfaceAction::NewScratch => Prompt::NewScratch,
            InterfaceAction::ScratchFiles => {
                match self.session.scratch_files() {
                    Ok(files) => Prompt::ScratchFiles(files),
                    Err(why) => return self.dispatch(Command::Notice(format!("Scratch files: {why}"))),
                }
            },
            InterfaceAction::SaveAs => match self.session.state().active() {
                Some(tab) => Prompt::SaveAs(tab.id()),
                None => return,
            },
            InterfaceAction::Create(kind) => match self.session.create_target() {
                // The highlighted row is the folder to create in: remember it before asking the core.
                Some(dir) => Prompt::Create { kind, dir },
                None => return self.dispatch(Command::Notice("No folder is open".into())),
            },
            InterfaceAction::PasteExplorerEntry => match self.session.paste_entry_request() {
                Ok(request) => Prompt::PasteEntry(request),
                Err(why) => return self.dispatch(Command::Notice(why.into())),
            },
            InterfaceAction::ExplorerHistory => {
                match self.session.explorer_history_launch(&self.exe(), true) {
                    Ok(launch) => self.foreground = Some(launch),
                    Err(why) => self.dispatch(Command::Notice(why)),
                }
                return;
            }
            InterfaceAction::Relocate(kind) => match self.session.relocate_target() {
                Some(from) => Prompt::Relocate { kind, from },
                None => return self.dispatch(Command::Notice("Highlight a file or folder in the explorer".into())),
            },
            InterfaceAction::ConfirmDelete(removal) => match self.session.relocate_target() {
                Some(from) => Prompt::ConfirmDelete { removal, from },
                None => return self.dispatch(Command::Notice("Highlight a file or folder in the explorer".into())),
            },
            InterfaceAction::OpenFolder => Prompt::OpenFolder,
            InterfaceAction::OpenInEditor | InterfaceAction::OpenWithDefaultApp => {
                let default_app = action == InterfaceAction::OpenWithDefaultApp;
                match self.session.external_command(&lenz_content::open::Host::current(), lenz_content::appearance::editor().as_deref(), default_app) {
                    Ok(command) => self.dispatch(command),
                    Err(why) => self.dispatch(Command::Notice(why.into())),
                }
                return;
            }
            InterfaceAction::NewBranch => Prompt::NewBranch { from: None, checkout: true },
            InterfaceAction::CheckoutRevision => Prompt::Revision,
            InterfaceAction::CompareWithBase => return self.git_diff(&code_core::branches::GitDiff::BaseBranch),
            InterfaceAction::CompareSelected => {
                let exe = self.exe();
                match self.session.compare_selected_launch(&exe, true) {
                    Ok(launch) => { self.dispatch(Command::Notice(code_core::external::COMPARISON_NOTICE.into())); self.foreground = Some(launch); },
                    Err(e) => self.dispatch(Command::Notice(e)),
                }
                return;
            }
            InterfaceAction::ShowLocalChanges => return self.git_diff(&code_core::branches::GitDiff::LocalChanges),
            InterfaceAction::ShowBranchChanges => return self.git_diff(&code_core::branches::GitDiff::BranchChanges(None)),
            InterfaceAction::ResolveConflicts => {
                let exe = self.exe();
                match self.session.merge_launch(&exe, true) {
                    Ok(launch) => self.foreground = Some(launch),
                    Err(e) => return self.dispatch(Command::Notice(e)),
                }
                return;
            }
            InterfaceAction::ShowCommitDiff => {
                let exe = self.exe();
                let launch = self.session.commit_diff_launch(&exe, true);
                self.dispatch(Command::CloseCommit);
                match launch {
                    Ok(launch) => self.foreground = Some(launch),
                    Err(e) => return self.dispatch(Command::Notice(e)),
                }
                return;
            }
            InterfaceAction::Paste => return self.dispatch(Command::Notice("Use terminal Paste (usually Ctrl+Shift+V); bracketed paste inserts one transaction".into())),
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
            InterfaceAction::ConfigureSqlMetadata => Prompt::SqlMetadata,
            InterfaceAction::WorkspaceRoot => Prompt::WorkspaceRoot,
            InterfaceAction::LanguageServer => Prompt::LanguageServer,
            InterfaceAction::EditingPreferences => Prompt::Details { title: "Editing preferences", body: self.session.editing_preferences_details(), logo: false, kind: Details::Text },
            InterfaceAction::SqlDialect => Prompt::Details { title: "SQL dialect", body: self.session.sql_dialect_details(), logo: false, kind: Details::Sql },
            InterfaceAction::Problems => Prompt::Details { title: "Problems", body: self.session.problems_details(), logo: false, kind: Details::Text },
            InterfaceAction::Keymap => Prompt::Keymap,
            InterfaceAction::ServerDetails | InterfaceAction::StatusDetails => {
                let (title, details) = if action == InterfaceAction::ServerDetails {
                    ("Server details", self.session.language_server_details())
                } else {
                    ("Status details", self.session.status_details())
                };
                Prompt::Details { title, body: details, logo: false, kind: Details::Text }
            },
            InterfaceAction::About => Prompt::Details { title: code_core::chrome::ABOUT_TITLE, body: code_core::chrome::about(), logo: true, kind: Details::Text },
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
        self.terminal_path_target = None;
        self.terminal_drag = None;
        if !matches!(kind, Prompt::Details { .. }) {
            self.dispatch(Command::CloseDocumentation);
        }
        let mut input = String::new();
        let mut secondary = String::new();
        let mut error = None;
        match kind {
            Prompt::Find => {
                input = self.session.find_seed();
                secondary = self.session.replace_text().to_owned();
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
            Prompt::SqlMetadata => input = self.session.sql_metadata_source().into(),
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
            Prompt::SaveAs(_) => {
                let (dir, name) = self.session.save_as_start();
                input = dir.join(name.unwrap_or_default()).display().to_string();
            }
            Prompt::RenameTerminal { id, .. } => input = self.session.state().terminal().tabs().iter().find(|tab| tab.id() == id).map_or(String::new(), |tab| tab.label().into()),
            Prompt::PasteEntry(ref request) => input = self.session.paste_entry_start(request),
            Prompt::Relocate { kind, ref from } => input = self.session.relocate_start(kind, from),
            Prompt::ConfirmDelete { removal, ref from } => error = self.session.delete_refusal(removal, from),
            _ => {}
        }
        let selected = matches!(kind, Prompt::RenameTerminal { .. } | Prompt::Search) && !input.is_empty();
        self.modal = Some(Modal {
            kind,
            input,
            secondary,
            secondary_focus: false,
            at: 0,
            top: 0,
            reveal_selection: true,
            error,
            capture: None,
            selected,
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
        if let Chosen::Command(command @ Command::ActivateNavigation { .. }) = chosen {
            return self.dispatch(command);
        }
        self.close_modal();
        self.focus = Focus::Text;
        match chosen {
            Chosen::Command(command) => self.dispatch(command),
            Chosen::Action(id) => self.run(id),
        }
    }
    fn choose_external(&mut self,choice:code_core::external::Choice) {
        if let Some(prompt)=self.session.external_prompt() { self.dispatch(Command::ChooseExternal {ticket:prompt.ticket,choice}); }
    }
    fn choose_reconcile(&mut self,choice:code_core::reconcile::Choice) {
        if let Some(prompt)=self.session.reconcile_prompt() { self.dispatch(Command::ChooseReconcile {ticket:prompt.ticket,choice}); }
    }
    fn choose_replace(&mut self,choice:code_core::save::ReplaceChoice) {
        if let Some(prompt)=self.session.save_as_prompt() { self.dispatch(Command::ChooseReplace {ticket:prompt.ticket,choice}); }
    }
    fn choose_close(&mut self,choice:code_core::close::Choice) {
        if let Some(p)=self.session.close_prompt() { self.dispatch(Command::ChooseClose {ticket:p.ticket,buffer:p.buffer,choice}); }
    }
    fn choose_recovery(&mut self,choice:code_core::recovery::Choice) {
        if let Some(p)=self.session.recovery_prompt() { self.dispatch(Command::ChooseRecoveredDraft {ticket:p.ticket,choice}); }
    }
    fn close_modal(&mut self) {
        self.terminal_path_target = None;
        if let Some(modal) = self.modal.take() {
            match modal.kind {
                Prompt::Themes => self.dispatch(Command::PreviewTheme(None)),
                Prompt::Navigation => self.dispatch(Command::CloseSearchEverywhere),
                Prompt::ServerRuntime => self.dispatch(Command::CloseServerRuntime),
                Prompt::Details { kind: Details::Terminal(ticket), .. } => self.terminal_command(code_core::terminal::Command::Confirm { ticket, accept: false }),
                Prompt::Details { kind: Details::Install, .. } => self.dispatch(Command::CloseServerInstall),
                Prompt::Details { kind: Details::External, .. } => self.choose_external(code_core::external::Choice::Cancel),
                Prompt::Details { kind: Details::Reconcile, .. } => self.choose_reconcile(code_core::reconcile::Choice::KeepDraft),
                Prompt::Details { kind: Details::Replace, .. } => self.choose_replace(code_core::save::ReplaceChoice::Cancel),
                Prompt::Details { kind: Details::Close, .. } => self.choose_close(code_core::close::Choice::Cancel),
                Prompt::Details { kind: Details::Recovered, .. } => self.choose_recovery(code_core::recovery::Choice::Later),
                Prompt::Details { kind: Details::Change, .. } => self.dispatch(Command::CloseChange),
                Prompt::Details { kind: Details::Commit, .. } => self.dispatch(Command::CloseCommit),
                Prompt::Details { kind: Details::Checkout, .. } => self.dispatch(Command::CheckoutChoice(code_core::branches::CheckoutChoice::Cancel)),
                Prompt::Branches | Prompt::BranchMenu { .. } => self.dispatch(Command::CloseBranches),
                Prompt::Details { .. } if self.session.state().documentation().is_some() => {
                    self.dispatch(Command::CloseDocumentation)
                }
                _ => {}
            }
        }
        self.sync_install_prompt();
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
                    // A separator reads as a line; it is passed over and does nothing.
                    label: if r.target == MenuTarget::Separator { "─".repeat(24) } else { r.label },
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
            Prompt::Keymap => self
                .session
                .keymap()
                .rows()
                .into_iter()
                .filter(|r| query.is_empty() || r.label.to_lowercase().contains(&query))
                .map(|r| Choice {
                    label: format!("{} — {}{}", r.label, r.chords, r.note.map_or_else(String::new, |note| format!(" ({note})"))),
                    detail: None,
                    disabled: None,
                    target: ChoiceTarget::Keymap(r.command),
                })
                .collect(),
            Prompt::ScratchFiles(ref files) => files.iter()
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(&query)))
                .cloned()
                .map(|p| simple(p.file_name().unwrap_or_default().to_string_lossy().into_owned(), ChoiceTarget::Path(p))).collect(),
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
            Prompt::Branches => self
                .session
                .state()
                .branch_rows(&modal.input)
                .into_iter()
                .map(|r| simple(code_core::branches::row_text(&r), ChoiceTarget::Branch(r.target)))
                .collect(),
            Prompt::BranchMenu { ref target, .. } => self
                .session
                .branch_actions(target)
                .into_iter()
                .map(|(label, action)| simple(label.into(), ChoiceTarget::BranchAction(target.clone(), action)))
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
            Prompt::Places(_) | Prompt::ServerRuntime | Prompt::Themes | Prompt::Branches | Prompt::BranchMenu { .. }
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
                Prompt::RenameTerminal { .. } => "Rename terminal tab · Enter renames · Esc cancels",
                Prompt::TerminalPath => "Paste path into terminal · Enter pastes · no Enter is sent to the shell",
                Prompt::NewScratch => "New scratch file · name with extension chooses language · Enter creates · Esc cancels",
                Prompt::ScratchFiles(_) => "Personal scratch files · type to filter · Enter opens · Esc cancels",
                Prompt::SaveAs(_) => "Save As · enter the new file's path",
                Prompt::Create { kind, dir } => {
                    let where_ = if dir.is_empty() { "the project root" } else { dir.as_str() };
                    return format!("{} in {where_} · Enter creates · Esc cancels", kind.label());
                }
                Prompt::PasteEntry(request) => return format!("Paste {} in {} · saved contents · edit name · Ctrl+A clears · Enter copies · Esc cancels", request.source().display(), if request.folder().is_empty() { "the project root" } else { request.folder() }),
                Prompt::Relocate { kind, from } => {
                    return format!("{} {from} · type {} · Enter · Esc cancels · references are not changed", kind.label(), kind.asks());
                }
                Prompt::ConfirmDelete { removal, from } => {
                    return format!("{} · Enter: {} · Esc cancels", self.session.delete_prompt(*removal, from), removal.label());
                }
                Prompt::OpenFolder => "Open folder · enter path",
                Prompt::Compare(_) => code_core::external::COMPARISON_PROMPT,
                Prompt::SqlMetadata => self.session.sql_metadata_prompt(),
                Prompt::WorkspaceRoot => "Workspace root · empty restores discovery",
                Prompt::ServerRuntime => {
                    "Server runtime executable / install folder · Enter saves and restarts"
                }
                Prompt::RecentRoots => "Recent folders",
                Prompt::Navigation => "Search Everywhere · Tab scope · ↑↓ select · Enter open",
                Prompt::Search => "Search in Project",
                Prompt::Find => "Find in file · Tab replaces · F3 next · Alt+Enter lists all · Alt+C/W/R options",
                Prompt::Declarations => "Find declaration · Tab: workspace symbols",
                Prompt::Symbols => "Workspace symbols · Tab: declarations",
                Prompt::GoTo => "Go to line · line:column",
                Prompt::Themes => "Theme",
                Prompt::Places(PlaceList::Bookmarks) => "Bookmarks · Delete forgets",
                Prompt::Places(PlaceList::RecentFiles) => "Recent files · Delete forgets",
                Prompt::Places(PlaceList::RecentLocations) => "Recent locations · Delete forgets",
                Prompt::Mnemonic => "Bookmark key · press 0–9 or A–Z",
                Prompt::Keymap if self.modal.as_ref().is_some_and(|m| m.capture.is_some()) => {
                    "Keymap · press the new shortcut (Esc cancels)"
                }
                Prompt::Keymap => {
                    return format!(
                        "Keymap: {} · type to filter · Enter rebinds · Delete restores the preset · Esc closes",
                        self.session.keymap().preset().label()
                    );
                }
                Prompt::Siblings { .. } => "Breadcrumb entries",
                Prompt::Help => "Reader help",
                Prompt::Details { title, .. } => title,
                Prompt::Branches => {
                    let Some(b) = self.session.state().branches() else { return String::new() };
                    return match b.state_text() {
                        Some(state) => format!("{} · {state}", b.heading()),
                        None if b.worktrees_only => format!("{} · type to filter · Enter compares · → more", b.heading()),
                        None if b.revisions_only => format!("{} · type to filter or a revision · Enter compares", b.heading()),
                        None => format!("{} · type to filter · Enter checks out · → more", b.heading()),
                    };
                }
                Prompt::BranchMenu { .. } => "Actions · ← back",
                Prompt::NewBranch { checkout: true, from } => return format!("New branch from {} · [x] check out (Tab) · Enter", from.as_deref().map_or("HEAD", code_core::branches::short_ref)),
                Prompt::NewBranch { checkout: false, from } => return format!("New branch from {} · [ ] check out (Tab) · Enter", from.as_deref().map_or("HEAD", code_core::branches::short_ref)),
                Prompt::Revision => "Checkout tag or revision · Enter",
            }
            .to_owned()
        })
    }
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => {
                // foot with a two-group layout (us,gr) reports a Shift press as Shift but its release as Caps Lock.
                let shift_release_as_lock = key.code == KeyCode::CapsLock && key.kind == KeyEventKind::Release && self.double_shift.held();
                let code = if shift_release_as_lock { KeyCode::Modifier(ModifierKeyCode::LeftShift) } else { key.code };
                if let KeyCode::Modifier(modifier) = code {
                    if matches!(
                        modifier,
                        ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift
                    ) {
                        if key.kind == KeyEventKind::Repeat {
                            self.double_shift.interrupt();
                            return false;
                        }
                        let other = key.modifiers.intersects(
                            KeyModifiers::CONTROL
                                | KeyModifiers::ALT
                                | KeyModifiers::SUPER
                                | KeyModifiers::HYPER
                                | KeyModifiers::META,
                        );
                        // A Shift release may come without its press (foot): only a Shift release counts that way.
                        let tapped = if key.kind == KeyEventKind::Press {
                            self.double_shift.update(true, other, Instant::now())
                        } else {
                            self.double_shift.release(other, Instant::now())
                        };
                        if tapped
                            && self.modal.is_none()
                            && !self.session.state().terminal().focused()
                        {
                            self.open_navigation(Scope::All);
                            return true;
                        }
                    } else {
                        // Another modifier, as another key: a chord with Shift held is not a tap.
                        self.double_shift.key(key.modifiers.contains(KeyModifiers::SHIFT));
                    }
                    return false;
                }
                self.double_shift.key(key.modifiers.contains(KeyModifiers::SHIFT));
                if key.kind == KeyEventKind::Release {
                    return false;
                }
                self.hover = None;
                if self.modal.is_some() {
                    self.modal_key(key);
                } else {
                    if self.terminal_key(key) { return true; }
                    if matches!(key.code, KeyCode::Char('q' | 'Q')) && key.modifiers.contains(KeyModifiers::CONTROL) {
                        if key.modifiers.contains(KeyModifiers::SHIFT) { self.quit_all = true; } else { self.quit = true; }
                    } else { self.key(key); }
                }
            }
            Event::Paste(text) => {
                if self.log_focus && self.modal.is_none() && !self.session.state().terminal().focused() { return false; }
                if self.modal.is_some() {
                    self.double_shift.key(false);
                    self.edit_prompt(&text, false);
                } else if self.session.state().terminal().focused() {
                    self.terminal_command(code_core::terminal::Command::Paste(text));
                } else if self.session.state().active().and_then(|tab| self.session.state().csv(tab.id())).is_some() {
                    self.dispatch(Command::Csv(code_core::csv::Command::Input(text)));
                } else {
                    {
                        self.double_shift.key(false);
                        self.replace_draft_input(&text, EditKind::Paste);
                    }
                }
            }
            // The pointer ends a first tap; with Shift held (a Shift-click or drag) that Shift is not a tap.
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Moved => {
                self.double_shift.key(mouse.modifiers.contains(KeyModifiers::SHIFT));
                let before = self.modal.as_ref().map(|m| m.at);
                self.mouse(mouse);
                return before != self.modal.as_ref().map(|m| m.at);
            }
            Event::Mouse(mouse) => {
                self.double_shift.key(mouse.modifiers.contains(KeyModifiers::SHIFT));
                self.mouse(mouse);
            }
            Event::FocusGained => self.dispatch(Command::Recheck),
            Event::Resize(_, _) => {
                self.tree_click = None;
                self.hover = None;
                self.annotation_drag = None;
                self.layout.invalidate_source();
                self.layout.terminal = Default::default();
                self.terminal_drag = None;
                self.clamp();
            }
            Event::FocusLost => {
                self.terminal_drag = None;
                self.tree_click = None;
                self.double_shift.reset();
                self.hover = None;
                self.drag = None;
                self.annotation_drag = None;
            }
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
                | Prompt::ConfirmDelete { .. }
        ) {
            return;
        }
        let target = if modal.secondary_focus {
            &mut modal.secondary
        } else {
            &mut modal.input
        };
        if std::mem::take(&mut modal.selected) {
            target.clear();
        } else if backspace {
            target.pop();
        }
        if !backspace {
            target.extend(text.chars().filter(|c| !c.is_control()));
        }
        modal.at = 0;
        modal.top = 0;
        modal.reveal_selection = true;
        modal.error = None;
        let (kind, query, replacement, replacing) =
            (modal.kind.clone(), modal.input.clone(), modal.secondary.clone(), modal.secondary_focus);
        match kind {
            Prompt::Navigation => {
                if let Some(search) = self.session.state().navigation() {
                    self.dispatch(Command::SearchEverywhere {
                        scope: search.scope,
                        query,
                    });
                }
            }
            // The replacement field only edits remembered text; Enter in it replaces (see `accept`).
            Prompt::Find if replacing => self.dispatch(Command::SetReplaceText(replacement)),
            Prompt::Find => self.dispatch(Command::Find(query)),
            Prompt::Declarations => self.dispatch(Command::FindDeclarations(query)),
            Prompt::Symbols => self.dispatch(Command::Symbols(query)),
            _ => {}
        }
    }
    // Callers require an open modal; branches that dismiss or replace it return before accessing it again.
    fn modal_key(&mut self, key: KeyEvent) {
        let kind = self.modal.as_ref().unwrap().kind.clone();
        if let Prompt::Details { kind: Details::Terminal(ticket), .. } = kind
            && matches!(key.code, KeyCode::Enter | KeyCode::Esc)
        {
            self.modal = None;
            self.terminal_command(code_core::terminal::Command::Confirm { ticket, accept: key.code == KeyCode::Enter });
            return;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        // The keymap waiting for a key: Escape cancels, any other shortcut becomes the highlighted command's.
        if kind == Prompt::Keymap
            && let Some(command) = self.modal.as_ref().and_then(|m| m.capture)
        {
            if key.code == KeyCode::Esc {
                if let Some(m) = self.modal.as_mut() { m.capture = None; m.error = None; }
                return;
            }
            let Some(press) = key_press(key) else { return };
            let Some(chord) = code_core::keymap::Chord::from_press(&press) else {
                if let Some(m) = self.modal.as_mut() { m.error = Some("A shortcut needs Ctrl, Alt or a function key".into()); }
                return;
            };
            if let Some(m) = self.modal.as_mut() { m.capture = None; m.error = None; }
            let blocked = chord.terminal_blocked();
            self.dispatch(Command::SetKeybinding { command, chord: chord.label() });
            if blocked {
                self.dispatch(Command::Notice(format!("{} is not delivered by most terminals; the command stays in F10", chord.label())));
            }
            return;
        }
        if kind == Prompt::LanguageServer
            && control
            && !alt
            && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.run(CommandId::CopyServerDetails);
            return;
        }
        if let Prompt::Details { kind: Details::Checkout, .. } = &kind {
            // s Smart Checkout, f Force, y confirms the discard (another key than f, so a held f never does); Esc
            // cancels; Enter takes the first safe choice.
            use code_core::branches::CheckoutChoice as C;
            let choices: Vec<C> = self.session.checkout_choices().into_iter().map(|c| c.1).collect();
            let pick = match key.code {
                KeyCode::Esc => Some(C::Cancel),
                KeyCode::Char('s' | 'S') if !control && !alt => Some(C::Smart),
                KeyCode::Char('f' | 'F') if !control && !alt => Some(C::Force),
                KeyCode::Char('y' | 'Y') if !control && !alt => Some(C::ConfirmForce),
                KeyCode::Enter => choices.first().copied().filter(|c| *c != C::ConfirmForce),
                _ => None,
            };
            if let Some(c) = pick.filter(|c| choices.contains(c)) {
                self.dispatch(Command::CheckoutChoice(c));
            }
            return;
        }
        // The Git Branches popup: Right opens the highlighted row's actions; in them Left or Esc goes back.
        if kind == Prompt::Branches && key.code == KeyCode::Right {
            return self.open_branch_menu(self.modal.as_ref().map_or(0, |m| m.at));
        }
        if matches!(kind, Prompt::BranchMenu { .. }) && key.code == KeyCode::Right {
            return self.accept();
        }
        if let Prompt::BranchMenu { query, row, .. } = &kind
            && matches!(key.code, KeyCode::Left | KeyCode::Esc)
        {
            let (query, row) = (query.clone(), *row);
            self.prompt(Prompt::Branches);
            if let Some(m) = self.modal.as_mut() {
                (m.input, m.at) = (query, row);
            }
            return;
        }
        if key.code == KeyCode::Tab
            && let Some(Modal { kind: Prompt::NewBranch { checkout, .. }, .. }) = self.modal.as_mut()
        {
            *checkout = !*checkout;
            return;
        }
        if kind == Prompt::Find && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) && !control && !alt {
            if let Some(m) = self.modal.as_mut() {
                // Tab gives the replacement row the keys; Shift-Tab (BackTab) returns to the query, as in the GUI.
                m.secondary_focus = !(matches!(key.code, KeyCode::BackTab) || key.modifiers.contains(KeyModifiers::SHIFT));
            }
            return;
        }
        // A delete confirmation takes Enter and Escape only: nothing types into it or reaches behind it.
        if matches!(kind, Prompt::ConfirmDelete { .. }) && !matches!(key.code, KeyCode::Enter | KeyCode::Esc) { return; }
        if matches!(kind, Prompt::Details {kind:Details::Replace,..}) {
            let choice=match key.code {
                KeyCode::Char('r'|'R') if !control && !alt => Some(code_core::save::ReplaceChoice::Replace),
                KeyCode::Esc => Some(code_core::save::ReplaceChoice::Cancel),
                _=>None,
            };
            if let Some(choice)=choice { self.choose_replace(choice); return; }
        }
        if matches!(kind, Prompt::Details {kind:Details::Reconcile,..}) {
            let choice=match key.code {
                KeyCode::Char('c'|'C') if !control && !alt => Some(code_core::reconcile::Choice::Compare),
                KeyCode::Char('d'|'D') if !control && !alt => Some(code_core::reconcile::Choice::TakeDisk),
                KeyCode::Char('s'|'S') if !control && !alt => Some(code_core::reconcile::Choice::SaveDraft),
                KeyCode::Esc => Some(code_core::reconcile::Choice::KeepDraft),
                _=>None,
            };
            if let Some(choice)=choice { self.choose_reconcile(choice); return; }
        }
        if matches!(kind, Prompt::Details {kind:Details::External,..}) {
            let choice=match key.code {
                KeyCode::Char('s'|'S') if !control && !alt => Some(code_core::external::Choice::SaveAndOpen),
                KeyCode::Char('o'|'O') if !control && !alt => Some(code_core::external::Choice::OpenSavedVersion),
                KeyCode::Esc => Some(code_core::external::Choice::Cancel),
                _=>None,
            };
            if let Some(choice)=choice { self.choose_external(choice); return; }
        }
        if matches!(kind, Prompt::Details {kind:Details::Recovered,..}) {
            let choice=match key.code {
                KeyCode::Char('r'|'R') if !control && !alt => Some(code_core::recovery::Choice::Restore),
                KeyCode::Char('d'|'D') if !control && !alt => Some(code_core::recovery::Choice::Discard),
                KeyCode::Esc => Some(code_core::recovery::Choice::Later),
                _=>None,
            };
            if let Some(choice)=choice { self.choose_recovery(choice); return; }
        }
        if matches!(kind, Prompt::Details {kind:Details::Install,..}) && !control && !alt {
            match key.code {
                KeyCode::Char('s'|'S') | KeyCode::Enter => return self.run(CommandId::StartServerInstall),
                KeyCode::Char('c'|'C') => return self.run(CommandId::CancelServerInstall),
                KeyCode::Char('d'|'D') => return self.run(CommandId::ServerDetails),
                _ => {}
            }
        }
        if matches!(kind, Prompt::Details {kind:Details::Close,..}) {
            let choice=match key.code {
                KeyCode::Char('s'|'S') if !control && !alt => Some(code_core::close::Choice::Save),
                KeyCode::Char('d'|'D') if !control && !alt => Some(code_core::close::Choice::Discard),
                KeyCode::Esc => Some(code_core::close::Choice::Cancel),
                _=>None,
            };
            if let Some(choice)=choice { self.choose_close(choice); return; }
        }
        if matches!(kind,Prompt::Details {kind:Details::Change,..}) && !control && !alt {
            match key.code { KeyCode::Char('d'|'D') => return self.run(CommandId::ShowChangeDiff),KeyCode::Char('r'|'R') => return self.run(CommandId::RevertChange),_=>{} }
        }
        if matches!(kind, Prompt::Details { kind: Details::Sql, .. }) && !control && !alt && matches!(key.code, KeyCode::Char('m' | 'M')) {
            return self.open_menu(Some(CommandGroup::Sql), 1);
        }
        if let Prompt::Details { kind: Details::Commit, .. } = &kind
            && !alt
        {
            match key.code {
                KeyCode::Char('c' | 'C') => return self.run(CommandId::CopyRevision),
                KeyCode::Char('d' | 'D') if !control => return self.run(CommandId::ShowCommitDiff),
                _ => {}
            }
        }
        if let Prompt::Details { body, .. } = &kind
            && control && !alt && matches!(key.code, KeyCode::Char('c' | 'C'))
        {
            self.dispatch(Command::CopyText(body.clone()));
            return;
        }
        // As in the GUI: Left or Escape goes back from a group to the groups, Right opens a group.
        match (&kind, key.code) {
            (Prompt::Menu(Some(group)), KeyCode::Left | KeyCode::Esc) => {
                let parent = group.menu_parent();
                let at = self.session.menu(parent).iter().position(|row| row.target == MenuTarget::Group(*group)).unwrap_or(0);
                return self.open_menu(parent, at);
            }
            (Prompt::Menu(group), KeyCode::Right) => {
                if self.modal.as_ref().and_then(|m| self.session.menu(*group).get(m.at).map(|row| row.target)).is_some_and(|target| matches!(target, MenuTarget::Group(_))) { self.accept(); }
                return;
            }
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
            } else if kind == Prompt::Find && matches!(key.code, KeyCode::Char('a' | 'A')) {
                self.dispatch(Command::ReplaceFind { all: true });
            } else if kind == Prompt::Find && key.code == KeyCode::Enter {
                // As in the GUI's find box: list every match, the prompt keeping the keys and the text its focus.
                let focus = self.focus;
                self.dispatch(Command::ListMatches);
                self.focus = focus;
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
                m.selected = false;
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
            KeyCode::Delete if kind == Prompt::Keymap => {
                let at = self.modal.as_ref().map(|m| m.at);
                let choices = self.choices();
                let command = at.and_then(|at| choices.get(at)).and_then(|c| match c.target {
                    ChoiceTarget::Keymap(command) => Some(command),
                    _ => None,
                });
                if let Some(command) = command {
                    self.dispatch(Command::ResetKeybinding(command));
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
                    let before = m.at;
                    m.at = stepped(m.at, count, key.code, page.max(1) as usize);
                    // A menu's separator line is passed over in the direction of the move.
                    if let Prompt::Menu(group) = &kind && self.session.menu(*group).get(m.at).is_some_and(|row| row.target == MenuTarget::Separator) {
                        m.at = if m.at < before { m.at.saturating_sub(1) } else { (m.at + 1).min(count.saturating_sub(1)) };
                    }
                    m.reveal_selection = true;
                }
                self.clamp();
                if kind == Prompt::Themes {
                    self.dispatch(Command::PreviewTheme(self.highlighted_theme()));
                }
            }
            KeyCode::Backspace if !control => self.edit_prompt("", true),
            KeyCode::Char('a') if control && matches!(kind, Prompt::PasteEntry(_)) => {
                if let Some(m) = self.modal.as_mut() { m.input.clear(); m.error = None; }
            }
            KeyCode::Char('a') if control && kind == Prompt::Search => {
                let Some(m) = self.modal.as_mut() else { return };
                m.selected = !if m.secondary_focus { &m.secondary } else { &m.input }.is_empty();
            }
            // Ctrl+Backspace reaches a terminal as Ctrl+H, Ctrl+W or a modified Backspace.
            KeyCode::Backspace | KeyCode::Char('w' | 'h') if control && kind == Prompt::Search => {
                let Some(m) = self.modal.as_mut() else { return };
                if !std::mem::take(&mut m.selected) { code_core::search::drop_word(if m.secondary_focus { &mut m.secondary } else { &mut m.input }); }
                else if m.secondary_focus { m.secondary.clear() } else { m.input.clear() }
                self.edit_prompt("", false);
            }
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
    /// The actions of Git Branches row `at` (none for the popup's own commands).
    fn open_branch_menu(&mut self, at: usize) {
        let query = self.modal.as_ref().map(|m| m.input.clone()).unwrap_or_default();
        let Some(r) = self.session.state().branch_rows(&query).into_iter().nth(at) else { return };
        if self.session.branch_actions(&r.target).is_empty() {
            return;
        }
        self.prompt(Prompt::BranchMenu { target: r.target, label: r.label, query, row: at });
    }
    /// Runs a Git Branches row's action.
    fn branch_action(&mut self, target: code_core::branches::BranchTarget, action: code_core::branches::BranchAction) {
        use code_core::branches::{BranchAction, BranchTarget};
        self.dispatch(Command::CloseBranches);
        match (action, target) {
            (BranchAction::Checkout, t) => {
                if let Some(target) = t.checkout() {
                    self.dispatch(Command::Checkout(target));
                }
            }
            (BranchAction::NewBranchFrom, t) => self.prompt(Prompt::NewBranch { from: t.rev(), checkout: true }),
            (BranchAction::Open, BranchTarget::Worktree(path)) => self.dispatch(Command::OpenWorktree(path)),
            (BranchAction::OpenInNewWindow, BranchTarget::Worktree(path)) => {
                self.new_view = true;
                self.new_view_root = Some(path);
            }
            (BranchAction::CompareWithWorkingTree | BranchAction::ChangesSinceBranching, t) => {
                if let Some(diff) = code_core::branches::GitDiff::of(&t, action) {
                    self.git_diff(&diff);
                }
            }
            (BranchAction::Open | BranchAction::OpenInNewWindow, _) => {}
        }
    }
    /// Runs Lenz Diff on a Git comparison in this terminal, or shows why not.
    fn git_diff(&mut self, diff: &code_core::branches::GitDiff) {
        let exe = self.exe();
        match self.session.git_diff_launch(diff, &exe, true) {
            Ok(launch) => self.foreground = Some(launch),
            Err(e) => self.dispatch(Command::Notice(e)),
        }
    }
    /// This program, for the apps installed beside it.
    fn exe(&self) -> PathBuf {
        #[cfg(test)]
        if let Some(exe) = &self.exe {
            return exe.clone();
        }
        std::env::current_exe().and_then(std::fs::canonicalize).unwrap_or_default()
    }
    fn accept(&mut self) {
        let Some(m) = self.modal.as_ref() else { return };
        if matches!(m.kind, Prompt::Commands | Prompt::Menu(_) | Prompt::LanguageServer) && self.choices().get(m.at).is_some_and(|c| c.disabled.is_some()) { return; }
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
                Some(MenuTarget::Back) => {
                    let parent = group.and_then(CommandGroup::menu_parent);
                    let at = self.session.menu(parent).iter().position(|row| matches!(row.target, MenuTarget::Group(g) if Some(g) == group)).unwrap_or(0);
                    self.open_menu(parent, at);
                },
                Some(MenuTarget::Run(id)) => {
                    self.close_modal();
                    self.run(id);
                }
                Some(MenuTarget::AllCommands) => self.open_navigation(Scope::Actions),
                Some(MenuTarget::Separator) | None => {}
            },
            Prompt::Context { .. } => {
                let choice = self.choices().get(at).cloned();
                let terminal_path = self.terminal_path_target.take();
                self.close_modal();
                if let Some(Choice {
                    target: ChoiceTarget::Action(id),
                    ..
                }) = choice
                {
                    if id == CommandId::PasteTerminalPath { self.terminal_path_target = terminal_path; }
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
                let exe = self.exe();
                let launch = match input.trim().is_empty() {
                    true => Err("Enter a file or folder path".to_string()),
                    false => self.session.compare_launch(&left, &self.resolve_path(&input), &exe, true),
                };
                match launch {
                    Ok(launch) => {
                        self.close_modal();
                        self.dispatch(Command::Notice(code_core::external::COMPARISON_NOTICE.into()));
                        self.foreground = Some(launch);
                    }
                    Err(e) => {
                        if let Some(modal) = self.modal.as_mut() {
                            modal.error = Some(e);
                        }
                    }
                }
            }
            Prompt::NewScratch => {
                let name = input.trim().to_owned();
                if let Some(why) = code_core::create::name_refusal(&name) {
                    if let Some(modal) = self.modal.as_mut() { modal.error = Some(why); }
                } else { self.close_modal(); self.dispatch(Command::NewScratch(name)); }
            }
            Prompt::SaveAs(tab) => {
                if input.trim().is_empty() {
                    if let Some(modal) = self.modal.as_mut() { modal.error = Some("Enter the new file's path".into()); }
                } else {
                    let path = self.resolve_path(&input);
                    self.close_modal();
                    self.dispatch(Command::SaveAs { tab, path });
                }
            }
            Prompt::ConfirmDelete { removal, from } => match self.session.delete_refusal(removal, &from) {
                Some(why) => {
                    if let Some(modal) = self.modal.as_mut() { modal.error = Some(why); }
                }
                None => {
                    self.close_modal();
                    self.dispatch(Command::DeleteEntry { removal, from: from.into() });
                }
            },
            Prompt::PasteEntry(request) => match self.session.paste_entry_refusal(&request, input.trim()) {
                Some(why) => { if let Some(modal) = self.modal.as_mut() { modal.error = Some(why); } }
                None => { self.close_modal(); self.dispatch(Command::PasteExplorerEntry { request, name: input.trim().to_string() }); }
            },
            Prompt::Relocate { kind, from } => match self.session.relocate_refusal(kind, &from, &input) {
                Some(why) => {
                    if let Some(modal) = self.modal.as_mut() { modal.error = Some(why); }
                }
                None => {
                    self.close_modal();
                    self.dispatch(Command::RelocateEntry { kind, from: from.into(), input });
                }
            },
            Prompt::Create { kind, dir } => {
                let name = input.trim().to_string();
                match self.session.create_refusal(&dir, &name) {
                    Some(why) => {
                        if let Some(modal) = self.modal.as_mut() { modal.error = Some(why); }
                    }
                    None => {
                        self.close_modal();
                        // A created file opens; the keys go to it, as in the desktop.
                        if kind == code_core::create::Kind::File { self.focus = Focus::Text; }
                        self.dispatch(Command::CreateEntry { kind, dir: dir.into(), name });
                    }
                }
            }
            Prompt::RenameTerminal { id, .. } => {
                if let Some(why) = self.session.terminal_rename_refusal(id, &input) {
                    if let Some(modal) = self.modal.as_mut() { modal.error = Some(why); }
                } else {
                    self.close_modal();
                    self.dispatch(Command::Terminal(code_core::terminal::Command::Rename { id, label: input }));
                }
            }
            Prompt::TerminalPath => {
                if input.trim().is_empty() {
                    if let Some(modal) = &mut self.modal { modal.error = Some("Enter a file or folder path".into()); }
                }
                else {
                    let path = self.resolve_path(&input);
                    self.close_modal();
                    self.terminal_command(code_core::terminal::Command::Paths(vec![path]));
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
            Prompt::SqlMetadata => {
                self.close_modal();
                self.dispatch(Command::ConfigureSqlMetadata(input));
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
            // From the replacement field Enter replaces the current match and keeps the prompt for the next one;
            // from the query field it steps on and closes, as before.
            Prompt::Find if self.modal.as_ref().is_some_and(|m| m.secondary_focus) => {
                self.dispatch(Command::ReplaceFind { all: false });
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
            Prompt::NewBranch { from, checkout } => match self.session.validate_branch_name(&input) {
                Ok(()) => {
                    self.close_modal();
                    self.dispatch(Command::NewBranch { name: input.trim().to_string(), from, checkout });
                }
                Err(e) => if let Some(m) = self.modal.as_mut() { m.error = Some(e) },
            },
            Prompt::Revision => match self.session.validate_revision(&input) {
                Ok(_) => {
                    self.close_modal();
                    self.dispatch(Command::Checkout(lenz_content::branches::Target::Detached(input.trim().to_string())));
                }
                Err(e) => if let Some(m) = self.modal.as_mut() { m.error = Some(e) },
            },
            Prompt::Mnemonic => {}
            // Enter on a keymap row asks for the next key instead of closing the dialog.
            Prompt::Keymap => {
                let command = self.choices().get(at).and_then(|c| match c.target {
                    ChoiceTarget::Keymap(command) => Some(command),
                    _ => None,
                });
                if let Some(modal) = &mut self.modal {
                    modal.capture = command;
                    modal.error = None;
                }
            }
            // Enter on the current branch or worktree does nothing (its other actions are in its menu), as in the GUI.
            Prompt::Branches if self.choices().get(at).is_some_and(|c| matches!(&c.target, ChoiceTarget::Branch(t) if !matches!(t, code_core::branches::BranchTarget::Action(_)) && self.session.branch_enter(t).is_none())) => {}
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
                // Asked while the popup is open: what Enter does depends on it (Compare with Worktree compares).
                let enter = match &choice.target {
                    ChoiceTarget::Branch(t) => self.session.branch_enter(t),
                    _ => None,
                };
                self.close_modal();
                match choice.target {
                    ChoiceTarget::Information | ChoiceTarget::ServerRuntime(_) | ChoiceTarget::Keymap(_) => {}
                    ChoiceTarget::Action(id) => self.run(id),
                    ChoiceTarget::Path(path) if kind == Prompt::RecentRoots => self.open_root(path),
                    ChoiceTarget::Path(path) if matches!(kind, Prompt::ScratchFiles(_)) => self.dispatch(Command::OpenScratch(path)),
                    ChoiceTarget::Path(path) => self.dispatch(Command::Open(path)),
                    ChoiceTarget::Theme(id) => {
                        if let Some(theme) = ThemeSelection::from_id(&id) {
                            self.dispatch(Command::Theme(ThemeCommand::Select(theme)));
                        }
                    }
                    ChoiceTarget::Place(place) => self.dispatch(Command::GoToPlace(place)),
                    ChoiceTarget::Branch(target) => match target {
                        code_core::branches::BranchTarget::Action(id) => self.run(id),
                        t => {
                            if let Some(action) = enter {
                                self.branch_action(t, action);
                            }
                        }
                    },
                    ChoiceTarget::BranchAction(target, action) => self.branch_action(target, action),
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
    #[cfg(test)]
    pub(crate) fn enable_draft_input_for_test(&mut self) { self.session.enable_text_input_for_test(); }

    fn draft_context(&self) -> bool {
        self.session.text_input_enabled() && self.session.vim_insert_allowed() && self.modal.is_none() && self.focus == Focus::Text
            && self.session.state().active().and_then(|t| self.session.state().view(t.id())).is_some_and(|v| !matches!(v.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)))
    }

    fn draft_target(&self) -> Option<(TabId, SnapshotKey, std::ops::Range<usize>)> {
        if !self.draft_context() { return None; }
        let state = self.session.state();
        let view = state.view(state.active()?.id())?;
        if view.edit_disabled().is_some() || matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return None; }
        let range = if let Some(selection) = view.tab().selection() { selection.range() }
            else { let start = view.snapshot().line_range(view.tab().top_line())?.start; start..start };
        Some((view.tab().id(), view.snapshot().key(), range))
    }

    /// Typed text replaces every selection of the focused view; the core makes it one transaction.
    fn replace_draft_input(&mut self, text: &str, kind: EditKind) -> bool {
        if self.draft_target().is_none() {
            if self.draft_context() && self.session.disabled(CommandId::Paste).is_some() { return true; }
            return false;
        }
        self.dispatch(Command::InsertText { text: text.to_owned(), kind });
        true
    }

    fn draft_key(&mut self, key: KeyEvent) -> bool {
        if !self.draft_context() { return false; }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let other = key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META);
        match key.code {
            KeyCode::Home | KeyCode::End if !other => {
                let motion=match (control,key.code) { (true,KeyCode::Home)=>Motion::DocumentStart,(true,_)=>Motion::DocumentEnd,(false,KeyCode::Home)=>Motion::LineStart,_=>Motion::LineEnd };
                self.dispatch(if key.modifiers.contains(KeyModifiers::SHIFT){Command::Extend(motion)}else{Command::MoveCaret(motion)});
                true
            }
            KeyCode::Char('z') if control && !other => {
                if self.draft_target().is_none() { return false; }
                self.run(if key.modifiers.contains(KeyModifiers::SHIFT) { CommandId::Redo } else { CommandId::Undo });
                true
            }
            // Ctrl+Shift+V is Markdown Source / Preview, as in the GUI, not Paste.
            KeyCode::Char('x' | 'v') if control && !other && !(key.code == KeyCode::Char('v') && key.modifiers.contains(KeyModifiers::SHIFT)) => {
                if !self.session.text_input_enabled() { return false; }
                self.run(if key.code == KeyCode::Char('x') { CommandId::Cut } else { CommandId::Paste }); true
            }
            KeyCode::Backspace | KeyCode::Delete if !control && !other => {
                if self.draft_target().is_none() { return false; }
                self.dispatch(Command::Delete { backward: key.code == KeyCode::Backspace }); true
            }
            KeyCode::Enter if !control && !other => {
                let newline = self.session.state().active().and_then(|t| self.session.state().view(t.id())).map_or("\n", |v| v.input_newline());
                self.replace_draft_input(newline, EditKind::Typing)
            }
            // Tab indents what is selected (or inserts the file's step); Shift+Tab, sent as BackTab or Tab+Shift,
            // takes a step back. A completion popup took the key before this.
            KeyCode::Tab | KeyCode::BackTab if !control && !key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SUPER | KeyModifiers::META) => {
                if self.draft_target().is_none() { return false; }
                let outdent = key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                self.run(if outdent { CommandId::OutdentSelection } else { CommandId::IndentSelection });
                true
            }
            KeyCode::Char(character) if !control && !other && !character.is_control() => self.replace_draft_input(&character.to_string(), EditKind::Typing),
            _ => false,
        }
    }

    /// Ctrl+Space (or the terminal's NUL for it) asks for completion; while its popup shows, it takes Up/Down/Page
    /// keys, Tab/Enter (accept) and Esc. Every other key, punctuation included, types as usual.
    fn completion_key(&mut self, key: KeyEvent) -> bool {
        if !self.draft_context() { return false; }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let other = key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT | KeyModifiers::SUPER | KeyModifiers::META);
        if !other && (key.code == KeyCode::Null || (control && key.code == KeyCode::Char(' '))) { self.run(CommandId::Completion); return true; }
        // The quick-fix list takes the same keys while it shows.
        if !control && !other && self.session.state().code_actions().is_some() {
            let command = match key.code {
                KeyCode::Up => Command::MoveCodeAction(-1),
                KeyCode::Down => Command::MoveCodeAction(1),
                KeyCode::PageUp => Command::MoveCodeAction(-(crate::render::COMPLETION_ROWS as isize)),
                KeyCode::PageDown => Command::MoveCodeAction(crate::render::COMPLETION_ROWS as isize),
                KeyCode::Enter | KeyCode::Tab => Command::ApplyCodeAction,
                KeyCode::Esc => Command::CloseCodeActions,
                _ => return false,
            };
            self.dispatch(command);
            return true;
        }
        if !control && !other && key.code == KeyCode::Esc && self.session.state().completion().is_none() && self.session.state().signature_help().is_some() {
            self.dispatch(Command::CloseSignature);
            return true;
        }
        if key.modifiers==KeyModifiers::ALT && self.session.state().completion().is_some() && matches!(key.code,KeyCode::PageUp|KeyCode::PageDown|KeyCode::Home|KeyCode::End) {
            let height=self.layout.completion_doc.height as usize;
            let maximum=self.layout.completion_doc_rows.saturating_sub(height);
            let top=self.layout.completion_doc_top;
            self.layout.completion_doc_top=match key.code {KeyCode::Home=>0,KeyCode::End=>maximum,KeyCode::PageUp=>top.saturating_sub(height),_=>top.saturating_add(height).min(maximum)};
            return true;
        }
        if control || other || self.session.state().completion().is_none() { return false; }
        let command = match key.code {
            KeyCode::Up => Command::MoveCompletion(-1),
            KeyCode::Down => Command::MoveCompletion(1),
            KeyCode::PageUp => Command::MoveCompletion(-(crate::render::COMPLETION_ROWS as isize)),
            KeyCode::PageDown => Command::MoveCompletion(crate::render::COMPLETION_ROWS as isize),
            KeyCode::Enter | KeyCode::Tab => Command::AcceptCompletion,
            KeyCode::Esc => Command::CloseCompletion,
            _ => return false,
        };
        self.dispatch(command);
        true
    }

    fn explorer_search_key(&mut self, key: KeyEvent) -> bool {
        use code_core::session::ExplorerSearchInput as Search;
        if self.focus != Focus::Tree || key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) { return false; }
        let active = !self.session.state().explorer_search().query().is_empty();
        let input = match key.code {
            KeyCode::Esc if active => Search::Clear,
            KeyCode::Backspace if active => Search::Backspace,
            KeyCode::Up if active => Search::Step { forward: false },
            KeyCode::Down if active => Search::Step { forward: true },
            KeyCode::Enter if active => Search::Activate,
            KeyCode::Char(' ') if !active => return false,
            KeyCode::Char(c) if !c.is_control() => Search::Text(c.to_string()),
            _ => return false,
        };
        self.remember_explorer();
        self.dispatch(Command::ExplorerSearch(input));
        true
    }

    fn key(&mut self, key: KeyEvent) {
        if self.log_focus && self.session.state().activity_log_shown() {
            if key_press(key).and_then(|press| self.session.key_command(&press)) == Some(CommandId::FindAction) { self.log_focus = false; return self.key(key); }
            let page = self.layout.activity.height.saturating_sub(1) as usize;
            let maximum = self.session.state().activity_log_rows(self.layout.activity.width.saturating_sub(2) as usize).len().saturating_sub(page);
            match key.code {
                KeyCode::Esc => self.run(CommandId::ToggleActivityLog),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.run(CommandId::CopyActivityLog),
                KeyCode::Up => self.log_scroll = self.log_scroll.saturating_add(1).min(maximum),
                KeyCode::PageUp => self.log_scroll = self.log_scroll.saturating_add(page).min(maximum),
                KeyCode::Down => self.log_scroll = self.log_scroll.saturating_sub(1),
                KeyCode::PageDown => self.log_scroll = self.log_scroll.saturating_sub(page),
                KeyCode::Home => self.log_scroll = maximum,
                KeyCode::End => self.log_scroll = 0,
                KeyCode::Tab | KeyCode::BackTab | KeyCode::F(10) => { self.log_focus = false; return self.key(key); }
                _ => {}
            }
            return;
        }
        if self.modal.is_none() && self.focus == Focus::Text {
            let press = key_press(key).map(|mut press| {
                if let KeyCode::Char(c) = key.code { press.key = c.to_string(); press.shift = false; }
                press
            });
            if let Some(press) = press.filter(|press| self.session.vim_handles(press)) {
                self.dispatch(Command::VimKey(press));
                return;
            }
        }
        if self.focus == Focus::Tree && key.code == KeyCode::Delete && key.modifiers.is_empty() {
            return self.run(CommandId::DeleteExplorerEntry);
        }
        // Alt+Left / Alt+Right in the results panel go through its earlier and later lists, before the keymap's Back.
        if self.focus == Focus::Results && key.modifiers == KeyModifiers::ALT && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            return self.run(if key.code == KeyCode::Left { CommandId::PreviousResults } else { CommandId::NextResults });
        }
        if self.explorer_search_key(key) { return; }
        if self.csv_key(key) { return; }
        if self.completion_key(key) { return; }
        if self.draft_key(key) { return; }
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
        if self.focus == Focus::Tree && ctrl && !alt && !shift {
            let action = match key.code { KeyCode::Char('c') => Some(CommandId::CopyExplorerEntry), KeyCode::Char('v') => Some(CommandId::PasteExplorerEntry), _ => None };
            if let Some(action) = action { return self.run(action); }
        }
        // Semantic commands take their keys from the session's keymap: a preset or an override changes what runs, and
        // an unavailable command says why instead of vanishing. Typing keys are never bound there.
        if let Some(id) = key_press(key).and_then(|press| self.session.key_command(&press)) {
            // A held search key must not reopen its list over and over.
            let opener = matches!(id, CommandId::SearchEverywhere | CommandId::FindFile | CommandId::FindSymbol | CommandId::FindAction | CommandId::RecentFiles);
            if key.kind != KeyEventKind::Repeat || !opener {
                self.run(id);
            }
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
            (KeyCode::Char('v' | 'V'), true, true, _) => Some(CommandId::MarkdownSource),
            (KeyCode::Char('/'), false, _, _) => Some(CommandId::FindInFile),
            (KeyCode::Char('a'), true, _, _) => Some(CommandId::SelectAll),
            (KeyCode::Char('c'), true, _, _) => Some(CommandId::Copy),
            (KeyCode::Char('t'), true, _, _) => Some(CommandId::FindDeclarations),
            (KeyCode::Char('e' | 'E'), true, true, _) => {
                self.tree_shown = true;
                self.focus = Focus::Tree;
                return;
            }
            (KeyCode::Char('b'), true, false, false) => Some(CommandId::ToggleExplorer),
            (KeyCode::Char('`' | '~'), true, _, _) => Some(CommandId::GitBranches),
            (KeyCode::Char('k' | 'K'), true, true, _) => Some(CommandId::PinTab),
            (KeyCode::Char('1'), true, _, _) => {
                self.dispatch(Command::FocusPane(0));
                return;
            }
            (KeyCode::Char('2'), true, _, _) => {
                self.dispatch(Command::FocusPane(1));
                return;
            }
            (KeyCode::Tab, true, true, _) | (KeyCode::BackTab, true, _, _) => {
                Some(CommandId::PreviousTab)
            }
            (KeyCode::Tab, true, false, _) => Some(CommandId::NextTab),
            (KeyCode::F(3), _, true, _) => Some(CommandId::PreviousMatch),
            (KeyCode::F(3), _, false, _) => Some(CommandId::NextMatch),
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
            } else if self.focus == Focus::Results && self.session.state().results().is_some() {
                // A running search is stopped first, keeping what it found; then hidden, not closed: Show / Hide
                // Results brings them back.
                self.dispatch(if self.session.disabled(CommandId::StopSearch).is_none() { Command::StopSearch } else { Command::ToggleResults });
            } else if self.focus == Focus::Tree && !self.session.state().tree_marks().is_empty() {
                // First the explorer's marks, as in the GUI; the next Esc leaves the explorer.
                self.dispatch(Command::ClearTreeMarks);
            } else if self.focus == Focus::Text && self.session.disabled(CommandId::CollapseSelections).is_none() {
                // Several cursors go back to one before Esc means leaving the text.
                self.run(CommandId::CollapseSelections);
            } else {
                self.focus = Focus::Text;
            }
            return;
        }
        // In the results panel Tab turns an outline into the server's symbols and back, and a hierarchy around, as
        // in the desktop; elsewhere it moves the keys between panels.
        if key.code == KeyCode::Tab && !shift && self.focus == Focus::Results && let Some(id) = self.results_tab_command() {
            self.run(id);
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
                // Space marks the entry for Compare Selected (Ctrl+D); Esc forgets the marks.
                if key.code == KeyCode::Char(' ') {
                    let path = self.session.state().tree_rows().nth(self.tree_at).map(|r| r.path.clone());
                    if let Some(path) = path {
                        self.dispatch(Command::MarkTreeRow { path, selected: None });
                    }
                } else if matches!(key.code, KeyCode::Enter | KeyCode::Right | KeyCode::Left) {
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
                if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
                    self.result_activate(key.code == KeyCode::Enter);
                } else if matches!(key.code, KeyCode::Right | KeyCode::Left) {
                    self.result_fold(key.code == KeyCode::Right);
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
                    self.session.state().view(tab.id()).is_some()
                        && !matches!(
                            self.session.state().view(tab.id()).and_then(|v| v.rendering()),
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
    /// What Tab does in the results panel: the outline's other view, or the hierarchy's other direction.
    fn results_tab_command(&self) -> Option<CommandId> {
        use code_core::search::ResultsKind;
        use code_core::semantics::hierarchy::Direction;
        let r = self.session.state().results()?;
        let id = match r.results_kind() {
            ResultsKind::Outline => CommandId::FileSymbols,
            ResultsKind::FileSymbols => CommandId::Outline,
            ResultsKind::Hierarchy => match r.hierarchy_direction().map(Direction::other) {
                Some(Direction::Callers) => CommandId::CallHierarchyCallers,
                Some(Direction::Supertypes) => CommandId::TypeHierarchySupertypes,
                Some(Direction::Subtypes) => CommandId::TypeHierarchySubtypes,
                Some(Direction::Callees) | None => CommandId::CallHierarchyCallees,
            },
            _ => return None,
        };
        self.session.disabled(id).is_none().then_some(id)
    }
    /// Right unfolds (or steps into a call's level), Left folds (or steps out to the group or caller holding the
    /// row), as in the desktop; neither ever does the other.
    fn result_fold(&mut self, unfold: bool) {
        use code_core::search::Fold;
        let Some(r) = self.session.state().results() else { return };
        let Some(row) = r.rows().get(self.results_at).cloned() else { return };
        match row {
            Row::Dir { dir, open, .. } if open != unfold => self.dispatch(Command::ToggleResultGroup(Group::Dir(dir))),
            Row::File { path, open, .. } if open != unfold => self.dispatch(Command::ToggleResultGroup(Group::File(path))),
            // A folded folder or file: Left steps out to the folder holding it.
            Row::Dir { .. } | Row::File { .. } if !unfold => {
                let depth = r.row_depth(self.results_at);
                if let Some(parent) = (0..self.results_at).rev().find(|&p| r.row_depth(p) < depth) { self.results_at = parent; }
            }
            Row::Hit(i) => match r.call(i) {
                Some(call) if unfold && matches!(call.fold, Fold::Closed | Fold::Failed(_)) => self.dispatch(Command::ToggleCall(i)),
                Some(call) if unfold && call.fold == Fold::Open => self.results_at = (self.results_at + 1).min(r.rows().len().saturating_sub(1)),
                Some(call) if !unfold && call.fold == Fold::Open => self.dispatch(Command::ToggleCall(i)),
                Some(call) => if !unfold && let Some(parent) = (0..i).rev().find(|&p| r.call(p).is_some_and(|c| c.depth < call.depth)) {
                    self.results_at = r.row_of_hit(parent).unwrap_or(self.results_at);
                },
                // A match folds the file holding it, then each folder out, and the highlight moves to that row.
                None if !unfold && r.grouped() => if let Some(group) = r.groups_of(i).into_iter().rev().find(|g| !r.is_folded(g)) {
                    self.dispatch(Command::ToggleResultGroup(group.clone()));
                    if let Some(r) = self.session.state().results() {
                        let folded = match group { Group::Dir(dir) => r.rows().iter().position(|x| matches!(x, Row::Dir { dir: d, .. } if *d == dir)), Group::File(path) => r.rows().iter().position(|x| matches!(x, Row::File { path: p, .. } if *p == path)) };
                        self.results_at = folded.unwrap_or(self.results_at);
                    }
                },
                None => {}
            },
            _ => {}
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
                    r.results_kind() == code_core::search::ResultsKind::Hierarchy
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
                    let view = pane.source_view(self)?;
                    let tab = view.tab();
                    let snapshot = view.snapshot();
                    if hit.line >= snapshot.line_count() || view.linkable(hit.line, hit.byte).is_none() {
                        return None;
                    }
                    Some((
                        tab.id(),
                        snapshot.key().version,
                        snapshot.line_range(hit.line)?.start + hit.byte,
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
    /// Whether a cell is on the split panes' boundary: the first pane's last column (the next one is the second
    /// pane's gutter), or the lower pane's tab bar; tabs and pane buttons keep their clicks.
    fn on_split_boundary(&self, x: u16, y: u16) -> bool {
        let Some(second) = self.layout.panes.iter().find(|g| g.pane == 1) else { return false };
        let r = self.layout.reading;
        let (b, at) = (second.tab_bar, ratatui::layout::Position::new(x, y));
        let free = self.layout.panes.iter().all(|g| !g.tabs.iter().any(|(_, t)| t.contains(at)) && !g.controls.iter().any(|(_, c)| c.contains(at)));
        free && match self.session.state().split() {
            Some(code_core::session::Split::Columns) => x + 1 == b.x && y >= r.y && y < r.bottom(),
            Some(code_core::session::Split::Rows) => y == b.y && x >= b.x && x < b.right(),
            None => false,
        }
    }

    fn mouse(&mut self, event: MouseEvent) {
        let tree_click = matches!(event.kind, MouseEventKind::Down(_)).then(|| self.tree_click.take()).flatten();
        let (x, y) = (event.column, event.row);
        let point = ratatui::layout::Position::new(x, y);
        if self.modal.is_some() || matches!(event.kind, MouseEventKind::Down(_)) { self.annotation_drag = None; }
        if let Some((pane, tab, key)) = self.annotation_drag {
            let current = self.session.state().shown(pane).is_some_and(|t| t.id() == tab && t.annotation_cols() != 0)
                && self.session.state().view(tab).is_some_and(|v| v.snapshot().key() == key);
            if !current { self.annotation_drag = None; return; }
            if event.kind == MouseEventKind::Drag(MouseButton::Left) {
                if let Some(g) = self.layout.panes.iter().find(|g| g.pane == pane) {
                    let max = (usize::from(g.ann.saturating_sub(1)) + usize::from(g.text.width.saturating_sub(8))).max(12);
                    let width = usize::from(x.saturating_sub(g.gutter_x).saturating_sub(1)).clamp(12, max);
                    self.dispatch(Command::ResizeAnnotations { width, commit: false });
                }
                return;
            }
            if event.kind == MouseEventKind::Up(MouseButton::Left) {
                self.annotation_drag = None;
                self.dispatch(Command::ResizeAnnotations { width: self.session.state().layout().annotation_cols(), commit: true });
                return;
            }
        }
        // The explorer's right edge drags its width.
        if let Some(width) = self.explorer_drag {
            match event.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.explorer_drag = Some(x.saturating_sub(self.layout.tree.x).saturating_add(1));
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.explorer_drag = None;
                    if u32::from(width) != self.divider_from {
                        self.dispatch(Command::SetLayout(code_core::session::Layout { explorer_columns: Some(u32::from(width)), ..self.session.state().layout() }));
                    }
                    return;
                }
                _ => self.explorer_drag = None,
            }
        }
        // A dialog opened mid-drag ends it, keeping the size from before.
        if self.modal.is_some() { self.explorer_drag = None; self.split_drag = None; self.results_drag = None; }
        // The results heading drags the panel's share of the height.
        if self.results_drag.is_some() {
            let main = self.layout.reading.union(self.layout.results);
            match event.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.results_drag = Some((u32::from(main.bottom().saturating_sub(y)) * 1000 / u32::from(main.height.max(1))).clamp(50, 950));
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    let share = self.results_drag.take();
                    if share != Some(self.divider_from) {
                        self.dispatch(Command::SetLayout(code_core::session::Layout { results_permille: share, ..self.session.state().layout() }));
                    }
                    return;
                }
                _ => self.results_drag = None,
            }
        }
        // The boundary between split panes drags the first pane's share.
        if self.split_drag.is_some() {
            match event.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    let r = self.layout.reading;
                    let (at, len) = if self.session.state().split() == Some(code_core::session::Split::Rows) { (y.saturating_sub(r.y), r.height) } else { (x.saturating_add(1).saturating_sub(r.x), r.width) };
                    self.split_drag = Some((u32::from(at) * 1000 / u32::from(len.max(1))).clamp(100, 900));
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    let share = self.split_drag.take();
                    if share != Some(self.divider_from) {
                        self.dispatch(Command::SetLayout(code_core::session::Layout { split_permille: share, ..self.session.state().layout() }));
                    }
                    return;
                }
                _ => self.split_drag = None,
            }
        }
        if self.modal.is_none() && event.kind == MouseEventKind::Down(MouseButton::Left) && !self.layout.tree.is_empty()
            && x + 1 == self.layout.tree.right() && y >= self.layout.tree.y && y < self.layout.tree.bottom()
        {
            self.explorer_drag = Some(self.layout.tree.width);
            self.divider_from = u32::from(self.layout.tree.width);
            self.drag = None;
            return;
        }
        if self.modal.is_none() && event.kind == MouseEventKind::Down(MouseButton::Left) && y + 1 == self.layout.results.y && self.layout.results.height > 0
            && x >= self.layout.reading.x && x < self.layout.reading.right() && !self.layout.controls.iter().any(|(_, c)| c.contains(point))
        {
            let main = self.layout.reading.union(self.layout.results);
            let now = u32::from(self.layout.results.height) * 1000 / u32::from(main.height.max(1));
            self.results_drag = Some(self.session.state().layout().results_permille.unwrap_or(now));
            self.divider_from = self.results_drag.unwrap_or(now);
            self.focus = Focus::Results;
            self.drag = None;
            return;
        }
        if self.modal.is_none() && event.kind == MouseEventKind::Down(MouseButton::Left) && self.on_split_boundary(x, y) {
            self.split_drag = Some(self.session.state().layout().split_permille.unwrap_or(500));
            self.divider_from = self.split_drag.unwrap_or(500);
            self.drag = None;
            return;
        }
        if self.modal.is_none() && self.terminal_mouse(event) { return; }
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
            if let Some(Modal { kind: Prompt::Details { kind: Details::Terminal(ticket), .. }, .. }) = self.modal.as_ref()
                && event.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(index) = self.layout.details_choices.iter().position(|r| r.contains(point)) {
                let ticket = *ticket;
                self.modal = None;
                self.terminal_command(code_core::terminal::Command::Confirm { ticket, accept: index == 0 });
                return;
            }
            if self.form_text().is_some() && matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) {
                if let Some(buttons) = self.layout.form_buttons {
                    if buttons[1].contains(point) || !self.layout.modal.contains(point) { self.close_modal(); }
                    else if buttons[0].contains(point) { self.accept(); }
                    else if self.layout.form_checkbox.contains(point) && let Some(Modal { kind: Prompt::NewBranch { checkout, .. }, .. }) = &mut self.modal { *checkout = !*checkout; }
                }
                return;
            }
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
                    modal.selected = false;
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
            if matches!(event.kind,MouseEventKind::Down(MouseButton::Left)) && self.modal.as_ref().is_some_and(|m|matches!(m.kind,Prompt::Details {kind:Details::External,..})) {
                if let Some(i)=self.layout.details_choices.iter().position(|b|b.contains(point)) && let Some(choice)=code_core::external::Choice::ALL.get(i) { self.choose_external(*choice); }
                return;
            }
            if matches!(event.kind,MouseEventKind::Down(MouseButton::Left)) && self.modal.as_ref().is_some_and(|m|matches!(m.kind,Prompt::Details {kind:Details::Reconcile,..})) {
                if let Some(i)=self.layout.details_choices.iter().position(|b|b.contains(point)) && let Some(choice)=code_core::reconcile::Choice::ALL.get(i) { self.choose_reconcile(*choice); }
                return;
            }
            if matches!(event.kind,MouseEventKind::Down(MouseButton::Left)) && self.modal.as_ref().is_some_and(|m|matches!(m.kind,Prompt::Details {kind:Details::Replace,..})) {
                if let Some(i)=self.layout.details_choices.iter().position(|b|b.contains(point)) && let Some(choice)=code_core::save::ReplaceChoice::ALL.get(i) { self.choose_replace(*choice); }
                return;
            }
            if matches!(event.kind,MouseEventKind::Down(MouseButton::Left)) && self.modal.as_ref().is_some_and(|m|matches!(m.kind,Prompt::Details {kind:Details::Close,..})) {
                if let Some(i)=self.layout.details_choices.iter().position(|b|b.contains(point)) && let Some(choice)=[code_core::close::Choice::Save,code_core::close::Choice::Discard,code_core::close::Choice::Cancel].get(i) { self.choose_close(*choice); }
                return;
            }
            if matches!(event.kind,MouseEventKind::Down(MouseButton::Left)) && self.modal.as_ref().is_some_and(|m|matches!(m.kind,Prompt::Details {kind:Details::Recovered,..})) {
                if let Some(i)=self.layout.details_choices.iter().position(|b|b.contains(point)) && let Some(choice)=code_core::recovery::Choice::ALL.get(i) { self.choose_recovery(*choice); }
                return;
            }
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(Modal { kind: Prompt::Details { kind: Details::Checkout, .. }, .. }) = &self.modal
            {
                if let Some(i) = self.layout.details_choices.iter().position(|b| b.contains(point))
                    && let Some(&(_, c)) = self.session.checkout_choices().get(i)
                {
                    self.dispatch(Command::CheckoutChoice(c));
                }
                return;
            }
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(Modal { kind: Prompt::Details { body, kind, .. }, .. }) = &self.modal
            {
                if *kind == Details::Install {
                    if let Some(i) = self.layout.details_choices.iter().position(|rect| rect.contains(point)) {
                        if let Some(id) = self.session.server_install_actions().get(i).copied() { return self.run(id); }
                        return self.close_modal();
                    }
                    return;
                }
                if *kind == Details::Change {
                    if self.layout.details_diff.contains(point) { return self.run(CommandId::ShowChangeDiff); }
                    if self.layout.details_copy.contains(point) { return self.run(CommandId::RevertChange); }
                }
                if *kind == Details::Sql && self.layout.details_diff.contains(point) {
                    return self.open_menu(Some(CommandGroup::Sql), 1);
                }
                if *kind == Details::Commit {
                    if self.layout.details_diff.contains(point) {
                        return self.run(CommandId::ShowCommitDiff);
                    }
                    if self.layout.details_copy.contains(point) {
                        return self.run(CommandId::CopyRevision);
                    }
                }
                if self.layout.details_copy.contains(point) {
                    self.dispatch(Command::CopyText(body.clone()));
                    return;
                }
                if self.layout.details_close.contains(point) {
                    self.close_modal();
                    return;
                }
            }
            // A Git Branches row: a right click shows its actions.
            if matches!(event.kind, MouseEventKind::Down(MouseButton::Right))
                && self.modal.as_ref().is_some_and(|m| m.kind == Prompt::Branches)
                && self.layout.modal_rows.contains(point)
            {
                let screen_row = y.saturating_sub(self.layout.modal_rows.y) as usize;
                if let Some(&row) = self.layout.modal_items.get(screen_row) {
                    self.open_branch_menu(row);
                }
                return;
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
        if matches!(event.kind, MouseEventKind::Down(_)) && self.session.state().terminal().focused() {
            self.terminal_command(code_core::terminal::Command::Focus(false));
        }
        if self.csv_mouse(event) { return; }
        // A click on a quick fix applies it.
        if event.kind == MouseEventKind::Down(MouseButton::Left) && self.layout.code_actions.contains(point) && self.session.state().code_actions().is_some() {
            self.dispatch(Command::SelectCodeAction(self.layout.code_actions_first + usize::from(y - self.layout.code_actions.y)));
            self.dispatch(Command::ApplyCodeAction);
            return;
        }
        if self.layout.activity.contains(point) && self.session.state().activity_log_shown() {
            let maximum = self.session.state().activity_log_rows(self.layout.activity.width.saturating_sub(2) as usize).len().saturating_sub(self.layout.activity.height.saturating_sub(1) as usize);
            match event.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.log_focus = true;
                    self.terminal_command(code_core::terminal::Command::Focus(false));
                    if let Some((id, _)) = self.layout.controls.iter().find(|(_, rect)| rect.contains(point)) { self.run(*id); }
                }
                MouseEventKind::ScrollUp => self.log_scroll = self.log_scroll.saturating_add(3).min(maximum),
                MouseEventKind::ScrollDown => self.log_scroll = self.log_scroll.saturating_sub(3),
                _ => {}
            }
            return;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            self.log_focus = false;
            if self.session.state().terminal().focused() { self.terminal_command(code_core::terminal::Command::Focus(false)); }
        }
        if self.session.state().completion().is_some() && self.layout.completion_doc.contains(point) {
            let maximum=self.layout.completion_doc_rows.saturating_sub(self.layout.completion_doc.height as usize);
            match event.kind {
                MouseEventKind::ScrollDown => self.layout.completion_doc_top=self.layout.completion_doc_top.saturating_add(3).min(maximum),
                MouseEventKind::ScrollUp => self.layout.completion_doc_top=self.layout.completion_doc_top.saturating_sub(3).min(maximum),
                _ => {}
            }
            return;
        }
        // A click on a suggestion accepts it.
        if event.kind == MouseEventKind::Down(MouseButton::Left) && self.layout.completion.contains(point) && self.session.state().completion().is_some() {
            self.dispatch(Command::SelectCompletion(self.layout.completion_first + usize::from(y - self.layout.completion.y)));
            self.dispatch(Command::AcceptCompletion);
            return;
        }
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(g) = self.layout.panes.iter().find(|g| g.ann > 0 && y >= g.text.y && y < g.text.bottom() && x.abs_diff(g.gutter_x + g.ann) <= 1)
            && let Some(view) = g.source_view(self)
        {
            self.annotation_drag = Some((g.pane, view.tab().id(), view.snapshot().key()));
            self.drag = None;
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
                    let path = self.session.state().tree_rows().nth(row).map(|r| r.path.clone());
                    self.focus = Focus::Tree;
                    // Ctrl+click marks it for Compare Selected (with the highlighted entry); a plain click forgets.
                    if event.modifiers.contains(KeyModifiers::CONTROL) && let Some(path) = path {
                        let selected = self.session.state().tree_rows().nth(self.tree_at).map(|r| r.path.clone());
                        self.dispatch(Command::MarkTreeRow { path, selected });
                        self.tree_at = row;
                        return;
                    }
                    self.dispatch(Command::ClearTreeMarks);
                    self.tree_at = row;
                    self.remember_explorer();
                    let Some(entry) = self.session.state().tree_rows().nth(row) else { return };
                    let arrow = self.layout.tree.x + 1 + (entry.depth.min(20) * 2) as u16;
                    let path = self.session.state().project().map(|p| p.root().join(&*entry.path));
                    let now = Instant::now();
                    let double = tree_click.is_some_and(|(when, prior)| path.as_ref() == Some(&prior) && now.duration_since(when) < Duration::from_millis(400));
                    if entry.dir.is_none() || x == arrow || double {
                        self.tree_activate(KeyCode::Enter);
                        // As in the desktop, a clicked file opens and the explorer keeps the keys (Enter moves them).
                        self.focus = Focus::Tree;
                    } else if let Some(path) = path { self.tree_click = Some((now, path)); }
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
                if pane.source_view(self).is_none() {
                    continue;
                }
                // A click on an annotation opens its commit.
                if !drag
                    && (pane.gutter_x..pane.gutter_x + pane.ann).contains(&x)
                    && let Some(&(_, line, 0)) = pane.rows.iter().find(|(row, _, _)| *row == y)
                    && let Some(tab) = self.session.state().shown(pane.pane).map(|t| t.id())
                {
                    let pane = pane.pane;
                    self.dispatch(Command::FocusPane(pane));
                    self.dispatch(Command::ShowCommitAt { tab, line });
                    self.focus = Focus::Text;
                    return;
                }
                if !drag && (pane.gutter_x+pane.ann..pane.gutter_x+pane.ann+2).contains(&x)
                    && let Some(&(_,line,0))=pane.rows.iter().find(|(row,_,_)|*row==y)
                    && let Some(tab)=self.session.state().shown(pane.pane).map(|t|t.id())
                    && self.session.state().markers(tab).is_some_and(|m|m.change(line).is_some() || m.deleted_above(line)>0)
                { let pane=pane.pane; self.dispatch(Command::FocusPane(pane));self.dispatch(Command::InspectChangeAt {tab,line}); self.focus=Focus::Text;return; }
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
                        let (Some(view), Some(Rendering::Preview(preview))) =
                            (self.session.state().view(tab.id()), self.session.state().view(tab.id()).and_then(|v| v.rendering()))
                        else {
                            return;
                        };
                        let (id, version, len) = (tab.id(), view.snapshot().key().version, preview.doc.text.len());
                        let point = code_core::session::PreviewPoint { tab: id, version: preview.version, rendered: byte.min(len) };
                        let source = PreviewSource::new(&view);
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
                        // The core learns the position (Show in Source, Follow Link); Ctrl+click goes there at once.
                        if !drag && event.modifiers.contains(KeyModifiers::CONTROL) {
                            self.drag = None;
                            self.focus = Focus::Text;
                            self.dispatch(Command::ActivatePreview(point));
                            return;
                        }
                        if !drag { self.dispatch(Command::PointPreview(point)); }
                        self.preview_selection = Some((id, source, Selection { anchor, head }));
                        self.drag = Some((pane_id, id, version, anchor));
                        self.focus = Focus::Text;
                        return;
                    }
                    let Some(view) = pane.source_view(self) else {
                        return;
                    };
                    let (id, version) = (view.tab().id(), view.snapshot().key().version);
                    if drag
                        && self
                            .drag
                            .is_none_or(|(p, t, v, _)| p != pane_id || t != id || v != version)
                    {
                        self.drag = None;
                        return;
                    }
                    let Some(source_line) = view.snapshot().line_range(line) else { return };
                    let offset = (source_line.start + byte).min(view.snapshot().len_bytes());
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
            if let Some(pane) = self.layout.panes.iter().find(|p| p.tab_bar.contains(point)).map(|p| p.pane) {
                let mut strips = self.tab_strips.get();
                if let Some(strip) = strips.get_mut(usize::from(pane)) {
                    let first = if down { strip.first.saturating_add(1).min(strip.last) } else { strip.first.saturating_sub(1) };
                    strip.manual |= first != strip.first;
                    strip.first = first;
                }
                self.tab_strips.set(strips);
            } else if self.layout.tree.contains(point) {
                // The wheel moves the view; the highlighted row stays where it is, as in the desktop.
                self.tree_top = if down { self.tree_top.saturating_add(3) } else { self.tree_top.saturating_sub(3) };
            } else if self.layout.results.contains(point) {
                self.results_top = if down { self.results_top.saturating_add(3) } else { self.results_top.saturating_sub(3) };
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
        if let Some((pane, tab)) = self.layout.panes.iter().find_map(|p| p.tabs.iter().find(|(_, r)| r.contains((x, y).into())).map(|(id, _)| (p.pane, *id))) {
            self.dispatch(Command::FocusPane(pane));
            self.dispatch(Command::ActivateTab(tab));
            self.focus = Focus::Text;
            let mut entries = self.session.tab_menu();
            entries.push(MenuEntry::Command(CommandId::PasteTerminalPath));
            self.prompt(Prompt::Context { x, y, entries });
            self.terminal_path_target = Some(self.session.terminal_path_command(None));
            return;
        }
        // The explorer's own menu (creation), on a row or the empty space below the rows.
        if self.session.state().has_tree() && self.layout.tree.contains((x, y).into()) {
            let row = self.tree_top + y.saturating_sub(self.layout.tree.y) as usize;
            self.focus = Focus::Tree;
            if row < self.session.state().tree_rows().count() {
                self.tree_at = row;
            }
            self.remember_explorer();
            let target = self.session.state().tree_rows().nth(self.tree_at).map(|r| r.path.to_string());
            let entries = self.session.tree_menu();
            self.prompt(Prompt::Context { x, y, entries });
            self.terminal_path_target = target.map(|path| self.session.terminal_path_command(Some(&path)));
            return;
        }
        let Some(pane) = self
            .layout
            .panes
            .iter()
            .find(|p| p.text.contains((x, y).into()) || ((p.gutter_x..p.text.x).contains(&x) && (p.text.y..p.text.bottom()).contains(&y)))
        else {
            return;
        };
        let pane_id = pane.pane;
        let preview = pane.preview;
        let Some(view) = pane.source_view(self) else { return };
        let offset = (!preview)
            .then(|| {
                let line = pane.rows.iter().find(|(row, _, _)| *row == y)?.1;
                // In the gutter (numbers, folds, blame): the line's start.
                let byte = pane
                    .hits
                    .iter()
                    .filter(|h| h.y == y && h.line == line && h.x <= x && x >= pane.text.x)
                    .max_by_key(|h| h.x)
                    .map_or(0, |h| h.byte);
                Some((view.snapshot().line_range(line)?.start + byte).min(view.snapshot().len_bytes()))
            })
            .flatten();
        if self.session.state().shown(pane_id).is_none() {
            return;
        }
        // In a preview, the menu's Show in Source and Follow Link act where it was opened.
        let point = preview.then(|| {
            let line = pane.rows.iter().find(|(row, _, _)| *row == y)?.1;
            let byte = pane.hits.iter().filter(|h| h.y == y && h.line == line && h.x <= x).max_by_key(|h| h.x)?.byte;
            let tab = self.session.state().shown(pane_id)?;
            let Some(Rendering::Preview(p)) = self.session.state().view(tab.id())?.rendering() else { return None };
            Some(code_core::session::PreviewPoint { tab: tab.id(), version: p.version, rendered: byte.min(p.doc.text.len()) })
        }).flatten();
        let gutter = x < pane.text.x;
        self.dispatch(Command::FocusPane(pane_id));
        if let Some(point) = point { self.dispatch(Command::PointPreview(point)); }
        // In the text, a click inside the selection keeps it; in the gutter the caret goes to that line.
        if let Some(at) = offset
            && (gutter || !self.session.state().selection().is_some_and(|s| s.range().contains(&at)))
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
    /// Follow the core's decision prompts. Any core dispatch can open or settle one (each finishes by accounting
    /// completed saves), a draw-time layout command included, so drawing calls this after its own dispatches.
    fn sync_change_inspector(&mut self) {
        match (self.session.inspected_change_text(),self.modal.as_mut()) {
            (Some(text),Some(Modal{kind:Prompt::Details{kind:Details::Change,body,..},..}))=>*body=text,
            (None,Some(Modal{kind:Prompt::Details{kind:Details::Change,..},..}))=>self.modal=None,
            (Some(body),None)=>self.prompt(Prompt::Details{title:"Marked change",body,logo:false,kind:Details::Change}),
            _=>{}
        }
    }
    fn sync_install_prompt(&mut self) {
        let install = self.session.state().server_install().map(|m| m.text());
        match (install, &mut self.modal) {
            (Some(text),Some(Modal {kind:Prompt::Details {kind:Details::Install,body,..},..})) => *body=text,
            (Some(body),None) => self.prompt(Prompt::Details {title:"Install language server",body,logo:false,kind:Details::Install}),
            (None,Some(Modal {kind:Prompt::Details {kind:Details::Install,..},..})) => self.modal=None,
            _=>{}
        }
    }
    pub(crate) fn sync_prompts(&mut self) {
        self.sync_terminal_prompt();
        self.sync_change_inspector();
        self.sync_install_prompt();
        let external=self.session.external_prompt().map(|prompt|prompt.text());
        match (external,&mut self.modal) {
            (Some(text),Some(Modal {kind:Prompt::Details {kind:Details::External,body,..},..})) => *body=text,
            (Some(body),None) => self.prompt(Prompt::Details {title:"Open file externally",body,logo:false,kind:Details::External}),
            (None,Some(Modal {kind:Prompt::Details {kind:Details::External,..},..})) => self.modal=None,
            _=>{}
        }
        let replace=self.session.save_as_prompt().map(|prompt|prompt.text());
        match (replace,&mut self.modal) {
            (Some(text),Some(Modal {kind:Prompt::Details {kind:Details::Replace,body,..},..})) => *body=text,
            (Some(body),None) => self.prompt(Prompt::Details {title:"Replace existing file?",body,logo:false,kind:Details::Replace}),
            (None,Some(Modal {kind:Prompt::Details {kind:Details::Replace,..},..})) => self.modal=None,
            _=>{}
        }
        let recovered=self.session.recovery_prompt().map(|p|p.text());
        match (recovered,&mut self.modal) {
            (Some(text),Some(Modal {kind:Prompt::Details {kind:Details::Recovered,body,..},..})) => *body=text,
            (Some(body),None) => self.prompt(Prompt::Details {title:"Recovered unsaved work",body,logo:false,kind:Details::Recovered}),
            (None,Some(Modal {kind:Prompt::Details {kind:Details::Recovered,..},..})) => self.modal=None,
            _=>{}
        }
        let reconcile=self.session.reconcile_prompt().map(|p|p.text());
        match (reconcile,&mut self.modal) {
            (Some(text),Some(Modal {kind:Prompt::Details {kind:Details::Reconcile,body,..},..})) => *body=text,
            (Some(body),None) => self.prompt(Prompt::Details {title:"File changed on disk",body,logo:false,kind:Details::Reconcile}),
            (None,Some(Modal {kind:Prompt::Details {kind:Details::Reconcile,..},..})) => self.modal=None,
            _=>{}
        }
        let close=self.session.close_prompt().map(|p|p.text());
        match (close,&mut self.modal) {
            (Some(text),Some(Modal {kind:Prompt::Details {kind:Details::Close,body,..},..})) => *body=text,
            (Some(body),None) => self.prompt(Prompt::Details {title:"Unsaved changes",body,logo:false,kind:Details::Close}),
            (None,Some(Modal {kind:Prompt::Details {kind:Details::Close,..},..})) => self.modal=None,
            _=>{}
        }
    }
    pub(crate) fn clamp(&mut self) {
        self.sync_prompts();

        if self.modal.as_ref().is_some_and(|modal| modal.kind == Prompt::Navigation) && self.session.state().navigation().is_none() {
            self.modal = None;
            self.focus = Focus::Text;
        }
        // A moved highlight comes into view; a wheel-scrolled view keeps it where it is, even off screen. Both lists
        // lose their title row, as drawn.
        let tree = self.session.state().tree_rows().count();
        let reveal = self.tree_at != self.tree_seen;
        clamp_list(&mut self.tree_at, &mut self.tree_top, tree, self.layout.tree.height.saturating_sub(1) as usize, reveal);
        self.tree_seen = self.tree_at;
        let results = self.session.state().results().map_or(0, |r| r.rows().len());
        let reveal = self.results_at != self.results_seen;
        clamp_list(&mut self.results_at, &mut self.results_top, results, self.layout.results.height.saturating_sub(1) as usize, reveal);
        self.results_seen = self.results_at;
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
/// A terminal key as the session keymap names it: its key and the modifiers that ran the binding. Terminals report
/// shifted symbols for Shift+- = `_`, Shift+\\ = `|`, Shift+. = `>`; the keymap normalizes those itself.
pub(crate) fn key_press(key: KeyEvent) -> Option<code_core::keymap::KeyPress> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab;
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let name = match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => "tab".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "escape".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Null => "null".into(),
        _ => return None,
    };
    code_core::keymap::KeyPress::new(&name, ctrl, shift, alt)
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
/// Keep `at` within the list and the view within its rows; `reveal` also scrolls the view to show `at`.
fn clamp_list(at: &mut usize, top: &mut usize, len: usize, rows: usize, reveal: bool) {
    *at = (*at).min(len.saturating_sub(1));
    *top = (*top).min(len.saturating_sub(rows.max(1)));
    if !reveal {
        return;
    }
    if *at < *top {
        *top = *at;
    }
    if *at >= top.saturating_add(rows.max(1)) {
        *top = at.saturating_add(1).saturating_sub(rows.max(1));
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
pub(crate) mod tests;

impl Drop for App {
    fn drop(&mut self) {
        self.remember_explorer();
    }
}
