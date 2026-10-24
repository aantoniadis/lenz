//! Creating a file or folder from the explorer. Name, collision and folder rules are checked before anything
//! exists, a created file opens as that file with its own buffer, and the explorer selects what was made.
use super::*;
use crate::create::{self, Kind};
use lenz_content::write::WriteOutcome;
use std::os::unix::fs::PermissionsExt;

impl Session {
    /// The folder New File / New Folder create in, project-relative (`""` is the root): the explorer's highlighted
    /// folder, the folder holding its highlighted file, or the root. `None` when no folder is open.
    pub fn create_target(&self) -> Option<String> {
        let project = self.state.project.as_ref()?;
        let selected = self.state.explorer.selected.as_deref().filter(|s| !s.is_empty() && create::is_relative(s));
        Some(match selected {
            Some(row) if project.has_file(row) => row.rsplit_once('/').map_or_else(String::new, |(dir, _)| dir.to_string()),
            Some(row) if project.has_dir(row) => row.to_string(),
            _ => String::new(),
        })
    }

    /// Why `kind` named `name` cannot be created in folder `dir` (project-relative, `""` the root): what both
    /// adapters' prompts show, and what the session itself checks before anything is created. A name other entries
    /// take, a missing folder and a folder whose own mode refuses new entries are refused here; a folder that only
    /// looks writable still fails at the create, which reports it then.
    pub fn create_refusal(&self, dir: &str, name: &str) -> Option<String> {
        let Some(project) = self.state.project.as_ref() else { return Some("No folder is open".into()) };
        if let Some(why) = create::dir_refusal(dir).or_else(|| create::name_refusal(name)) {
            return Some(why);
        }
        let folder = if dir.is_empty() { project.root().to_path_buf() } else { project.root().join(dir) };
        let path = folder.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Some(format!("{} already exists", create::relative(dir, name))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Some(format!("{} cannot be checked: {e}", path.display())),
        }
        match std::fs::metadata(&folder) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Some("That is not a folder".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some("The folder no longer exists".into()),
            Err(e) => return Some(format!("The folder cannot be read: {e}")),
        }
        if std::fs::metadata(&folder).is_ok_and(|m| m.permissions().mode() & 0o222 == 0) {
            return Some("The folder is read-only".into());
        }
        None
    }

    /// Create `kind` named `name` in folder `dir`, then show and select it; a file also opens as that file.
    pub(super) fn create_entry(&mut self, kind: Kind, dir: String, name: String) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        if let Some(why) = self.create_refusal(&dir, &name) {
            self.state.messages.push(format!("{}: {why}. Nothing was created.", kind.label()));
            return repaint;
        }
        let Some(folder) = self.state.project.as_ref().map(|p| if dir.is_empty() { p.root().to_path_buf() } else { p.root().join(&dir) }) else {
            return Effect::default();
        };
        let path = folder.join(&name);
        let rel: Arc<str> = create::relative(&dir, &name).into();
        match kind {
            Kind::Folder => {
                if let Err(e) = std::fs::create_dir(&path) {
                    self.state.messages.push(format!("New folder: {e}. Nothing was created."));
                    return repaint;
                }
                if let Some(p) = &mut self.state.project { p.add_folder(rel.clone()); }
                self.state.tree_revision += 1;
                self.show_created(rel.clone());
                self.state.messages.push(format!("Created folder {rel}"));
                repaint
            }
            Kind::File => {
                // The exact writer creates a new file and never replaces an entry that appeared meanwhile.
                match lenz_content::write::create_exact(&path, "UTF-8", false, "") {
                    WriteOutcome::Saved { .. } => {}
                    WriteOutcome::Rejected(why) => {
                        self.state.messages.push(format!("New file: {why}. Nothing was created."));
                        return repaint;
                    }
                    WriteOutcome::Uncertain(why) => {
                        self.state.messages.push(format!("New file: the outcome is uncertain: {why}. Inspect {} before using it.", path.display()));
                        return repaint;
                    }
                }
                if let Some(p) = &mut self.state.project { p.add_file(rel.clone()); }
                self.state.tree_revision += 1;
                self.show_created(rel.clone());
                self.state.messages.push(format!("Created file {rel}"));
                // Kept, like a file opened from the explorer by double-click: later navigation cannot replace it.
                self.navigate(path, true, None, true)
            }
        }
    }

    /// Ask both adapters to show and select `rel`, which is what the user just created.
    pub(super) fn show_created(&mut self, rel: Arc<str>) {
        self.state.explorer.selected = Some(rel.to_string());
        let ticket = self.state.tree_reveal.as_ref().map_or(1, |(_, t)| t + 1);
        self.state.tree_reveal = Some((rel, ticket));
    }
}
