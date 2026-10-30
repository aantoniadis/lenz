//! Draft controls and protection for two-way editable text comparisons.
use super::*;
use crate::{edit_session::EditMenuCommand,file_actions::FileAction};

impl Viewer {
    pub(super) fn edit_menu(&mut self, command:EditMenuCommand,cx:&mut Context<Self>) {
        if let Some(reason)=self.edit_disabled(command) {
            self.toast(reason,cx);
        } else if let Some(command)=command.command(self.key_block()) {
            self.edit_current(command,cx);
        } else {
            self.search=None;self.applied_scroll=0.0;cx.notify();
        }
    }

    pub(super) fn edit_disabled(&self,command:EditMenuCommand)->Option<String> {
        if command==EditMenuCommand::Revert && (!matches!(&self.spec,Spec::Git{cached,revs,..} if crate::changeset::right_is_worktree(*cached,revs))
            || self.set.comparisons.get(self.comparison.current).is_none_or(|c|!matches!(&c.right,Source::File(_)))) {
            return Some("Revert hunk requires a working-tree comparison".into());
        }
        self.comparison.edit_disabled(&self.edits,self.doc().map(|d|&d.text),command)
    }

    pub(super) fn file_disabled(&self,action:FileAction,index:usize)->Option<String> {
        let dirty_alias=self.set.git_root().zip(self.set.comparisons.get(index))
            .is_some_and(|(root,comparison)|self.edits.dirty_path(&root.join(&comparison.path)));
        if matches!(action,FileAction::Discard|FileAction::UndoDiscard) && (self.edits.dirty(index)||dirty_alias) {
            return Some("Save or discard unsaved changes first".into());
        }
        self.file_actions.disabled(action,&self.set,index,&self.spec,self.edits.has_undo(index)).map(str::to_string)
    }

    /// Replacement and app close never silently drop drafts. The focused draft
    /// retains its Save / Discard controls; cancelling means staying in this view.
    pub fn edits_may_close(&mut self,cx:&mut Context<Self>)->bool {
        self.edits_ask("Unsaved changes retained — Save (Ctrl/Cmd+S) or Discard unsaved changes before changing comparisons or closing.",cx)
    }

    /// Shows the first unsaved draft and says why the view stays.
    fn edits_ask(&mut self,message:&str,cx:&mut Context<Self>)->bool {
        if !self.edits.has_dirty() {return true;}
        if let Some(index)=self.edits.dirty_files().into_iter().find(|&i|i<self.set.comparisons.len()) && index!=self.comparison.current {self.open(index,Place::Top,cx);}
        self.write_error=Some(message.into());self.toast(message.into(),cx);
        false
    }

    /// Quit, the close button and Ctrl/Cmd+W: unsaved drafts ask first, and a second quit closes without them.
    pub fn may_quit(&mut self,cx:&mut Context<Self>)->bool {
        (self.edits.quit()||self.edits_ask(crate::edit_session::QUIT_QUESTION,cx)) && self.merge_may_close(cx) && self.picker_v_may_close(cx) && self.conflicts_may_close(cx)
    }

    pub(super) fn quit_if_ready(&mut self,cx:&mut Context<Self>) {
        if self.may_quit(cx) {cx.quit();}
    }

    pub(super) fn install_file_undo(&mut self,index:usize,written:crate::edit_session::Written,cx:&mut Context<Self>) {
        self.loaded.insert(index,Loaded::new(Arc::new(Doc{text:written.document,image:None}),self.comparison.generation));
        self.comparison.cache.0.remove(&index);
        self.toast("Change undone in draft — open that file and Save to write it".into(),cx);
    }
}
