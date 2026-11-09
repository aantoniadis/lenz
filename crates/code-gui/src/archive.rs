//! Archive presentation uses the same explorer rows and editable tabs as ordinary files.
use super::*;

pub(super) fn source_path_label(path: &std::path::Path) -> String {
    code_core::archive::Location::from_uri(path).map_or_else(
        || path.display().to_string(),
        |location| location.display_path(),
    )
}

impl ReaderView {
    pub(super) fn explorer_title(&self) -> String {
        let state = self.session.state();
        state.explorer_search().label().unwrap_or_else(|| {
            state
                .project()
                .map(|project| {
                    project.root().file_name().map_or_else(
                        || project.root().display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    )
                })
                .or_else(|| state.archive_tree().title())
                .unwrap_or_else(|| "Files".into())
        })
    }

    /// An explicit archive request reveals its browser even when a narrow window hides the explorer.
    pub(super) fn reveal_archive_browser(&mut self, path: &str) {
        if self.session.state().archive_tree().is_archive(path) {
            self.tree_focus = true;
            self.tree_shown = true;
            self.results_focus = false;
            self.log_focus = false;
            self.find_focus = None;
            self.crumb_focus = None;
        }
    }

    /// Enter or a double-click takes typing into the member that the core actually opened.
    pub(super) fn focus_archive_member(&mut self, path: &str) {
        let state = self.session.state();
        if state.archive_tree().is_member(path)
            && state
                .active()
                .is_some_and(|tab| tab.source() == std::path::Path::new(path))
        {
            self.tree_focus = false;
            self.results_focus = false;
            self.log_focus = false;
        }
    }

    /// Left follows the shared archive parent/close action.
    pub(super) fn archive_parent_key(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(path) = self
            .session
            .state()
            .tree_rows()
            .nth(self.tree_at())
            .map(|row| row.path.clone())
        else {
            return false;
        };
        if !self.session.state().archive_tree().is_virtual(&path) {
            return false;
        }
        self.dispatch(Command::ArchiveTreeParent { path }, cx);
        true
    }

    pub(super) fn tree_icon(&self, row: &code_core::project::TreeRow) -> icons::Icon {
        if self.session.state().archive_tree().is_archive(&row.path) {
            icons::file(&row.name)
        } else {
            match row.dir {
                Some(open) => icons::folder(open),
                None => icons::file(&row.name),
            }
        }
    }
}
