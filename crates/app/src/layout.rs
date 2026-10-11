//! Which panels surround the diff, and Focus: the diff alone, restoring the panels it hid. Both frontends.
use crate::repository_session::{RepositoryCommand, RepositorySession};

/// Branches (picker sidebar), History (commit log) and Files (navigator).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Panels { pub branches: bool, pub history: bool, pub files: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutCommand { ToggleBranches, ToggleHistory, ToggleFiles, ToggleFocus, LeaveFocus }

impl LayoutCommand {
    pub const fn label(self) -> &'static str {
        match self {
            Self::ToggleBranches => "Show / hide Branches",
            Self::ToggleHistory => "Show / hide History",
            Self::ToggleFiles => "Show / hide Files",
            Self::ToggleFocus => "Focus",
            Self::LeaveFocus => "Overview",
        }
    }
}

/// Files visibility and, in Focus, the Overview panels to restore.
#[derive(Debug, Default)]
pub struct Layout { files: bool, saved: Option<Panels> }

impl Layout {
    pub fn new(files: bool) -> Self { Self { files, saved: None } }
    pub fn files(&self) -> bool { self.files }
    /// Show or hide Files for a view that owns the screen (merge, Version Picker, Conflict List); Focus is unchanged.
    pub fn set_files(&mut self, files: bool) { self.files = files; }
    /// In Focus while Branches and History are still hidden; anything that shows one (file history, the Conflict List)
    /// ends it, so Esc never swaps in the layout from before. Windows without a repository have no Overview.
    pub fn focused(&self, repo: Option<&RepositorySession>) -> bool {
        let Some(r) = repo else { return false };
        self.saved.is_some() && r.panels() == (false, false)
    }
    /// Whether adapters show the Focus toolbar: in Focus, or in a window that has no Overview.
    pub fn shows_focus(&self, repo: Option<&RepositorySession>) -> bool { repo.is_none() || self.focused(repo) }

    pub fn disabled(&self, command: LayoutCommand, repo: Option<&RepositorySession>) -> Option<&'static str> {
        match command {
            LayoutCommand::ToggleFocus | LayoutCommand::LeaveFocus if repo.is_none() => Some("No Overview without a repository"),
            LayoutCommand::ToggleBranches | LayoutCommand::ToggleHistory if self.focused(repo) => Some("Leave Focus to show Branches / History"),
            _ => None,
        }
    }

    pub fn panels(&self, repo: Option<&RepositorySession>) -> Panels {
        let (branches, history) = repo.map_or((false, false), RepositorySession::panels);
        Panels { branches, history, files: self.files }
    }

    pub fn execute(&mut self, command: LayoutCommand, repo: Option<&mut RepositorySession>) {
        if self.disabled(command, repo.as_deref()).is_some() { return; }
        let focused = self.focused(repo.as_deref());
        let now = self.panels(repo.as_deref());
        match command {
            LayoutCommand::ToggleFocus if !focused => {
                self.saved = Some(now);
                self.apply(repo, Panels { branches: false, history: false, files: now.files });
            }
            LayoutCommand::ToggleFocus | LayoutCommand::LeaveFocus => {
                // left some other way: forget the panels it hid
                if let Some(saved) = self.saved.take().filter(|_| focused) {
                    self.apply(repo, Panels { files: self.files, ..saved });
                }
            }
            LayoutCommand::ToggleFiles => self.files = !self.files,
            LayoutCommand::ToggleBranches => self.toggle(repo, now, |p| p.branches = !p.branches),
            LayoutCommand::ToggleHistory => self.toggle(repo, now, |p| p.history = !p.history),
        }
    }

    fn toggle(&mut self, repo: Option<&mut RepositorySession>, mut panels: Panels, flip: impl FnOnce(&mut Panels)) {
        self.saved = None;
        flip(&mut panels);
        self.apply(repo, panels);
    }

    fn apply(&mut self, repo: Option<&mut RepositorySession>, panels: Panels) {
        self.files = panels.files;
        if let Some(r) = repo {
            r.execute(RepositoryCommand::SetPanels { branches: panels.branches, history: panels.history });
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::{commit_all, git, repo, write};
    use crate::repository_session::RepositorySession;

    fn session(root: &std::path::Path) -> RepositorySession {
        let request = crate::cli::parse(&["log".into()], root).unwrap();
        let mut state = RepositorySession::open(&request, root.into());
        let started = std::time::Instant::now();
        while state.busy() { state.poll(); assert!(started.elapsed().as_secs() < 10); std::thread::sleep(std::time::Duration::from_millis(2)); }
        state
    }

    #[test]
    fn toggling_files_leaves_the_keyboard_where_it_was() {
        use crate::repository_session::Pane;
        let root = repo(); write(root.path(), "f", "a\n"); commit_all(root.path(), "a");
        let mut r = session(root.path());
        r.picker.focus = Pane::Diff;
        let mut layout = Layout::new(true);
        layout.execute(LayoutCommand::ToggleFiles, Some(&mut r));
        assert_eq!(r.picker.focus, Pane::Diff, "Files is not a picker pane");
        layout.execute(LayoutCommand::ToggleBranches, Some(&mut r));
        assert_eq!(r.picker.focus, Pane::Diff, "hiding Branches while the diff has the keyboard keeps it there");
        layout.execute(LayoutCommand::ToggleBranches, Some(&mut r));
        assert_eq!(r.picker.focus, Pane::Sidebar, "a pane that appears takes the keyboard");
    }


    #[test]
    fn focus_hides_branches_and_history_and_keeps_files() {
        let root = repo(); write(root.path(), "f", "a\n"); commit_all(root.path(), "a");
        let mut r = session(root.path());
        let mut layout = Layout::new(true);
        let before = layout.panels(Some(&r));
        assert_eq!(before, Panels { branches: true, history: true, files: true });
        layout.execute(LayoutCommand::ToggleFocus, Some(&mut r));
        assert!(layout.focused(Some(&r)));
        assert_eq!(layout.panels(Some(&r)), Panels { branches: false, history: false, files: true });
        layout.execute(LayoutCommand::LeaveFocus, Some(&mut r));
        assert!(!layout.focused(Some(&r)));
        assert_eq!(layout.panels(Some(&r)), before);
    }

    #[test]
    fn files_toggle_inside_focus_keeps_focus_and_survives_leaving() {
        let root = repo(); write(root.path(), "f", "a\n"); commit_all(root.path(), "a");
        let mut r = session(root.path());
        let mut layout = Layout::new(true);
        layout.execute(LayoutCommand::ToggleFocus, Some(&mut r));
        layout.execute(LayoutCommand::ToggleFiles, Some(&mut r));
        assert!(layout.focused(Some(&r)), "Files is part of Focus");
        assert!(!layout.files());
        layout.execute(LayoutCommand::LeaveFocus, Some(&mut r));
        assert_eq!(layout.panels(Some(&r)), Panels { branches: true, history: true, files: false }, "the Files choice stays");
    }

    #[test]
    fn branches_and_history_are_disabled_in_focus() {
        let root = repo(); write(root.path(), "f", "a\n"); commit_all(root.path(), "a");
        let mut r = session(root.path());
        let mut layout = Layout::new(true);
        layout.execute(LayoutCommand::ToggleFocus, Some(&mut r));
        assert_eq!(layout.disabled(LayoutCommand::ToggleHistory, Some(&r)), Some("Leave Focus to show Branches / History"));
        assert_eq!(layout.disabled(LayoutCommand::ToggleBranches, Some(&r)), Some("Leave Focus to show Branches / History"));
        layout.execute(LayoutCommand::ToggleHistory, Some(&mut r));
        assert!(layout.focused(Some(&r)), "a disabled command does nothing");
        assert_eq!(layout.disabled(LayoutCommand::ToggleFocus, Some(&r)), None);
    }

    #[test]
    fn without_a_repository_there_is_no_overview() {
        let mut layout = Layout::new(true);
        assert!(!layout.focused(None));
        assert!(layout.shows_focus(None));
        assert_eq!(layout.disabled(LayoutCommand::ToggleFocus, None), Some("No Overview without a repository"));
        assert_eq!(layout.disabled(LayoutCommand::LeaveFocus, None), Some("No Overview without a repository"));
        layout.execute(LayoutCommand::ToggleFocus, None);
        assert_eq!(layout.panels(None), Panels { files: true, ..Panels::default() }, "nothing to hide");
        layout.execute(LayoutCommand::ToggleFiles, None);
        assert_eq!(layout.panels(None), Panels::default());
    }

    #[test]
    fn showing_a_panel_any_other_way_ends_focus() {
        let root = repo(); write(root.path(), "f", "a\n"); commit_all(root.path(), "a");
        let mut r = session(root.path());
        let mut layout = Layout::new(true);
        layout.execute(LayoutCommand::ToggleFocus, Some(&mut r));
        // e.g. File history opens the picker on its own
        r.execute(crate::repository_session::RepositoryCommand::TogglePicker);
        assert!(!layout.focused(Some(&r)), "panels are back, so Focus is over");
        let shown = layout.panels(Some(&r));
        layout.execute(LayoutCommand::LeaveFocus, Some(&mut r));
        assert_eq!(layout.panels(Some(&r)), shown, "Esc must not swap in the layout from before Focus");
        assert_eq!(layout.disabled(LayoutCommand::ToggleHistory, Some(&r)), None);
    }

    #[test]
    fn refresh_keeps_focus() {
        let root = repo(); write(root.path(), "f", "a\n"); commit_all(root.path(), "a");
        git(root.path(), &["checkout", "-qb", "side"]);
        let mut r = session(root.path());
        let mut layout = Layout::new(true);
        layout.execute(LayoutCommand::ToggleFocus, Some(&mut r));
        r.execute(crate::repository_session::RepositoryCommand::RefreshRepository);
        let started = std::time::Instant::now();
        while r.busy() { r.poll(); assert!(started.elapsed().as_secs() < 10); std::thread::sleep(std::time::Duration::from_millis(2)); }
        assert_eq!(layout.panels(Some(&r)), Panels { branches: false, history: false, files: true });
        assert!(layout.focused(Some(&r)));
    }
}
