//! Project Tabs: one window (or terminal) holding several independent project views, each its own [`Session`] with
//! its own folder, tabs, explorer, language servers and drafts. They are shown one at a time as tabs, or Side by Side
//! as columns. Views never share or synchronize documents; closing one asks about its own unsaved work only.
//!
//! The adapters own their views (GUI entities, terminal apps) and their geometry; this module owns the policy both
//! follow: which view is active, how the active one moves when views open and close, which views a layout shows for
//! the room there is, how that room is divided, and what each project tab is called.
use std::ops::Range;

use crate::session::Session;

/// How a window shows its projects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// One project at a time, chosen from the project tab strip.
    #[default]
    Tabs,
    /// Every project that fits, as equal columns left to right; the active one has the keys.
    SideBySide,
}

/// What a project-tab command asks the adapter's host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// A new project tab on the start screen.
    New,
    /// Ask for a folder, then open it in a new project tab.
    OpenFolder,
    /// Close this project's tab (its unsaved work is asked about first); the last one closes the window.
    Close,
    Next,
    Previous,
    /// Switch between [`Layout::Tabs`] and [`Layout::SideBySide`].
    ToggleSideBySide,
}

/// The host's view of its projects, handed to each session so menus resolve availability and toggles centrally.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// How many projects the window holds (one: no strip is shown).
    pub count: usize,
    /// This session's place among them.
    pub position: usize,
    pub layout: Layout,
}

/// The project views' order and the active one. Adapters keep their views in a parallel list and apply each change
/// to both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Projects {
    count: usize,
    active: usize,
    layout: Layout,
}

impl Default for Projects {
    fn default() -> Self { Self::new(1) }
}

/// The narrowest column a project is given Side by Side, in terminal cells; adapters scale it to pixels.
pub const MIN_COLUMN_CELLS: u32 = 36;

impl Projects {
    pub fn new(count: usize) -> Self { Self { count, active: 0, layout: Layout::Tabs } }
    /// A set as an adapter holds it: `active` is clamped to the projects there are, and one project is shown as tabs.
    pub fn with(count: usize, active: usize, layout: Layout) -> Self {
        Self { count, active: active.min(count.saturating_sub(1)), layout: if count > 1 { layout } else { Layout::Tabs } }
    }
    pub fn count(&self) -> usize { self.count }
    pub fn active(&self) -> usize { self.active }
    pub fn layout(&self) -> Layout { self.layout }
    /// Whether the project tab strip is shown: only once there is a second project.
    pub fn strip(&self) -> bool { self.count > 1 }

    /// A project was added at the end; it becomes the active one.
    pub fn opened(&mut self) -> usize {
        self.count += 1;
        self.active = self.count - 1;
        self.active
    }

    /// Make `index` active; false when there is no such project.
    pub fn activate(&mut self, index: usize) -> bool {
        if index >= self.count { return false; }
        self.active = index;
        true
    }

    /// The next (or previous) project, wrapping around.
    pub fn step(&mut self, forward: bool) -> usize {
        if self.count > 0 {
            self.active = if forward { (self.active + 1) % self.count } else { (self.active + self.count - 1) % self.count };
        }
        self.active
    }

    /// The projects whose `kept` entry is false were closed. The active project stays active when it is kept;
    /// otherwise its left neighbour (or the first) takes over. The layout returns to tabs with one project left.
    pub fn retain(&mut self, kept: &[bool]) {
        debug_assert_eq!(kept.len(), self.count);
        let before = kept.iter().take(self.active).filter(|k| **k).count();
        let active_kept = kept.get(self.active).copied().unwrap_or(false);
        self.count = kept.iter().filter(|k| **k).count();
        self.active = if active_kept { before } else { before.saturating_sub(1) };
        self.active = self.active.min(self.count.saturating_sub(1));
        if self.count < 2 { self.layout = Layout::Tabs; }
    }

    /// The project at `index` was closed.
    pub fn remove(&mut self, index: usize) {
        let kept: Vec<bool> = (0..self.count).map(|i| i != index).collect();
        self.retain(&kept);
    }

    pub fn toggle_layout(&mut self) -> Layout {
        self.layout = match self.layout {
            Layout::Tabs if self.count > 1 => Layout::SideBySide,
            _ => Layout::Tabs,
        };
        self.layout
    }

    /// The projects shown when `capacity` columns fit: the active one alone in tabs; Side by Side as many as fit,
    /// in order, a window that keeps the active one in view.
    pub fn shown(&self, capacity: usize) -> Range<usize> {
        if self.count == 0 { return 0..0; }
        let fit = match self.layout { Layout::Tabs => 1, Layout::SideBySide => capacity.clamp(1, self.count) };
        let start = (self.active + 1).saturating_sub(fit).min(self.count - fit);
        start..start + fit
    }

    /// The session's view of this set at `position`.
    pub fn info(&self, position: usize) -> Info { Info { count: self.count, position, layout: self.layout } }
}

/// How many columns of at least `min` fit in `total`, each after the first also paying for a `divider`.
pub fn capacity(total: u32, min: u32, divider: u32) -> usize {
    if total < min || min == 0 { return 1; }
    (1 + (total - min) / (min + divider)) as usize
}

/// Split `total` into `count` columns separated by `divider`: the starts and widths, left to right, with the
/// remainder spread over the first columns so widths differ by at most one unit.
pub fn columns(total: u32, count: usize, divider: u32) -> Vec<Range<u32>> {
    let count = count.max(1) as u32;
    let room = total.saturating_sub(divider * (count - 1));
    let (each, extra) = (room / count, room % count);
    let mut at = 0;
    (0..count).map(|i| {
        let width = each + u32::from(i < extra);
        let column = at..at + width;
        at += width + divider;
        column
    }).collect()
}

/// Where a project chosen while this tab already has one open goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenChoice {
    /// Open it here, in place of the project this tab has open.
    ThisTab,
    /// Open it in a new project tab, keeping this one as it is.
    NewTab,
    Cancel,
}

impl OpenChoice {
    pub const ALL: [Self; 3] = [Self::ThisTab, Self::NewTab, Self::Cancel];
    pub fn label(self) -> &'static str {
        match self {
            Self::ThisTab => "This Tab",
            Self::NewTab => "New Project Tab",
            Self::Cancel => "Cancel",
        }
    }
}

/// The question asked when a project is chosen (Open Recent Project) while this tab has another one open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenPrompt {
    pub ticket: u64,
    pub path: std::path::PathBuf,
    pub name: String,
    pub location: String,
    /// The project this tab has open.
    pub current: String,
}

impl OpenPrompt {
    pub fn text(&self) -> String {
        format!("Open {} ({}/{})?\n\nThis project tab has {} open. Open the project here in its place, or in a new project tab beside it?\n\nThis Tab (t, Enter) · New Project Tab (n) · Cancel (Esc)", self.name, self.location, self.name, self.current)
    }
}

impl Session {
    /// The host's projects as this session sees them (set by the adapter whenever they change).
    pub fn projects(&self) -> Info { self.projects_info }

    pub fn set_projects(&mut self, info: Info) { self.projects_info = info; }

    /// The project tab's title: the open folder's name, else the shown file's, else the start screen's.
    pub fn project_title(&self) -> String {
        let state = self.state();
        if let Some(project) = state.project() {
            let root = project.root();
            return root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
        }
        state.active().map(|t| t.title()).unwrap_or_else(|| "Start".into())
    }

    /// Where this project tab is, for its label beside the title: the shown file's path (`~` for home), else the
    /// open folder's, else nothing (the start screen). A project tab reads `title – where`, as IntelliJ's windows do.
    pub fn project_place(&self) -> String {
        let state = self.state();
        let home = crate::start::home();
        state.active().filter(|t| t.is_file()).map(|t| t.source().to_owned())
            .or_else(|| state.project().map(|p| p.root().to_owned()))
            .map(|p| crate::start::tilde(&p, home.as_deref()))
            .unwrap_or_default()
    }

    /// Whether any of this project's tabs holds unsaved work (the project tab shows a mark).
    pub fn project_dirty(&self) -> bool {
        let state = self.state();
        state.tabs().iter().any(|t| state.tab_dirty(t.id()))
    }

    /// Open the project at `root` as chosen from a list (Open Recent Project): here when this tab has no project
    /// open or has this one, otherwise after asking where (see [`Session::open_project_prompt`]).
    pub(crate) fn open_project(&mut self, root: std::path::PathBuf) -> crate::session::Effect {
        let current = self.state().project().map(|p| p.root().to_owned());
        if current.as_ref().is_none_or(|c| *c == root) { return self.dispatch(crate::session::Command::OpenRoot(root)); }
        let home = crate::start::home();
        let (name, location) = crate::start::place(&root, home.as_deref());
        self.open_project_seq += 1;
        self.open_prompt = Some(OpenPrompt { ticket: self.open_project_seq, path: root, name, location, current: self.project_title() });
        crate::session::Effect { repaint: true, ..Default::default() }
    }

    /// The pending question of where to open a chosen project.
    pub fn open_project_prompt(&self) -> Option<&OpenPrompt> { self.open_prompt.as_ref() }

    /// The answer to [`Session::open_project_prompt`]: a stale ticket changes nothing.
    pub(crate) fn choose_open_project(&mut self, ticket: u64, choice: OpenChoice) -> crate::session::Effect {
        if self.open_prompt.as_ref().is_none_or(|p| p.ticket != ticket) { return Default::default(); }
        let Some(prompt) = self.open_prompt.take() else { return Default::default() };
        match choice {
            OpenChoice::ThisTab => self.dispatch(crate::session::Command::OpenRoot(prompt.path)),
            OpenChoice::NewTab => { self.project_tab_request = Some(prompt.path); crate::session::Effect { repaint: true, ..Default::default() } }
            OpenChoice::Cancel => crate::session::Effect { repaint: true, ..Default::default() },
        }
    }

    /// A project to open in a new project tab, which only the adapter's host can create: taken once.
    pub fn take_project_tab_request(&mut self) -> Option<std::path::PathBuf> { self.project_tab_request.take() }

    /// Why a project-tab command cannot run (`None`: it can).
    pub(crate) fn project_disabled(&self, action: Action) -> Option<&'static str> {
        let info = self.projects_info;
        match action {
            Action::New | Action::OpenFolder | Action::Close => None,
            Action::Next | Action::Previous => (info.count < 2).then_some("Only one project is open"),
            Action::ToggleSideBySide => (info.count < 2).then_some("Open a second project first"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_stepping_and_closing_keep_a_valid_active_project() {
        let mut p = Projects::new(1);
        assert!(!p.strip());
        assert_eq!(p.opened(), 1);
        assert_eq!(p.opened(), 2);
        assert!(p.strip());
        assert_eq!(p.step(true), 0, "wraps forward");
        assert_eq!(p.step(false), 2, "wraps back");
        assert!(!p.activate(3));
        assert!(p.activate(1));
        p.remove(1);
        assert_eq!((p.count(), p.active()), (2, 0), "a closed active project gives way to its left neighbour");
        p.activate(1);
        p.remove(0);
        assert_eq!((p.count(), p.active()), (1, 0), "an earlier project closing keeps the active one");
        p.retain(&[false]);
        assert_eq!((p.count(), p.active()), (0, 0));
        assert_eq!(p.shown(4), 0..0);
    }

    #[test]
    fn side_by_side_shows_what_fits_around_the_active_project_and_returns_to_tabs_with_one_left() {
        let mut p = Projects::new(5);
        assert_eq!(p.toggle_layout(), Layout::SideBySide);
        p.activate(0);
        assert_eq!(p.shown(3), 0..3);
        p.activate(4);
        assert_eq!(p.shown(3), 2..5);
        assert_eq!(p.shown(9), 0..5);
        assert_eq!(p.shown(0), 4..5, "always at least the active project");
        p.toggle_layout();
        assert_eq!(p.shown(9), 4..5, "tabs show the active project alone");
        p.toggle_layout();
        p.retain(&[true, false, false, false, false]);
        assert_eq!(p.layout(), Layout::Tabs);
        assert_eq!(p.toggle_layout(), Layout::Tabs, "one project has nothing to show beside it");
    }

    #[test]
    fn columns_divide_the_room_evenly_and_capacity_respects_the_minimum() {
        assert_eq!(columns(10, 3, 1), vec![0..3, 4..7, 8..10]);
        assert_eq!(columns(100, 1, 1), vec![0..100]);
        let parts = columns(1001, 4, 1);
        assert_eq!(parts.last().unwrap().end, 1001);
        assert!(parts.windows(2).all(|w| w[1].start == w[0].end + 1));
        assert_eq!(capacity(80, 36, 1), 2);
        assert_eq!(capacity(72, 36, 1), 1);
        assert_eq!(capacity(10, 36, 1), 1);
        assert_eq!(capacity(1000, 0, 1), 1);
    }
}
