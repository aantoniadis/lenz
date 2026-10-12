//! The Change Set Picker: the sidebar of sources and the commit log with its branch graph, above the File
//! Navigator and panes. Hidden in Focus Mode.

use super::*;
use crate::history::{self, Branches};

/// Graph lane width.
const LANE_W: f32 = 12.0;

pub(super) use crate::repository_session::{
    ConflictsEntry, Pane, Picker, SourceAction, SourceRow, Sources,
};
impl Sources {
    pub(super) fn rows(&self, palette: &theme::Palette, walking: &Branches) -> Vec<SourceRow> {
        let p = palette;
        self.rows_with_palette(
            walking,
            crate::repository_session::SourcePalette {
                fg: p.fg,
                dim: p.dim,
                accent: p.accent,
                status: [
                    p.status[0],
                    p.status[1],
                    p.status[2],
                    p.status[3],
                    p.status[4],
                ],
            },
        )
    }
}

impl Viewer {
    /// The picker sidebar's width (0 in Focus Mode or when hidden).
    pub(super) fn picker_w(&self) -> f32 {
        match self.repo.as_ref().map(|r| &r.picker) {
            Some(p) if p.shown && p.sidebar => crate::pane_layout::clamp_size(self.panes.viewport[0],self.panes.picker,100.0,120.0+if self.layout.files(){100.0}else{0.0}),
            _ => 0.0,
        }
    }

    /// The log's height, header included (0 in Focus Mode or when hidden).
    pub(super) fn log_h(&self) -> f32 {
        match self.repo.as_ref().map(|r| &r.picker) {
            Some(p) if p.shown && p.log_shown => crate::pane_layout::clamp_size((self.panes.viewport[1]-toolbar_h()).max(0.0),self.panes.log,2.0*line_h(),hdr_h()+4.0*line_h()),
            _ => 0.0,
        }
    }

    pub(super) fn picker_shown(&self) -> bool {
        self.repo.as_ref().map(|r| &r.picker).is_some_and(|p| p.shown)
    }

    /// Show or hide the picker (leave or enter Focus Mode); the log loads the first time it shows.
    pub(super) fn set_picker_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.nav_focused = false;
        self.panel_focused = false;
        let Some(r) = self.repo.as_mut() else { return };
        if r.picker.shown != shown {
            // the session reads the sidebar inventory the first time, and the log streams once it arrives
            r.execute(crate::repository_session::RepositoryCommand::TogglePicker);
        } else {
            r.picker.focus = if shown { Pane::Log } else { Pane::Diff };
        }
        self.poll_repository(cx);
        cx.notify();
    }


    /// Poll the shared session every 30 ms while it has background work (inventory, pull requests, log
    /// batches, a picked comparison). A paused log restarts the loop from render when the view nears its end.
    pub(super) fn poll_repository(&mut self, cx: &mut Context<Self>) {
        if self.repo.is_none() || std::mem::replace(&mut self.repo_polling, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
                if !this.update(cx, |v: &mut Viewer, cx| v.poll_repository_once(cx)).unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Run a repository command on the session and present what it produced at once.
    pub(super) fn repository_command(&mut self, command: crate::repository_session::RepositoryCommand, cx: &mut Context<Self>) {
        let Some(r) = self.repo.as_mut() else { return };
        r.execute(command);
        self.present_repository(cx);
        self.poll_repository(cx);
        cx.notify();
    }

    fn poll_repository_once(&mut self, cx: &mut Context<Self>) -> bool {
        let log_rows = self.log_rows();
        let Some(r) = self.repo.as_mut() else {
            self.repo_polling = false;
            return false;
        };
        let changed = r.poll_rows(log_rows);
        let more = r.busy();
        self.repo_polling = more;
        if changed && self.conflicts.is_some() {
            // the open Conflict List's own progress, over the inventory's reading from disk
            self.update_entry();
        }
        self.present_repository(cx);
        // a tab the session opened (a PR's Conversation) shows in Focus
        if self.pr_details_shown() && !self.layout.focused(self.repo.as_ref()) { self.layout_command(crate::layout::LayoutCommand::ToggleFocus, cx); }
        cx.notify();
        more
    }

    /// The session's results: a loaded comparison, a notice, effects.
    pub(super) fn present_repository(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.repo.as_mut() else { return };
        let (ready, notice, effects) = (r.take_change_set(), r.notice.take(), r.take_effects());
        let refreshed = ready.is_some() && r.take_refresh_placement();
        match ready {
            // a refresh keeps the file on screen, its place and the edits made to it
            Some((set, spec)) if refreshed => self.accept_repository_refresh(Arc::unwrap_or_clone(set), spec, cx),
            Some((set, spec)) if self.edits_may_close(cx) => self.stage_change_set(set, spec, cx),
            _ => {}
        }
        if let Some(notice) = notice {
            self.toast(notice, cx);
        }
        for effect in effects {
            self.repository_effect(effect, cx);
        }
    }

    /// Replace the comparison once its first file is read (at most `SWAP_WAIT` later), so the panes never go blank
    /// between two comparisons; a newer change set supersedes one still being read.
    pub(super) fn stage_change_set(&mut self, set: Arc<ChangeSet>, spec: Spec, cx: &mut Context<Self>) {
        use crate::repository_session::SWAP_WAIT;
        if set.comparisons.is_empty() {
            self.incoming = None;
            self.spec = spec;
            return self.show_change_set(set, cx);
        }
        let token = Arc::new(());
        let named = |p: &str| PathBuf::from(self.name.clone().unwrap_or_else(|| p.to_string()));
        let (lp, rp) = (named(&set.comparisons[0].old_path), named(&set.comparisons[0].path));
        let current = Arc::downgrade(&self.comparison.work_id);
        let (read, wait) = (Arc::downgrade(&token), Arc::downgrade(&token));
        let reading = set.clone();
        self.incoming = Some((token, set, spec));
        cx.spawn(async move |this, cx| {
            let doc = cx.background_executor().spawn(async move {
                crate::work::document(current, move || Doc::load(&reading, 0, &lp, &rp, false).ok())
            }).await.flatten();
            this.update(cx, |v: &mut Viewer, cx| v.swap_in(&read, doc.map(Arc::new), cx)).ok();
        }).detach();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SWAP_WAIT).await;
            this.update(cx, |v: &mut Viewer, cx| v.swap_in(&wait, None, cx)).ok();
        }).detach();
    }

    /// Show the staged change set if `token` is still the latest, with its first file when it was read in time.
    fn swap_in(&mut self, token: &std::sync::Weak<()>, first: Option<Arc<Doc>>, cx: &mut Context<Self>) {
        let current = self.incoming.as_ref().is_some_and(|(t, _, _)| token.upgrade().is_some_and(|live| Arc::ptr_eq(t, &live)));
        if !current { return; }
        let Some((_, set, spec)) = self.incoming.take() else { return };
        self.spec = spec;
        self.show_change_set_with(set, first, cx);
    }

    fn repository_effect(&mut self, effect: crate::repository_session::RepositoryEffect, cx: &mut Context<Self>) {
        use crate::repository_session::RepositoryEffect as E;
        match effect {
            E::Copy(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                self.toast(format!("Copied: {text}"), cx);
            }
            E::OpenUrl(url) => self.browse(&url, cx),
            E::OpenWorktree(path) => self.new_window(&path, cx),
            E::OpenRevisions { root, revs } => self.spawn_lenz(&revs, &root, cx),
            // the next unviewed file of a pull request, at its first change
            E::OpenFile(index) => self.open(index, Place::FirstChange, cx),
            E::Conflicts => self.show_conflicts(cx),
        }
    }

    pub(super) fn browse(&mut self, url: &str, cx: &mut Context<Self>) {
        if let Err(e) = open::browse(url) {
            self.toast(e, cx);
        }
    }

    /// `lenz args` in `cwd`, as a window of its own.
    fn spawn_lenz(&mut self, args: &[String], cwd: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(e) = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(args).current_dir(cwd).spawn()) {
            self.toast(format!("Cannot start lenz: {e}"), cx);
        }
    }

    pub(super) fn new_window(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(e) = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).arg("log").current_dir(path).spawn()) {
            self.toast(format!("Cannot start lenz: {e}"), cx);
        }
    }

    /// Keys while the log filter is being typed; false when not typing or the key isn't the filter's.
    pub(super) fn filter_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        use crate::repository_session::RepositoryCommand as C;
        if self.nav_focused || self.search.as_ref().is_some_and(|s|s.editing) { return false; }
        let Some(r) = self.repo.as_mut().filter(|r| r.picker.shown && r.picker.focus == Pane::Log && r.picker.filter_editing) else { return false;
        };
        r.picker.cancel_refresh();
        let command = match ev.keystroke.key.as_str() {
            "escape" if r.picker.filter.is_empty() => {
                r.picker.filter_editing = false;
                None
            }
            "escape" => Some(C::Filter(String::new())),
            "backspace" => {
                let mut filter = r.picker.filter.clone();
                filter.pop();
                Some(C::Filter(filter))
            }
            "enter" => {
                r.picker.filter_editing = false;
                Some(C::ApplyFilter)
            }
            "up" | "down" | "pageup" | "pagedown" => return false,
            _ => match Self::typed(ev) {
                Some(t) => Some(C::Filter(format!("{}{t}", r.picker.filter))),
                None => return false,
            },
        };
        match command {
            Some(command) => self.repository_command(command, cx),
            None => cx.notify(),
        }
        true
    }

    /// Esc in the log: leave file history or branch compare, or clear the filter; false when there's nothing to leave.
    pub(super) fn log_escape(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.repo.as_ref().map(|r| &r.picker).filter(|p| p.shown) else { return false;
        };
        if p.file_paths.is_none() && p.filter.is_empty() && p.compare.is_none() {
            return false;
        }
        self.repository_command(crate::repository_session::RepositoryCommand::ClearFilter, cx);
        true
    }

    /// Branch compare: the current branch against `refname`.
    pub(super) fn compare_with_current(&mut self, refname: String, three_dot: bool, cx: &mut Context<Self>) {
        let command = crate::repository_session::RepositoryCommand::Compare { left: "HEAD".into(), right: refname, merge_base: three_dot };
        self.repository_command(command, cx);
    }

    /// File history: the log narrowed to `path`'s commits (following renames), each showing only that file.
    pub(super) fn show_file_history(&mut self, path: String, cx: &mut Context<Self>) {
        self.repository_command(crate::repository_session::RepositoryCommand::FileHistory(path), cx);
    }


    /// The branch menu (Ctrl/Cmd+B, or the header): which branches the log walks.
    pub(super) fn branch_menu(&mut self, x: f32, cx: &mut Context<Self>) {
        let Some(p) = self.repo.as_ref().map(|r| &r.picker) else { return;
        };
        let mut items = vec![
            (crate::repository_session::command_label(&crate::repository_session::RepositoryCommand::RefreshRepository).into(),MenuAction::RepositoryRefresh),
            ("All branches".to_string(), MenuAction::Walk(Branches::All)),
            ("Current branch".to_string(), MenuAction::Walk(Branches::Current),
            ),
        ];
        items.extend(p.sources.branches.iter().take(20).map(|b| {
            (format!("⎇ {}", b.name), MenuAction::Walk(Branches::One(format!("refs/heads/{}", b.name))),
            )
        }));
        self.show_menu(Menu {
            x,
            y: toolbar_h() + line_h(),
            items,
            hover: None,
        });
        cx.notify();
    }

    /// Right-click in the picker: the commit menu in the log, branch and worktree actions in the sidebar.
    pub(super) fn picker_menu(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        if !self.picker_shown() {
            return false;
        }
        let (pw, lh) = (self.picker_w(), self.log_h());
        let file = self.set.comparisons.get(self.comparison.current).map(|c| c.repository_path().to_owned()).unwrap_or_default();
        let Some(p) = self.repo.as_mut().map(|r| &mut r.picker) else { return false;
        };
        let mut items = Vec::new();
        if x < pw {
            let row = ((y - toolbar_h()) / line_h() - 1.0 + p.sources.scroll).floor();
            let rows = p.sources.rows(&self.palette, &p.branches);
            match (row >= 0.0).then(|| rows.get(row as usize)).flatten().and_then(|r| r.action.clone()) {
                // a click on the branch shows its log
                Some(SourceAction::Branch(refname, _)) => {
                    items.push(("Compare with Current".to_string(), MenuAction::CompareBranch(refname),
                    ));
                }
                Some(SourceAction::Worktree(path)) => items.push(("Open in New Window".to_string(), MenuAction::NewWindow(path),
                )),
                // the open PR refreshes from its row
                Some(SourceAction::Pr(_)) => if let Some(r) = self.repo.as_ref() {
                    items.extend(r.source_menu(row as usize).into_iter().map(|(label, action)| (label.to_string(), MenuAction::Run(action))));
                },
                _ => {},
            }
        } else if y >= toolbar_h() + line_h() && y < toolbar_h() + lh {
            let row = ((y - toolbar_h()) / line_h() - 1.0 + p.scroll).floor();
            if row < 1.0 || row as u32 >= p.rows() {
                return true;
            }
            let row = row as u32;
            let Some(i) = p.log_row(row) else { return true };
            let id = p.log.id(i);
            let Some(r) = self.repo.as_ref() else { return true };
            for command in crate::repository_session::RepositorySession::log_context_commands(row, &file) {
                let label = crate::repository_session::command_label(&command);
                let label = r.row_disabled(row, &command).map_or_else(|| label.to_string(), |why| format!("{label} ({why})"));
                items.push((label, MenuAction::RepositoryRow { row, id: id.clone(), command }));
            }
            let p = &r.picker;
            items.push(("Compare with Branch…".to_string(), MenuAction::CompareWithBranch { row, id: id.clone() }));
            if let Some(url) = github_url(&p.root) {
                items.push(("Open on GitHub".to_string(), MenuAction::Browse(format!("{url}/commit/{id}"))));
            }
        } else {
            return false;
        }
        self.show_menu(Menu { x, y, items, hover: None,
        });
        cx.notify();
        true
    }

    /// Do a picker menu entry.
    pub(super) fn picker_menu_action(&mut self, action: &MenuAction, cx: &mut Context<Self>) {
        let Some(p) = self.repo.as_mut().map(|r| &mut r.picker) else { return;
        };
        let root = p.root.clone();
        match action {
            MenuAction::Walk(b) => self.repository_command(crate::repository_session::RepositoryCommand::Walk(b.clone()), cx),
            MenuAction::RepositoryRow { row, id, command } => {
                if p.log_row(*row).is_none_or(|i| p.log.id(i) != *id) {
                    self.toast("Commits changed — reopen the menu".into(), cx);
                    return;
                }
                let Some(r) = self.repo.as_mut() else { return };
                match r.row_disabled(*row, command) {
                    Some(why) => self.toast(why.into(), cx),
                    None => {
                        r.execute_on_row(*row, command.clone());
                        self.present_repository(cx);
                        self.poll_repository(cx);
                    }
                }
            }
            MenuAction::CompareBranch(refname) => {
                self.compare_with_current(refname.clone(), true, cx)
            }
            MenuAction::CompareWithBranch { row, id } => {
                let items = self.repo.as_ref().map_or_else(Vec::new, |r| r.branch_targets(*row))
                    .into_iter()
                    .map(|(label, command)| (label, MenuAction::RepositoryRow { row: *row, id: id.clone(), command }))
                    .collect();
                let (x, y) = self.menu.as_ref().map_or((300.0, 120.0), |m| (m.x, m.y));
                self.show_menu(Menu { x, y, items, hover: None });
            }
            MenuAction::Browse(url) => self.browse(url, cx),
            MenuAction::Spawn(args) => self.spawn_lenz(args, &root, cx),
            MenuAction::NewWindow(path) => self.new_window(path, cx),
            _ => {}
        }
        cx.notify();
    }


    /// Shared quiet refresh: local sources/history and active comparisons; no GitHub calls.
    pub(super) fn refresh_repository(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.repo.as_mut() else { return };
        r.showing(self.set.clone(), self.spec.clone());
        self.repository_command(crate::repository_session::RepositoryCommand::RefreshRepository, cx);
    }
    pub(super) fn accept_repository_refresh(&mut self,mut set:ChangeSet,spec:Spec,cx:&mut Context<Self>){
        let identity=crate::changeset::comparison_identity(&self.set,self.comparison.current)
            .map(|(group,old,path)|(group.to_string(),old.to_string(),path.to_string()));
        let anchor=self.comparison.view.anchor_line();
        self.file_actions.retain_files(&mut set);self.edits.retain_applied_files(&self.set,&mut set);self.edits.remap_files(&self.set,&set);self.comparison.remap_files(&self.set,&set);
        let index=identity.and_then(|identity|(0..set.comparisons.len()).find(|&index|
            crate::changeset::comparison_identity(&set,index)==Some((identity.0.as_str(),identity.1.as_str(),identity.2.as_str())))).unwrap_or(0);
        self.comparison.markdown_invalidate();self.markdown=super::markdown::Adapter::default();self.markdown_poll_task=None;
        self.spec=spec;self.set=Arc::new(set);self.loaded.clear();self.comparison.cache=DiffCache::default();
        self.blames.clear();self.nav=navigator::grouped_rows(&self.set.comparisons,&self.set.groups);
        self.sel=None;self.comparison.find=None;self.popup=None;self.search=None;self.switcher=None;self.menu=None;self.command_palette=None;self.diff_options=None;self.iview=ImageView::fresh();
        if self.set.comparisons.is_empty(){self.nothing=Some("No changes in this selection".into());self.comparison.current=0;let rows=self.comparison.view.viewport_rows();let mode=self.comparison.view.mode();self.comparison.view=View::new(0,0,&[],rows);self.comparison.view.set_mode(mode);self.comparison.begin();}
        else{self.nothing=None;self.comparison.current=index;self.load(index,Place::Anchor(anchor),false,cx);self.ensure_blame(cx);self.reveal_in_nav();}
        // files kept for their edits are part of what the session refreshes next
        if let Some(r)=self.repo.as_mut(){r.showing(self.set.clone(),self.spec.clone());}
        cx.notify();
    }


    /// Do what a sidebar row says, through the shared session.
    pub(super) fn source_action(&mut self, action: SourceAction, cx: &mut Context<Self>) {
        self.repository_command(crate::repository_session::RepositoryCommand::Source(action), cx);
    }

    /// Sidebar keys while it has the keyboard: ↑/↓ move, Enter picks, typing filters, Esc clears the filter.
    pub(super) fn sidebar_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.repo.as_mut().map(|r| &mut r.picker).filter(|p| p.shown && p.focus == Pane::Sidebar) else { return false;
        };
        let last = p.sources.rows(&self.palette, &p.branches).len().saturating_sub(1);
        let ks = &ev.keystroke;
        let (selected, page) = (p.sources.selected, p.sources_visible_rows.max(1.0) as usize);
        let to = |target: usize| crate::repository_session::RepositoryCommand::SourceMove(target as i32 - selected as i32);
        let command = match ks.key.as_str() {
            "up" => to(selected.saturating_sub(1)),
            "down" => to((selected + 1).min(last)),
            "home" => to(0),
            "end" => to(last),
            "pageup" => to(selected.saturating_sub(page)),
            "pagedown" => to(selected.saturating_add(page).min(last)),
            "left" | "right" if !(ks.modifiers.alt && ks.modifiers.shift) => return true,
            "enter" | "space" => {
                self.repository_command(crate::repository_session::RepositoryCommand::SourceSelect(selected), cx);
                return true;
            }
            "backspace" => {
                let mut filter = p.sources.filter.clone();
                filter.pop();
                crate::repository_session::RepositoryCommand::SourceFilter(filter)
            }
            "escape" if !p.sources.filter.is_empty() => crate::repository_session::RepositoryCommand::SourceFilter(String::new()),
            "tab" | "escape" => return false,
            _ => match Self::typed(ev) {
                Some(t) if !ks.modifiers.alt => crate::repository_session::RepositoryCommand::SourceFilter(format!("{}{t}", p.sources.filter)),
                _ => return false,
            },
        };
        self.repository_command(command, cx);
        let Some(p) = self.repo.as_mut().map(|r| &mut r.picker) else { return true };
        let sel = p.sources.selected as f32;
        let visible = (p.sources_visible_rows - 1.0).max(1.0);
        if sel < p.sources.scroll {
            p.sources.scroll = sel;
        } else if sel >= p.sources.scroll + visible {
            p.sources.scroll = sel - visible + 1.0;
        }
        cx.notify();
        true
    }

    /// Load the Change Set for the log's selection in the background and show it.
    pub(super) fn pick_change_set(&mut self, cx: &mut Context<Self>) {
        self.repository_command(crate::repository_session::RepositoryCommand::Select, cx);
    }

    /// Log keys while the log has the keyboard; false when `key` isn't one.
    pub(super) fn log_key(&mut self, key: &str, shift: bool, cmd: bool, cx: &mut Context<Self>,
    ) -> bool {
        let log_rows=self.log_rows();
        let Some(p) = self.repo.as_mut().map(|r| &mut r.picker).filter(|p| p.shown && p.focus == Pane::Log) else { return false;
        };
        if matches!(key, "left" | "right") { return true; }
        let last = p.rows().saturating_sub(1);
        let page = (log_rows as u32).saturating_sub(1).max(1);
        let target = |from: u32| match key {
            "up" => Some(from.saturating_sub(1)),
            "down" => Some((from + 1).min(last)),
            "pageup" => Some(from.saturating_sub(page)),
            "pagedown" => Some((from + page).min(last)),
            "home" => Some(0),
            "end" => Some(last),
            _ => None,
        };
        match key {
            "/" if cmd && !shift => {
                p.filter_editing = true;
                return true;
            }
            "g" if cmd && p.graph_notice => {
                self.repository_command(crate::repository_session::RepositoryCommand::WriteGraph, cx);
                return true;
            }
            "b" if cmd => {
                self.branch_menu(self.picker_w() + self.sidebar_w(), cx);
                return true;
            }
            "t" if cmd && p.compare.is_some() => {
                let (branch, three_dot) = p.compare.clone().expect("checked");
                self.compare_with_current(branch, !three_dot, cx);
                return true;
            }
            _ => {}
        }
        if cmd && key == "m" && p.merge_parents() > 1 {
            let next = p.merge_parent + 1;
            self.repository_command(crate::repository_session::RepositoryCommand::Parent(next), cx);
            return true;
        }
        let two = cmd && matches!(key, "up" | "down");
        let from = if shift || two {
            p.second.unwrap_or(p.anchor)
        } else {
            p.anchor
        };
        let Some(to) = target(from) else { return false };
        self.repository_command(crate::repository_session::RepositoryCommand::Move { delta: to as i32 - from as i32, extend: shift, two }, cx);
        // keep the moved end in view
        if let Some(p) = self.repo.as_mut().map(|r| &mut r.picker) {
            let row = p.second.unwrap_or(p.anchor) as f32;
            if row < p.scroll {
                p.scroll = row;
            } else if row >= p.scroll + log_rows - 1.0 {
                p.scroll = row - log_rows + 2.0;
            }
        }
        true
    }

    /// A click in the picker (sidebar or log); true when it was there.
    pub(super) fn picker_click(&mut self, x: f32, y: f32, ev: &MouseDownEvent, cx: &mut Context<Self>,
    ) -> bool {
        if !self.picker_shown() {
            return false;
        }
        let (pw, lh) = (self.picker_w(), self.log_h());
        let Some(p) = self.repo.as_mut().map(|r| &mut r.picker) else { return false;
        };
        if x < pw {
            let row = ((y - toolbar_h()) / line_h() - 1.0 + p.sources.scroll).floor();
            let listed = p.sources.rows(&self.palette, &p.branches).len();
            let command = if row >= 0.0 && (row as usize) < listed { crate::repository_session::RepositoryCommand::SourceSelect(row as usize) } else { crate::repository_session::RepositoryCommand::Focus(Pane::Sidebar) };
            self.repository_command(command, cx);
            return true;
        }
        let Some(r) = self.repo.as_mut() else { return false };
        if y < toolbar_h() || y >= toolbar_h() + lh {
            r.execute(crate::repository_session::RepositoryCommand::Focus(Pane::Diff));
            return false;
        }
        r.execute(crate::repository_session::RepositoryCommand::Focus(Pane::Log));
        let p = &r.picker;
        if y < toolbar_h() + line_h() {
            // the header's "vs parent" switch
            if p.merge_parents() > 1 {
                let next = p.merge_parent + 1;
                self.repository_command(crate::repository_session::RepositoryCommand::Parent(next), cx);
            }
            return true;
        }
        let row = ((y - toolbar_h()) / line_h() - 1.0 + p.scroll).floor();
        if row < 0.0 || row as u32 >= p.rows() {
            return true;
        }
        let row = row as u32;
        if ev.click_count >= 2 {
            // the first click already showed this row's changes
            self.layout_command(crate::layout::LayoutCommand::ToggleFocus, cx);
            return true;
        }
        let two = ev.modifiers.control || ev.modifiers.platform;
        self.repository_command(crate::repository_session::RepositoryCommand::SelectRow { row, extend: ev.modifiers.shift, two }, cx);
        true
    }

    /// Wheel over the picker scrolls the log; true when it was there.
    pub(super) fn picker_wheel(&mut self, x: f32, y: f32, rows: f32, cx: &mut Context<Self>,
    ) -> bool {
        let log_rows=self.log_rows();
        // the last row may scroll up to the pane's bottom edge, under its header
        if self.picker_shown() && x < self.picker_w() {
            if let Some(r) = self.repo.as_mut() {
                let visible = r.picker.sources_visible_rows - 1.0;
                r.execute(crate::repository_session::RepositoryCommand::Scroll { pane: Pane::Sidebar, rows, visible });
                cx.notify();
            }
            return true;
        }
        if !self.picker_shown() || y >= toolbar_h() + self.log_h() {
            return false;
        }
        let Some(r) = self.repo.as_mut() else { return false;
        };
        r.execute(crate::repository_session::RepositoryCommand::Scroll { pane: Pane::Log, rows, visible: log_rows - 1.0 });
        cx.notify();
        true
    }

    /// The log to paint: while a restarted log (another branch, a refresh) has no rows yet, the last frame stays up
    /// for a moment, so the history never blinks empty between a click and the first new rows. Clicks on held rows
    /// reach nothing: the new log has no commit rows yet.
    pub(super) fn shown_log_frame(&mut self) -> Option<LogFrame> {
        let restarting = self.repo.as_ref().is_some_and(crate::repository_session::RepositorySession::holding_log);
        if restarting && let Some(held) = &self.held_log { return Some(held.clone()); }
        let frame = self.log_frame();
        self.held_log = frame.clone();
        frame
    }

    /// What the log looks like now, for painting.
    pub(super) fn log_frame(&self) -> Option<LogFrame> {
        let log_rows=self.log_rows();
        let p=self.repo.as_ref().map(|r| &r.picker).filter(|p|p.shown&&p.log_shown)?;
        let first = p.scroll.floor() as u32;
        let end = first.saturating_add(log_rows.ceil() as u32).min(p.rows());
        let mut rows = Vec::new();
        for r in first..end {
            let selected = p.pane_search.current().map_or_else(||p.selected(r),|candidate|candidate==r as usize);
            let Some(i) = p.log_row(r) else {
                rows.push(LogRow { col: None, lane_free: false, labels: Vec::new(), subject: "Uncommitted changes".into(), author: String::new(), sha: String::new(), date: String::new(), selected,
                });
                continue;
            };
            let labels = p.refs.as_ref().map_or(Vec::new(), |refs| {
                refs.at(&p.log.id(i))
                    .iter()
                    .map(|rf| {
                        let color = match rf.kind {
                            history::RefKind::Head => 0xE0C46C,
                            history::RefKind::Branch => 0x6A9FD8,
                            history::RefKind::Remote => 0x9876AA,
                            history::RefKind::Tag => 0x6A8759,
                        };
                        (rf.name.clone(), color)
                    })
                    .collect()
            });
            let mut subject=p.log.subject(i).to_string();
            let query=p.pane_search.query();
            if !query.is_empty() {
                let id=p.log.id(i);
                let email=p.log.author(i).1;
                // Expose otherwise hidden metadata when it is what matched.
                if crate::pane_search::contains(email,query) {subject=format!("{email} · {subject}");}
                if crate::pane_search::contains(&id,query) {subject=format!("{id} · {subject}");}
            }
            rows.push(LogRow {
                col: p.matches.is_none().then(|| p.log.column(i)),
                lane_free: p.matches.is_some(),
                labels,
                subject,
                author: p.log.author(i).0.to_string(),
                sha: p.log.id(i).chars().take(8).collect(),
                date: p.log.stamp(i),
                selected,
            });
        }
        // graph edges for the log rows in view, shifted by the Uncommitted row (no graph while filtered)
        let edges = if p.matches.is_some() {
            Vec::new()
        } else {
            p.log.edges(first.saturating_sub(1)..end.saturating_sub(1)).into_iter().map(|e| (e, 1.0)).collect()
        };
        let mut status = match (&p.error, p.loading) {
            (Some(e), _) => e.clone(),
            (None, true) => format!("History — {} commits, loading…", p.log.len()),
            (None, false) => match p.more_label() {
                Some(more) => format!("History — {more}"),
                None => format!("History — {} commits", p.log.len()),
            },
        };
        if let Some((path, _)) = &p.file_paths {
            status = format!("History of {path} — {} commits   (Esc to leave)", p.rows() - 1);
        }
        if let Some((branch, three_dot)) = &p.compare {
            let short = branch.trim_start_matches("refs/heads/").trim_start_matches("refs/remotes/");
            let how = if *three_dot { "what this branch adds since splitting off (Ctrl/Cmd+T: tip ⟷ tip)" } else { "tip ⟷ tip (Ctrl/Cmd+T: since splitting off)" };
            status = format!("Comparing with {short}: {how}   ·   {} commits on one side only", p.rows() - 1);
        }
        if let Some(pr) = self.pr_status().filter(|_| p.compare.is_none() && p.file_paths.is_none()) {
            status = pr;
        }
        if p.filter_editing || !p.filter.is_empty() {
            let shown = p.matches.as_ref().map_or(p.log.len() as usize, Vec::len);
            let cursor = if p.filter_editing { "▏" } else { "" };
            status = format!("Filter: {}{cursor}   {shown} of {} commits   (author: path: after: before:, Enter searches messages, Esc clears)", p.filter, p.log.len());
        }
        if p.graph_notice {
            status += "   ·   slow without a commit-graph file — Ctrl/Cmd+G: Speed up history (git commit-graph write)";
        }
        let walking = match &p.branches {
            Branches::All => "all".to_string(),
            Branches::Current => "current".to_string(),
            Branches::One(r) => r.trim_start_matches("refs/heads/").trim_start_matches("refs/remotes/").to_string(),
        };
        if let Some(t) = self.repo.as_ref().and_then(|r| r.selection_title()) { status += &format!("   ·   {}", t.text); }
        status += &format!("   ·   branches: {walking} (Ctrl/Cmd+B)");
        if p.merge_parents() > 1 {
            status += &format!("   ·   merge: vs parent {} of {} (Ctrl/Cmd+M or click to switch)", p.merge_parent + 1, p.merge_parents());
        }
        if p.pane_search.active() {
            status=p.pane_search.status();
            if p.pane_search.searching(p.rows() as usize) || p.loading {
                status=status.replace("No matches","0 matches so far");
                status.push_str(" · searching…");
            }
        }
        let focused = p.focus == Pane::Log;
        Some(LogFrame { height:self.log_h(), rows, edges, status, focused, scroll: p.scroll, char_w: self.char_w, query:p.pane_search.query().to_owned(),
        })
    }
}

#[derive(Clone)]
pub(super) struct LogRow {
    col: Option<u32>,
    /// A filtered row: no graph, subject from the left.
    lane_free: bool,
    labels: Vec<(String, u32)>,
    pub(super) subject: String,
    author: String,
    /// The commit's id, shortened.
    sha: String,
    /// Date and time in the committer's zone.
    date: String,
    selected: bool,
}

#[derive(Clone)]
pub(super) struct LogFrame {
    query: String,
    height: f32,
    pub(super) rows: Vec<LogRow>,
    /// Edges, and the offset (in rows) from a log row to its display position.
    edges: Vec<(history::Edge, f32)>,
    pub(super) status: String,
    focused: bool,
    scroll: f32,
    char_w: f32,
}

/// Author, short id and date columns' x, each dropped as the log narrows (date last).
fn log_metadata_columns(width:f32)->(Option<f32>,Option<f32>,Option<f32>) {
    ((width>=800.0).then_some(width-400.0),(width>=640.0).then_some(width-220.0),(width>=540.0).then_some(width-140.0))
}

#[test]
fn narrow_log_reserves_room_for_the_subject() {
    assert_eq!(log_metadata_columns(400.0),(None,None,None));
    assert_eq!(log_metadata_columns(600.0),(None,None,Some(460.0)));
    assert_eq!(log_metadata_columns(700.0),(None,Some(480.0),Some(560.0)));
    assert_eq!(log_metadata_columns(900.0),(Some(500.0),Some(680.0),Some(760.0)));
}

/// Paint the log at `(x0, y0)`, `w` wide: header, graph, ref labels, subject, author, date.
#[allow(clippy::too_many_arguments)] // Palette plus adapter layout inputs.
pub(super) fn paint_log(palette: &theme::Palette, window: &mut Window, cx: &mut App, font: &Font, x0: f32, y0: f32, w: f32, frame: &LogFrame,
) {
    let char_w = frame.char_w;
    let h = frame.height;
    let p = palette;
    fill(window, x0, y0, w, h, p.bg);
    fill(window, x0, y0, w, line_h(), p.header_bg);
    text(window, cx, font, x0 + 8.0, y0, &frame.status, if frame.focused { p.fg } else { p.dim },
    );
    let body = Bounds { origin: point(px(x0), px(y0 + line_h())), size: size(px(w), px(h - line_h())),
    };
    let top = y0 + line_h();
    let first = frame.scroll.floor();
    let row_y = |display: f32| top + (display - frame.scroll) * line_h();
    let lanes = frame.rows.iter().filter_map(|r| r.col).max().unwrap_or(0).max(frame.edges.iter().map(|(e, _)| e.lane).max().unwrap_or(0));
    let graph_w = ((lanes + 1) as f32 * LANE_W + 12.0).min(w / 3.0);
    window.with_content_mask(Some(ContentMask { bounds: body }), |window| {
        for (k, row) in frame.rows.iter().enumerate() {
            let y = row_y(first + k as f32);
            if row.selected {
                fill(window, x0, y, w, line_h(), p.selected_bg);
            }
        }
        // graph
        let lx = |lane: u32| x0 + 10.0 + lane as f32 * LANE_W;
        let mid = |log_row: u32, off: f32| row_y(log_row as f32 + off) + line_h() / 2.0;
        window.with_content_mask(Some(ContentMask { bounds: Bounds { origin: point(px(x0), px(top)), size: size(px(graph_w), px(h - line_h())),
                },
            }), |window| {
            for (e, off) in &frame.edges {
                let color = Hsla::from(rgb([0x6A9FD8, 0x629755, 0xCC7832, 0x9876AA, 0xE0C46C, 0x20999D, 0xD25252,
                    ][e.lane as usize % 7]));
                let (cx0, cy) = (lx(e.child_col), mid(e.child, *off));
                let mut path = PathBuilder::stroke(px(1.5));
                if e.stub {
                    // long edges: a short arrow at each end
                    path.move_to(point(px(cx0), px(cy)));
                    path.line_to(point(px(lx(e.lane)), px(cy + line_h() * 0.8)));
                    if let Some(pr) = e.parent {
                        let py = mid(pr, *off);
                        path.move_to(point(px(lx(e.lane)), px(py - line_h() * 0.8)));
                        path.line_to(point(px(lx(e.parent_col)), px(py)));
                    }
                } else {
                    path.move_to(point(px(cx0), px(cy)));
                    match e.parent {
                        Some(pr) if pr == e.child + 1 => {
                                path.line_to(point(px(lx(e.parent_col)), px(mid(pr, *off))))
                            }
                            Some(pr) => {
                            let py = mid(pr, *off);
                            path.line_to(point(px(lx(e.lane)), px(cy + line_h())));
                            path.line_to(point(px(lx(e.lane)), px(py - line_h())));
                            path.line_to(point(px(lx(e.parent_col)), px(py)));
                        }
                        None => {
                            path.line_to(point(px(lx(e.lane)), px(cy + line_h())));
                            path.line_to(point(px(lx(e.lane)), px(top + h)));
                        }
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
            for (k, row) in frame.rows.iter().enumerate() {
                if let Some(col) = row.col {
                    let (cx0, cy) = (lx(col), row_y(first + k as f32) + line_h() / 2.0);
                    window.paint_quad(quad(
                        Bounds { origin: point(px(cx0 - 4.5), px(cy - 4.5)), size: size(px(9.0), px(9.0)),
                            },
                        px(4.5),
                        Hsla::from(rgb(p.fg)),
                        px(1.5),
                        Hsla::from(rgb(p.bg)),
                        Default::default(),
                    ));
                }
            }
        },
        );
        // labels, subject, author, date
        for (k, row) in frame.rows.iter().enumerate() {
            let y = row_y(first + k as f32);
            let mut tx = x0 + graph_w;
            if row.col.is_none() {
                tx = x0 + 10.0;
            }
            if row.lane_free {
                tx = x0 + 10.0;
            }
            let (author_x,sha_x,date_x)=log_metadata_columns(w);
            let subject_end=x0+author_x.or(sha_x).or(date_x).unwrap_or(w);
            for (name, color) in &row.labels {
                let lw = name.chars().count() as f32 * char_w + 10.0;
                if tx+lw+4.0>subject_end-120.0 {break;}
                fill(window, tx, y + 3.0, lw, line_h() - 6.0, p.header_bg);
                text(window, cx, font, tx + 5.0, y, name, *color);
                tx += lw + 4.0;
            }
            window.with_content_mask(Some(clip(body, tx, (subject_end - tx - 8.0).max(0.0))), |window| {
                let found:Vec<_>=crate::pane_search::ranges(&row.subject,&frame.query).into_iter().map(|r|(r,p.found_bg)).collect();
                colored_text(p,window,cx,font,tx,y,&row.subject,&[(0..row.subject.len(),if row.col.is_none() && !row.lane_free {p.status[2]} else {p.fg})],&found);
                },
            );
            if let Some(author_x)=author_x.map(|x|x0+x) {window.with_content_mask(Some(clip(body, author_x, 162.0)), |window| {
                let found:Vec<_>=crate::pane_search::ranges(&row.author,&frame.query).into_iter().map(|r|(r,p.found_bg)).collect();
                colored_text(p,window,cx,font,author_x,y,&row.author,&[(0..row.author.len(),p.dim)],&found);
            });}
            if let Some(sha_x)=sha_x {text(window, cx, font, x0+sha_x, y, &row.sha, p.dim);}
            if let Some(date_x)=date_x {text(window, cx, font, x0+date_x, y, &row.date, p.dim);}
        }
    });
    fill(window, x0, y0 + h - 1.0, w, 1.0, p.border);
}

use crate::repository_session::github_url;

/// The sidebar as painted: its rows, scroll, selection, filter and whether it has the keyboard.
pub(super) struct SidebarFrame {
    rows: Vec<SourceRow>,
    scroll: f32,
    selected: usize,
    filter: String,
    focused: bool,
}

impl Viewer {
    pub(super) fn sidebar_frame(&mut self, h: f32) -> Option<SidebarFrame> {
        let p = self.repo.as_mut().map(|r| &mut r.picker).filter(|p| p.shown && p.sidebar)?;
        p.sources_visible_rows = (h / line_h() - 1.0).max(1.0);
        Some(SidebarFrame {
            rows: p.sources.rows(&self.palette, &p.branches),
            scroll: p.sources.scroll,
            selected: p.sources.selected,
            filter: p.sources.filter.clone(),
            focused: p.focus == Pane::Sidebar,
        })
    }
}

/// Paint the picker sidebar: a header (with the filter while typing), then sections and sources.
#[allow(clippy::too_many_arguments)] // Palette plus adapter layout inputs.
pub(super) fn paint_picker_sidebar(palette: &theme::Palette, window: &mut Window, cx: &mut App, font: &Font, x0: f32, y0: f32, h: f32, picker_w: f32, f: &SidebarFrame,
) {
    let p = palette;
    fill(window, x0, y0, picker_w, h, p.sidebar_bg);
    fill(window, x0, y0, picker_w, line_h(), p.header_bg);
    let title = if f.filter.is_empty() { "Repository — type to filter".to_string() } else { format!("Filter: {}▏", f.filter) };
    window.with_content_mask(Some(ContentMask{bounds:Bounds{origin:point(px(x0),px(y0)),size:size(px(picker_w),px(line_h()))}}),|window| {
        text(window, cx, font, x0 + 8.0, y0, &title, if f.focused { p.fg } else { p.dim });
    });
    let list = Bounds { origin: point(px(x0), px(y0 + line_h())), size: size(px((picker_w - 1.0).max(0.0)), px((h - line_h()).max(0.0))),
    };
    window.with_content_mask(Some(ContentMask { bounds: list }), |window| {
        let first = f.scroll.floor() as usize;
        for (i, row) in f.rows.iter().enumerate().skip(first).take((h / line_h()) as usize + 1) {
            let y = y0 + line_h() + (i as f32 - f.scroll) * line_h();
            if row.current || (f.focused && i == f.selected) {
                fill(window, x0, y, picker_w, line_h(), p.selected_bg);
            }
            text(window, cx, font, x0 + 8.0 + row.indent, y, &row.text, row.color,
            );
        }
    });
    fill(window, x0 + picker_w - 1.0, y0, 1.0, h, p.border);
}

#[cfg(all(test,feature="gui-test"))]
mod refresh_gui_tests {
    use super::*;
    use gpui::{AppContext, Entity, TestAppContext};
    use super::super::merge_view::gui_tests::event;
    fn git(root:&std::path::Path,args:&[&str])->String{
        let output=std::process::Command::new("git").env("GIT_CONFIG_GLOBAL","/dev/null").env("GIT_CONFIG_NOSYSTEM","1")
            .args(["-c","user.name=Refresh Author","-c","user.email=refresh@example.invalid","-c","commit.gpgsign=false"])
            .args(args).current_dir(root).output().unwrap();
        assert!(output.status.success(),"git {args:?}: {}",String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().into()
    }
    fn fixture(cx:&mut TestAppContext)->(tempfile::TempDir,Entity<Viewer>){
        cx.skip_drawing();
        let dir=tempfile::Builder::new().prefix(".lenz-gui-refresh-").tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let root=dir.path();git(root,&["init","-q","-b","main"]);
        for name in ["a.txt","m.txt","z.txt"]{std::fs::write(root.join(name),"base\nkeep\nkeep\nkeep\nkeep\n").unwrap();}
        git(root,&["add","."]);git(root,&["commit","-qm","Refresh base"]);
        std::fs::write(root.join("history"),"history\n").unwrap();git(root,&["add","history"]);git(root,&["commit","-qm","Refresh second"]);
        for name in ["m.txt","z.txt"]{std::fs::write(root.join(name),"changed\nkeep\nkeep\nkeep\nkeep\n").unwrap();}
        let spec=crate::repository_session::local_spec();let set=Arc::new(crate::repo::local_changes(root,false).unwrap());
        let doc=Arc::new(Doc::load(&set,0,&root.join("m.txt"),&root.join("m.txt"),false).unwrap());
        let view=doc.view(&doc.line_diff(&Options::default()),3.0);
        let mut picker=Picker::new(root.into(),true);picker.sources=Sources::read_repository(root);picker.refs=history::refs(root).ok();
        picker.start_stream();let started=Instant::now();while picker.loading{picker.poll_stream(8.0);assert!(started.elapsed().as_secs()<10);std::thread::sleep(std::time::Duration::from_millis(2));}
        let viewer=cx.new(|cx|{let mut v=Viewer::new(set,spec,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);v.repo = Some(crate::repository_session::RepositorySession::with_picker(picker));v});
        (dir,viewer)
    }
    fn pump(cx:&mut TestAppContext,viewer:&Entity<Viewer>,done:impl Fn(&Viewer)->bool){
        let started=Instant::now();loop{cx.executor().run_until_parked();cx.executor().advance_clock(std::time::Duration::from_millis(35));
            if viewer.read_with(cx,|v,_|done(v)){break;}
            assert!(started.elapsed().as_secs()<15,"GUI refresh did not settle");std::thread::sleep(std::time::Duration::from_millis(2));}
    }
    fn focus_diff(v:&mut Viewer,w:&mut Window,cx:&mut Context<Viewer>) {
        for _ in 0..8 {
            if !v.nav_focused && !v.panel_focused && v.repo.as_ref().map(|r| &r.picker).is_none_or(|p|!p.shown || p.focus==Pane::Diff) {return;}
            v.on_key(&event("tab",false,false,false),w,cx);
        }
        panic!("comparison pane did not receive keyboard focus");
    }
    #[test]
    fn gui_log_rows_show_short_id_and_commit_date_with_time(){
        let mut cx=TestAppContext::single();let (_dir,viewer)=fixture(&mut cx);
        viewer.read_with(&cx,|v,_|{
            let frame=v.log_frame().expect("log shown");let p=&v.repo.as_ref().unwrap().picker;
            let row=frame.rows.iter().find(|r|!r.sha.is_empty()).expect("a commit row");
            assert_eq!(row.sha.len(),8);assert!((0..p.log.len()).any(|i|p.log.id(i).starts_with(&row.sha)));
            let (date,time)=row.date.split_once(' ').expect("date and time");
            assert_eq!(date.len(),10);assert_eq!(time.len(),5);assert_eq!(&time[2..3],":");
        });
    }

    #[test]
    fn gui_repository_refresh_real_menu_updates_history_tags_and_local_files_preserving_controls(){
        let mut cx=TestAppContext::single();let (dir,viewer)=fixture(&mut cx);let root=dir.path();
        viewer.update(&mut cx,|v,cx|{v.open(1,Place::Anchor(2.0),cx);let p=v.repo.as_mut().map(|r| &mut r.picker).unwrap();p.set_filter("author:Refresh".into());p.sources.filter="main".into();p.sources.collapsed.insert("Branches");p.sources.show_all_branches=true;});
        pump(&mut cx,&viewer,|v|v.doc().is_some()&&v.comparison.current==1);
        let anchor=viewer.read_with(&cx,|v,_|v.comparison.view.anchor_line());
        std::fs::write(root.join("new-commit"),"commit\n").unwrap();git(root,&["add","new-commit"]);git(root,&["commit","-qm","Refresh third"]);git(root,&["tag","after-refresh"]);
        std::fs::write(root.join("a.txt"),"new tracked change\n").unwrap();std::fs::write(root.join("new-local"),"untracked\n").unwrap();
        let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
        window.update(&mut cx,|v,w,cx|v.on_key(&event("b",true,false,false),w,cx)).unwrap();
        viewer.read_with(&cx,|v,_|assert!(matches!(v.menu.as_ref().unwrap().items[0].1,MenuAction::RepositoryRefresh)));
        viewer.update(&mut cx,|v,_|v.repo.as_mut().map(|r| &mut r.picker).unwrap().focus=Pane::Sidebar);
        window.update(&mut cx,|v,w,cx|{v.on_key(&event("home",false,false,false),w,cx);v.on_key(&event("enter",false,false,false),w,cx);}).unwrap();
        pump(&mut cx,&viewer,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|!p.refreshing()&&!p.loading)&&v.doc().is_some()&&v.set.comparisons.len()==4);
        viewer.read_with(&cx,|v,_|{
            let p=v.repo.as_ref().map(|r| &r.picker).unwrap();assert_eq!(v.set.comparisons[v.comparison.current].path,"z.txt");assert_eq!(v.comparison.current,2);
            assert!((v.comparison.view.anchor_line()-anchor).abs()<0.01);assert_eq!(p.filter,"author:Refresh");assert_eq!(p.sources.filter,"main");
            assert!(p.sources.collapsed.contains("Branches")&&p.sources.show_all_branches);assert_eq!(p.focus,Pane::Sidebar);
            assert_eq!(p.sources.local,4);assert!(p.sources.tags.iter().any(|tag|tag.name=="after-refresh"));
            assert!((0..p.log.len()).any(|row|p.log.subject(row)=="Refresh third"));assert!(v.set.comparisons.iter().any(|c|c.path=="new-local"));
            // the session's report, as in the terminal
            assert_eq!(v.toast.as_ref().map(|t|t.0.as_str()),Some("Repository refreshed"));
        });
    }
    #[test]
    fn gui_show_in_graph_rejects_a_held_follow_result_and_keeps_full_log(){
        let mut cx=TestAppContext::single();let (_dir,viewer)=fixture(&mut cx);
        let (request,result,selected)=viewer.update(&mut cx,|v,_|{
            let p=v.repo.as_mut().map(|r| &mut r.picker).unwrap();
            let previous=p.begin_file_history("z.txt".into());let result=previous.read_followed();
            assert!(matches!(p.accept_file_history(&previous,result),crate::repository_session::HistoryAcceptance::Followed{pick:true}));
            assert!(p.matches.is_some());
            let request=p.begin_file_history("a.txt".into());let result=request.read_followed();
            let selected=p.log.id(p.log_row(p.anchor).unwrap());(request,result,(p.anchor,selected))
        });
        let (row,selected)=selected;
        viewer.update(&mut cx,|v,cx|v.picker_menu_action(&MenuAction::RepositoryRow{row,id:selected.clone(),command:crate::repository_session::RepositoryCommand::ShowInGraph},cx));
        viewer.update(&mut cx,|v,_|{
            let p=v.repo.as_mut().map(|r| &mut r.picker).unwrap();
            assert!(matches!(p.accept_file_history(&request,result),crate::repository_session::HistoryAcceptance::Stale));
            assert!(p.file_paths.is_none()&&p.matches.is_none());assert_eq!(p.log.id(p.log_row(p.anchor).unwrap()),selected);
        });
    }
    #[test]
    fn gui_follow_requests_use_latest_path_and_filter_generation(){
        let mut cx=TestAppContext::single();let (_dir,viewer)=fixture(&mut cx);
        viewer.update(&mut cx,|v,cx|{v.show_file_history("a.txt".into(),cx);v.show_file_history("z.txt".into(),cx);});
        pump(&mut cx,&viewer,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.file_paths.as_ref().is_some_and(|(path,_)|path=="z.txt"))
            &&v.doc().is_some()&&v.set.comparisons.len()==1&&v.set.comparisons[0].path=="z.txt");
        viewer.read_with(&cx,|v,_|{
            let p=v.repo.as_ref().map(|r| &r.picker).unwrap();assert_eq!(p.file_paths.as_ref().unwrap().0,"z.txt");
            let Spec::Git{paths,..}=&v.spec else{panic!("followed comparison is Git");};assert_eq!(paths,&["z.txt"]);
        });
        let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
        viewer.update(&mut cx,|v,cx|{
            v.show_file_history("a.txt".into(),cx);
            let p=v.repo.as_mut().map(|r| &mut r.picker).unwrap();p.set_filter("Refresh base".into());p.filter_editing=true;
        });
        window.update(&mut cx,|v,w,cx|v.on_key(&event("enter",false,false,false),w,cx)).unwrap();
        pump(&mut cx,&viewer,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.grep_ids.is_some()));
        viewer.read_with(&cx,|v,_|{let p=v.repo.as_ref().map(|r| &r.picker).unwrap();assert_eq!(p.file_paths.as_ref().unwrap().0,"z.txt");assert_eq!(p.filter,"Refresh base");});
    }
    #[test]
    fn gui_empty_repository_refresh_renders_and_command_menu_can_refresh_new_changes(){
        let mut cx=TestAppContext::single();let (dir,viewer)=fixture(&mut cx);
        let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
        for name in ["m.txt","z.txt"]{std::fs::write(dir.path().join(name),"base\nkeep\nkeep\nkeep\nkeep\n").unwrap();}
        viewer.update(&mut cx,|v,cx|v.refresh_repository(cx));
        pump(&mut cx,&viewer,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|!p.refreshing()&&!p.loading)&&v.set.comparisons.is_empty());
        window.update(&mut cx,|v,w,cx|{let _=v.render(w,cx);focus_diff(v,w,cx);v.on_key(&event(":",false,false,false),w,cx);}).unwrap();
        let index=viewer.read_with(&cx,|v,_|v.command_palette.as_ref().expect("palette opens").rows().position(|r|r.entry.action==crate::catalog::Action::Repository(crate::catalog::RepoAction::Refresh)).expect("empty repository keeps Refresh available"));
        std::fs::write(dir.path().join("m.txt"),"new after empty\n").unwrap();
        window.update(&mut cx,|v,w,cx|{v.on_key(&event("home",false,false,false),w,cx);for _ in 0..index{v.on_key(&event("down",false,false,false),w,cx);}v.on_key(&event("enter",false,false,false),w,cx);}).unwrap();
        pump(&mut cx,&viewer,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|!p.refreshing()&&!p.loading)&&v.doc().is_some()&&v.set.comparisons.len()==1);
        viewer.read_with(&cx,|v,_|{assert!(v.nothing.is_none());assert_eq!(v.set.comparisons[0].path,"m.txt");assert_eq!(v.doc().unwrap().text(Side::Right),"new after empty\n");});
    }
    #[test]
    fn gui_repository_reload_refreshes_sources_and_retains_applied_file_for_safe_undo(){
        let mut cx=TestAppContext::single();let (dir,viewer)=fixture(&mut cx);
        viewer.update(&mut cx,|v,cx|v.open(1,Place::FirstChange,cx));pump(&mut cx,&viewer,|v|v.doc().is_some()&&v.comparison.current==1);
        let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
        window.update(&mut cx,|v,w,cx|{focus_diff(v,w,cx);v.on_key(&event("right",false,true,true),w,cx);}).unwrap();
        pump(&mut cx,&viewer,|v|v.doc().is_some()&&!v.edits.applied(v.comparison.current).is_empty());
        window.update(&mut cx,|v,w,cx|v.on_key(&event("s",true,false,false),w,cx)).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("z.txt")).unwrap(),"base\nkeep\nkeep\nkeep\nkeep\n");
        std::fs::write(dir.path().join("a.txt"),"new tracked change\n").unwrap();
        viewer.update(&mut cx,|v,cx|v.reload(cx));
        pump(&mut cx,&viewer,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|!p.refreshing()&&!p.loading)&&v.doc().is_some()&&v.set.comparisons.iter().any(|c|c.path=="a.txt"));
        viewer.read_with(&cx,|v,_|{assert_eq!(v.set.comparisons[v.comparison.current].path,"z.txt");assert_eq!(v.edits.applied(v.comparison.current).len(),1);});
        window.update(&mut cx,|v,w,cx|v.on_key(&event("z",true,false,false),w,cx)).unwrap();
        pump(&mut cx,&viewer,|v|v.doc().is_some());
        window.update(&mut cx,|v,w,cx|v.on_key(&event("s",true,false,false),w,cx)).unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("z.txt")).unwrap(),"changed\nkeep\nkeep\nkeep\nkeep\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),"new tracked change\n");
    }
}
