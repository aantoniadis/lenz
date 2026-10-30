//! History of the explorer's selected entry, independent of the active editor tab.
use super::*;

impl Session {
    fn explorer_history_target(&self) -> Result<(&Path, &str, bool), &'static str> {
        let project = self.state.project.as_ref().ok_or("No folder is open")?;
        let selected = self.state.explorer.selected.as_deref().unwrap_or("");
        if selected.contains("://") || !crate::create::is_relative(selected) {
            return Err("Highlight a file or folder in the open project");
        }
        let directory = selected.is_empty() || project.has_dir(selected);
        if !directory && !project.has_file(selected) {
            return Err("The highlighted file or folder is no longer in the project");
        }
        if self.state.repo.is_none() {
            return Err(if self.state.repo_answered != self.state.repo_anchor { "Looking up the Git repository" } else { "Not in a Git repository" });
        }
        Ok((project.root(), selected, directory))
    }

    /// Why the explorer's Show History command cannot run, without filesystem or Git work in menus.
    pub fn explorer_history_disabled(&self) -> Option<&'static str> { self.explorer_history_target().err() }

    /// Lenz Diff history for the highlighted file (following renames) or folder, including the project root.
    /// Both adapters launch these same arguments directly; names remain literal Git pathspecs.
    pub fn explorer_history_launch(&self, exe: &Path, terminal: bool) -> Result<lenz_content::open::Launch, String> {
        let fail = |why: &str| format!("Show History: {why}");
        let (project, selected, directory) = self.explorer_history_target().map_err(fail)?;
        let root = &self.state.repo.as_ref().ok_or_else(|| fail("Not in a Git repository"))?.root;
        let project = project.canonicalize().map_err(|e| fail(&e.to_string()))?;
        let root = root.canonicalize().map_err(|e| fail(&e.to_string()))?;
        let relative = project.strip_prefix(&root).map_err(|_| fail("The project is no longer in this Git repository"))?;
        let relative = if selected.is_empty() { relative.to_path_buf() } else { relative.join(selected) };
        let metadata = std::fs::symlink_metadata(root.join(&relative)).map_err(|e| fail(&e.to_string()))?;
        if directory != metadata.is_dir() {
            return Err(fail("The highlighted entry changed; refresh the project first"));
        }
        let path = relative.to_str().ok_or_else(|| fail("This path is not valid UTF-8"))?;
        let path = if directory { format!(":(literal){}", if path.is_empty() { "." } else { path }) } else { path.to_string() };
        let program = lenz_diff(exe, terminal, "Show History")?;
        let args = vec!["log".into(), if directory { "--" } else { "--follow" }.into(), path];
        Ok(lenz_content::open::Launch { program, args, cwd: Some(root) })
    }
}
