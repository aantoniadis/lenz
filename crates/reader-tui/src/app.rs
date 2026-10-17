//! Terminal input translates to the shared reader session; only focus and cell geometry live here.
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use lenz_content::{
    goto::parse_line_column,
    theme::{ThemeCommand, ThemeSelection},
};
use reader_core::{
    actions::{CatalogAction, InterfaceAction},
    breadcrumbs::SiblingTarget,
    search::{Group, Row},
    session::{
        Command, CommandId, Completion, Config, Motion, Place, PlaceList, Rendering, SearchIn,
        Selection, Session,
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
    Open,
    OpenFolder,
    WorkspaceRoot,
    RecentRoots,
    Finder,
    Search,
    Find,
    Declarations,
    Symbols,
    GoTo,
    Themes,
    Places(PlaceList),
    Mnemonic,
    Siblings { pane: u8, crumb: usize },
    Help,
    Details(String),
}
pub(crate) struct Modal {
    pub kind: Prompt,
    pub input: String,
    pub secondary: String,
    pub secondary_focus: bool,
    pub at: usize,
    pub top: usize,
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
    pub session: Session,
    pub modal: Option<Modal>,
    pub focus: Focus,
    pub tree_at: usize,
    pub tree_top: usize,
    pub results_at: usize,
    pub results_top: usize,
    pub tree_shown: bool,
    pub notice: Option<String>,
    pub quit: bool,
    pub new_view: bool,
    pub copy: Option<String>,
    pub layout: crate::render::Geometry,
    pub crumb_at: usize,
    drag: Option<(u8, TabId, u64, usize)>,
    hover: Option<Hover>,
    pub(crate) preview_selection: Option<(TabId, u64, Selection)>,
    pub(crate) preview_scroll: HashMap<TabId, (u64, usize)>,
}
impl App {
    pub fn new(config: Config, path: Option<PathBuf>) -> Self {
        let mut app = Self {
            session: Session::new(config),
            modal: None,
            focus: Focus::Text,
            tree_at: 0,
            tree_top: 0,
            results_at: 0,
            results_top: 0,
            tree_shown: true,
            notice: None,
            quit: false,
            new_view: false,
            copy: None,
            layout: Default::default(),
            crumb_at: 0,
            drag: None,
            hover: None,
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
        let effect = self.session.dispatch(command);
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
        let tree_path = self
            .session
            .state()
            .project()
            .and_then(|p| p.rows().get(self.tree_at))
            .map(|r| r.path.clone());
        let results_id = self.session.state().results().map(|r| r.id());
        let preview_positions = self.preview_positions();
        let effect = self.session.dispatch(completion);
        if let Some(path) = tree_path
            && let Some(project) = self.session.state().project()
            && project
                .rows()
                .get(self.tree_at)
                .is_none_or(|r| r.path != path)
        {
            self.tree_at = project
                .rows()
                .iter()
                .position(|r| r.path == path)
                .unwrap_or(0);
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
            self.prompt(Prompt::Details(doc.preview.doc.text.clone()));
        }
    }
    fn open_root(&mut self, path: PathBuf) {
        self.tree_at = 0;
        self.tree_top = 0;
        self.results_at = 0;
        self.results_top = 0;
        self.dispatch(Command::OpenRoot(path));
    }
    pub fn run(&mut self, id: CommandId) {
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
                    self.tree_at = self
                        .session
                        .state()
                        .project()
                        .and_then(|p| {
                            let path = self.session.state().active_in_tree()?;
                            p.rows().iter().position(|r| r.path == path)
                        })
                        .unwrap_or(0);
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
    fn interface(&mut self, action: InterfaceAction) {
        let kind = match action {
            InterfaceAction::Open => Prompt::Open,
            InterfaceAction::OpenFolder => Prompt::OpenFolder,
            InterfaceAction::OpenRecent => Prompt::RecentRoots,
            InterfaceAction::FindFile => {
                self.dispatch(Command::FindFile(String::new()));
                Prompt::Finder
            }
            InterfaceAction::SearchProject => Prompt::Search,
            InterfaceAction::FindInFile => Prompt::Find,
            InterfaceAction::GoToLine => Prompt::GoTo,
            InterfaceAction::FindDeclarations => Prompt::Declarations,
            InterfaceAction::Symbols => Prompt::Symbols,
            InterfaceAction::SetBookmarkMnemonic => Prompt::Mnemonic,
            InterfaceAction::ShowBookmarks => Prompt::Places(PlaceList::Bookmarks),
            InterfaceAction::RecentFiles => Prompt::Places(PlaceList::RecentFiles),
            InterfaceAction::RecentLocations => Prompt::Places(PlaceList::RecentLocations),
            InterfaceAction::ChooseTheme => {
                self.dispatch(Command::Theme(ThemeCommand::OpenChooser));
                Prompt::Themes
            }
            InterfaceAction::WorkspaceRoot => Prompt::WorkspaceRoot,
            InterfaceAction::NewWindow => {
                self.new_view = true;
                return;
            }
            InterfaceAction::ToggleExplorer => {
                self.tree_shown = !self.tree_shown;
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
            error: None,
        });
        self.drag = None;
        self.hover = None;
    }
    fn close_modal(&mut self) {
        if let Some(modal) = self.modal.take() {
            match modal.kind {
                Prompt::Finder => self.dispatch(Command::CloseFinder),
                Prompt::Details(_) if self.session.state().documentation().is_some() => {
                    self.dispatch(Command::CloseDocumentation)
                }
                _ => {}
            }
        }
    }
    pub fn choices(&self) -> Vec<Choice> {
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
            Prompt::Commands => self
                .session
                .actions()
                .into_iter()
                .map(|a| Choice {
                    label: format!(
                        "{}{}",
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
            Prompt::Finder => self.session.state().project().map_or_else(Vec::new, |p| {
                p.finder().map_or_else(Vec::new, |f| {
                    f.hits
                        .iter()
                        .map(|h| simple(h.to_string(), ChoiceTarget::Path(p.root().join(&**h))))
                        .collect()
                })
            }),
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
        if matches!(modal.kind, Prompt::Finder | Prompt::Places(_)) || query.is_empty() {
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
                Prompt::Commands => "Commands · type to filter",
                Prompt::Open => "Open file · enter path",
                Prompt::OpenFolder => "Open folder · enter path",
                Prompt::WorkspaceRoot => "Workspace root · empty restores discovery",
                Prompt::RecentRoots => "Recent folders",
                Prompt::Finder => "Find file · name:line:column",
                Prompt::Search => "Search project · Tab switches file pattern",
                Prompt::Find => "Find in file · F3 next · Alt+C/W/R options",
                Prompt::Declarations => "Find declaration · Tab: workspace symbols",
                Prompt::Symbols => "Workspace symbols · Tab: declarations",
                Prompt::GoTo => "Go to line · line:column",
                Prompt::Themes => "Choose theme",
                Prompt::Places(PlaceList::Bookmarks) => "Bookmarks · Delete forgets",
                Prompt::Places(PlaceList::RecentFiles) => "Recent files · Delete forgets",
                Prompt::Places(PlaceList::RecentLocations) => "Recent locations · Delete forgets",
                Prompt::Mnemonic => "Bookmark key · press 0–9 or A–Z",
                Prompt::Siblings { .. } => "Breadcrumb entries",
                Prompt::Help => "Reader help",
                Prompt::Details(_) => "Details",
            }
            .to_owned()
        })
    }
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
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
                self.edit_prompt(&text, false);
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Moved => {
                self.mouse(mouse);
                return false;
            }
            Event::Mouse(mouse) => self.mouse(mouse),
            Event::FocusGained => self.dispatch(Command::Recheck),
            Event::Resize(_, _) => {
                self.hover = None;
                self.clamp();
            }
            Event::FocusLost => {
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
            Prompt::Help | Prompt::Details(_) | Prompt::Mnemonic
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
        modal.error = None;
        let (kind, query) = (modal.kind.clone(), modal.input.clone());
        match kind {
            Prompt::Finder => self.dispatch(Command::FindFile(query)),
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
                let count = if matches!(kind, Prompt::Help | Prompt::Details(_)) {
                    self.layout.modal_content_rows
                } else {
                    self.choices().len()
                };
                let m = self.modal.as_mut().unwrap();
                m.at = stepped(
                    m.at,
                    count,
                    key.code,
                    self.layout.modal_rows.height.max(1) as usize,
                );
                if matches!(kind, Prompt::Help | Prompt::Details(_)) {
                    m.top = m.at;
                }
                self.clamp();
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
            Prompt::Help | Prompt::Details(_) => self.close_modal(),
            Prompt::Mnemonic => {}
            _ => {
                let Some(choice) = self.choices().get(at).cloned() else {
                    return;
                };
                if let Some(why) = choice.disabled {
                    self.modal.as_mut().unwrap().error = Some(why.into());
                    return;
                }
                let finder_at = self
                    .session
                    .state()
                    .project()
                    .and_then(|p| p.finder())
                    .and_then(|f| f.at());
                self.close_modal();
                match choice.target {
                    ChoiceTarget::Action(id) => self.run(id),
                    ChoiceTarget::Path(path) if kind == Prompt::RecentRoots => self.open_root(path),
                    ChoiceTarget::Path(path) => {
                        if let Some(at) = finder_at.filter(|_| kind == Prompt::Finder) {
                            self.dispatch(Command::OpenAt { path, at });
                        } else {
                            self.dispatch(Command::Open(path));
                        }
                    }
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
            (KeyCode::F(10), _, _, _)
            | (KeyCode::Char(':'), false, _, _)
            | (KeyCode::Char('p' | 'P'), true, true, _) => {
                self.prompt(Prompt::Commands);
                return;
            }
            (KeyCode::F(1), _, _, _) | (KeyCode::Char('?'), false, _, _) => {
                self.prompt(Prompt::Help);
                return;
            }
            (KeyCode::F(2), _, _, _) => {
                let mut notices = self.session.state().notices();
                if let Some(notice) = &self.notice {
                    notices.push(notice.clone());
                }
                self.prompt(Prompt::Details(if notices.is_empty() {
                    "No reader notices".into()
                } else {
                    notices.join("\n\n")
                }));
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
            (KeyCode::Char('p'), true, false, _) => Some(CommandId::FindFile),
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
            (KeyCode::Char('n' | 'N'), true, true, _) => Some(CommandId::NewWindow),
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
            self.run(id);
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
            if self.tree_shown && self.session.state().project().is_some() {
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
                let len = self.session.state().project().map_or(0, |p| p.rows().len());
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
        let Some((root, row)) = self
            .session
            .state()
            .project()
            .and_then(|p| Some((p.root().to_owned(), p.rows().get(self.tree_at)?.clone())))
        else {
            return;
        };
        if let Some(open) = row.dir {
            if !(key == KeyCode::Left && !open || key == KeyCode::Right && open) {
                self.dispatch(Command::ToggleDir(row.path.to_string()));
            }
        } else if key != KeyCode::Left {
            self.dispatch(Command::Open(root.join(&*row.path)));
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
            self.hover_at(x, y);
            return;
        }
        self.hover = None;
        if self.modal.is_some() {
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
            } else if matches!(
                event.kind,
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
            ) {
                let key = if event.kind == MouseEventKind::ScrollDown {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                };
                self.modal_key(KeyEvent::new(key, KeyModifiers::NONE));
            }
            return;
        }
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Right)) {
            self.prompt(Prompt::Commands);
            return;
        }
        if matches!(event.kind, MouseEventKind::Down(MouseButton::Left)) && y == 0 {
            self.prompt(Prompt::Commands);
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
                if self
                    .session
                    .state()
                    .project()
                    .is_some_and(|p| row < p.rows().len())
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
    pub(crate) fn clamp(&mut self) {
        let tree = self.session.state().project().map_or(0, |p| p.rows().len());
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
            && (!self.tree_shown || self.session.state().project().is_none())
        {
            self.focus = Focus::Text;
        }
        if let Some(modal) = &self.modal
            && !matches!(modal.kind, Prompt::Help | Prompt::Details(_))
        {
            let len = self.choices().len();
            let modal = self.modal.as_mut().unwrap();
            modal.at = modal.at.min(len.saturating_sub(1));
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
