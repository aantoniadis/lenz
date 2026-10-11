//! The toolbar's Find, following keyboard focus.
use super::*;

impl Viewer {
    /// A toolbar Find follows the same focus as Ctrl/Cmd+F, including list search.
    pub(super) fn find_focused(&mut self,cx:&mut Context<Self>) {
        if self.merge.is_some(){self.merge_act(crate::merge_session::Act::Find,cx);return;}
        if self.layout.files() && self.nav_focused {self.comparison.find=None;self.begin_file_search(true);}
        else if self.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.shown && p.focus==Pane::Log) {self.comparison.find=None;self.begin_log_search(true);}
        else if self.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.shown && p.sidebar && p.focus==Pane::Sidebar) {self.comparison.find=None;}
        else if let Some(why)=self.interaction_disabled(commands::Interaction::Find) {self.toast(why.into(),cx);}
        else {self.open_find();}
        cx.notify();
    }
}
