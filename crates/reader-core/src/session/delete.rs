//! Deleting a file or folder from the explorer: into the trash by default, from where this session (or the desktop)
//! can restore it, or permanently once confirmed. Unsaved work is never deleted with its file: a document with a
//! draft must be saved or discarded first, and deleted work is not offered back by recovery.
use super::*;
use crate::trash::{self, Removal};

impl Session {
    /// What deleting `from` (project-relative) does, for the confirmation dialog: never the entry's contents.
    pub fn delete_prompt(&self, removal: Removal, from: &str) -> String {
        let source = self.state.project.as_ref().map(|p| p.root().join(from));
        let folder = source.as_deref().is_some_and(Path::is_dir);
        let open = source.as_deref().map_or(0, |source| self.state.tabs.iter().filter(|t| t.retrieved.is_none() && !t.untitled() && t.source.starts_with(source)).count());
        let what = if folder { format!("the folder {from} and everything in it") } else { from.to_string() };
        let how = match removal {
            Removal::Trash => "It moves to the trash; Restore Last Deleted Entry (or the desktop's trash) puts it back.",
            Removal::Permanent => "It is removed at once and cannot be restored.",
        };
        let tabs = match open { 0 => String::new(), 1 => " Its open tab closes.".into(), n => format!(" {n} open tabs close.") };
        format!("{} {what}? {how}{tabs}", removal.label())
    }

    /// Why `from` cannot be deleted now: what both dialogs show, and what the session checks again before deleting.
    pub fn delete_refusal(&self, _removal: Removal, from: &str) -> Option<String> {
        let Some(project) = self.state.project.as_ref() else { return Some("No folder is open".into()) };
        if from.is_empty() || !crate::create::is_relative(from) { return Some("Choose a file or folder of the open project".into()); }
        let source = project.root().join(from);
        if std::fs::symlink_metadata(&source).is_err() { return Some(format!("{from} is no longer there")); }
        if let Some(tab) = self.state.tabs.iter().find(|t| t.is_file() && t.source.starts_with(&source) && self.state.tab_dirty(t.id)) {
            return Some(format!("{} has unsaved changes: save or discard them first", tab.title()));
        }
        self.relocate_blocked(&source)
    }

    pub(super) fn delete_entry(&mut self, removal: Removal, from: String) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        if let Some(why) = self.delete_refusal(removal, &from) {
            self.state.messages.push(format!("{}: {why}. Nothing was deleted.", removal.label()));
            return repaint;
        }
        let Some(source) = self.state.project.as_ref().map(|p| p.root().join(&from)) else { return Effect::default() };
        let outcome = match removal {
            Removal::Trash => trash::trash(&source, self.trash_home.as_deref()).map(|trashed| self.trashed.push(trashed)),
            Removal::Permanent => match std::fs::symlink_metadata(&source) {
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(&source),
                _ => std::fs::remove_file(&source),
            }.map_err(|e| e.to_string()),
        };
        // What is gone is reconciled even after a partial failure; what remains stays open and listed.
        let gone = std::fs::symlink_metadata(&source).is_err();
        self.forget_deleted(&source);
        if gone { if let Some(p) = &mut self.state.project { p.remove(&from); } } else { self.rescan(); }
        self.state.tree_revision += 1;
        let parent = crate::relocate::split(&from).0;
        self.state.explorer.selected = (!parent.is_empty()).then(|| parent.to_string());
        self.synced = 0;
        self.rewatch();
        let message = match (removal, outcome) {
            (Removal::Trash, Ok(())) => format!("Moved {from} to the trash. Restore Last Deleted Entry puts it back."),
            (Removal::Permanent, Ok(())) => format!("Deleted {from} permanently."),
            (Removal::Trash, Err(why)) => format!("Delete: {why}. Nothing was deleted; Delete Permanently removes it without the trash."),
            (Removal::Permanent, Err(why)) if gone => format!("Delete Permanently: {why}."),
            (Removal::Permanent, Err(why)) => format!("Delete Permanently: {why}. Some of {from} may remain; what remains is still listed."),
        };
        self.state.messages.push(message);
        repaint
    }

    /// Open documents whose files are gone close (they are clean: a draft refuses the delete), their cached text
    /// goes, and recovery cannot offer them back.
    fn forget_deleted(&mut self, source: &Path) {
        if self.state.tabs.iter().any(|t| t.comparison_side.is_some() && t.source.starts_with(source)) { self.close_saved_comparison(); }
        let closing: Vec<(TabId, PathBuf, Option<crate::text::BufferId>)> = self.state.tabs.iter()
            .filter(|t| t.retrieved.is_none() && !t.untitled() && t.source.starts_with(source) && std::fs::symlink_metadata(&t.source).is_err())
            .map(|t| (t.id, t.source.clone(), t.buffer)).collect();
        for (id, path, buffer) in closing {
            self.release(id);
            self.remove_tab(id, true);
            self.cache.take_any(&path);
            self.recovery.discard(crate::recovery::Key::File { path: normalized(&path) });
            if let Some(buffer) = buffer.filter(|b| !self.state.tabs.iter().any(|t| t.buffer == Some(*b))) {
                self.state.sources.buffers.remove(&buffer);
                self.state.sources.aliases.retain(|_, b| *b != buffer);
            }
        }
    }

    /// Puts the entry this session last moved to the trash back where it was.
    pub(super) fn restore_deleted(&mut self) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        let Some(trashed) = self.trashed.pop() else {
            self.state.messages.push("Nothing was moved to the trash in this session".into());
            return repaint;
        };
        let name = self.state.project.as_ref().and_then(|p| p.relative(&trashed.original)).map_or_else(|| trashed.original.display().to_string(), |r| r.to_string());
        match trash::restore(&trashed) {
            Ok(()) => {
                self.rescan();
                self.state.messages.push(format!("Restored {name} from the trash."));
            }
            Err(why) => {
                // Still in the trash but blocked: it stays offered. Gone from the trash: nothing left to offer.
                if std::fs::symlink_metadata(&trashed.item).is_ok() { self.trashed.push(trashed); }
                self.state.messages.push(format!("Restore: {why}."));
            }
        }
        repaint
    }

    /// The home trash tests use instead of the user's own.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_trash_home_for_test(&mut self, home: PathBuf) { self.trash_home = Some(home); }
}
