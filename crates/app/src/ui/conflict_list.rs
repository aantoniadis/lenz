//! The Conflict List: the sidebar's **Conflicts** entry and the File Navigator listing the Conflicted Files of the
//! merge / rebase / cherry-pick / revert / stash pop in progress, each resolved in the Merge View, marked resolved on
//! request, re-read after every Mark resolved and whenever the window comes back.

use super::{
    merge_view::MergeState,
    picker::{ConflictsEntry, SourceAction},
    *,
};
use crate::conflict_session::Opening;
use crate::conflict_session::tag;
pub use crate::conflict_session::{
    Snapshot, State, change_set, entry_text, from_top, next_unresolved, read, states,
};

/// The Repository Window's Conflict List.
pub(super) struct ConflictList {
    root: PathBuf,
    /// `lenz merge <path>…`: only these.
    filter: Vec<String>,
    snap: Snapshot,
    /// As last grouped, in list order.
    states: Vec<(String, State)>,
    /// Kind tags of the files seen this session (a marked file's stages are gone).
    tags: HashMap<String, String>,
    /// The Merge Views of files left this session, unsaved work and all.
    merges: HashMap<String, MergeState>,
    /// The Conflict List is the Change Set shown.
    shown: bool,
    /// Mark resolved found markers in this file and waits for a second Mark resolved.
    confirm: Option<String>,
    /// Closing the window was asked once with unsaved Merge Views.
    close_asked: bool,
}

impl ConflictList {
    pub(super) fn confirming(&self)->bool{self.confirm.is_some()}
    fn new(root: PathBuf, filter: Vec<String>, snap: Snapshot) -> Self {
        let mut cl = ConflictList {
            root,
            filter,
            snap: snap.clone(),
            states: Vec::new(),
            tags: HashMap::new(),
            merges: HashMap::new(),
            shown: false,
            confirm: None,
            close_asked: false,
        };
        cl.update(snap, &HashMap::new());
        cl
    }

    fn state(&self, path: &str) -> State {
        self.states
            .iter()
            .find(|s| s.0 == path)
            .map_or(State::Marked, |s| s.1)
    }

    fn left(&self) -> usize {
        self.snap.files.len()
    }

    /// Take in a new snapshot; whether the list's rows changed.
    fn update(&mut self, snap: Snapshot, unsaved: &HashMap<String, usize>) -> bool {
        for f in &snap.files {
            self.tags.insert(f.path.clone(), tag(f));
        }
        let states = states(&snap, unsaved);
        let changed = states != self.states;
        (self.snap, self.states) = (snap, states);
        changed
    }
}

impl Viewer {
    /// Open on the Conflict List of the repository the picker shows (`lenz merge`, or plain `lenz` during an
    /// operation).
    pub fn start_conflicts(&mut self, snap: Snapshot, filter: Vec<String>, cx: &mut Context<Self>) {
        let Some(root) = self.repo.as_ref().map(|r| &r.picker).map(|p| p.root.clone()) else {
            return;
        };
        self.conflicts = Some(ConflictList::new(root, filter, snap));
        self.show_conflicts(cx);
    }

    /// Conflicts left in the files open with unsaved changes.
    fn unsaved(&self) -> HashMap<String, usize> {
        let stashed = self.conflicts.iter().flat_map(|c| c.merges.values());
        self.merge
            .iter()
            .chain(stashed)
            .filter(|m| m.dirty)
            .map(|m| (m.path.clone(), m.m.unresolved()))
            .collect()
    }

    /// The sidebar's Conflicts entry, as the list stands.
    pub(super) fn update_entry(&mut self) {
        let entry = self.conflicts.as_ref().map(|cl| {
            let (left, total) = (cl.left(), cl.snap.files.len() + cl.snap.marked.len());
            let copy = cl
                .snap
                .op
                .operation
                .continue_command()
                .filter(|_| left == 0)
                .map(str::to_string);
            ConflictsEntry {
                lines: entry_text(&cl.snap.op, left, total),
                copy,
                shown: cl.shown,
            }
        });
        if let Some(p) = self.repo.as_mut().map(|r| &mut r.picker) {
            p.sources.conflicts = entry;
        }
    }

    /// Show the Conflict List, keeping the file shown if it's listed, else the first unresolved one.
    pub(super) fn show_conflicts(&mut self, cx: &mut Context<Self>) {
        if !self.edits_may_close(cx) {return;}
        let Some(cl) = self.conflicts.as_ref() else {
            return;
        };
        let keep = (cl.shown && self.comparison.current < self.set.comparisons.len())
            .then(|| self.set.comparisons[self.comparison.current].path.clone());
        let set = change_set(&cl.root, &cl.states, &cl.snap);
        let empty = cl.states.is_empty();
        let continue_with = cl
            .snap
            .op
            .operation
            .continue_command()
            .unwrap_or("git status");
        self.leave_conflicts();
        match set {
            Ok(set) if !empty => {
                self.spec = Spec::Git {
                    cached: false,
                    revs: vec!["HEAD".into()],
                    paths: Vec::new(),
                    untracked: false,
                };
                self.show_change_set(set, cx);
            }
            Ok(_) => {
                self.nav = Vec::new();
                self.nothing = Some(format!("Nothing to resolve — run {continue_with}"));
            }
            Err(e) => self.toast(e, cx),
        }
        let Some(cl) = self.conflicts.as_mut() else {
            return;
        };
        if empty {
            self.update_entry();
            return;
        }
        cl.shown = true;
        self.layout.set_files(true);
        let order: Vec<(&str, State)> = self
            .set
            .comparisons
            .iter()
            .map(|c| (c.path.as_str(), cl.state(&c.path)))
            .collect();
        let at = keep
            .and_then(|k| order.iter().position(|o| o.0 == k))
            .or_else(|| next_unresolved(&order, None, true))
            .unwrap_or(0);
        self.comparison.current = usize::MAX; // so open() takes it as new
        self.open(at, Place::Top, cx);
        self.update_entry();
        cx.notify();
    }

    /// Another Change Set is about to show: the Merge View shown goes back to the session's.
    pub(super) fn leave_conflicts(&mut self) {
        let Some(cl) = self.conflicts.as_mut().filter(|c| c.shown) else {
            return;
        };
        if let Some(mut m) = self.merge.take() {
            m.release_syntax();
            cl.merges.insert(m.path.clone(), m);
        }
        // the Version Picker belongs to a Conflicted File: it goes with the list
        self.picker_v = None;
        cl.shown = false;
        cl.confirm = None;
        self.update_entry();
    }

    /// The file just opened from the Conflict List: its Merge View from this session, or from its index stages;
    /// a file resolved whole and a marked file show their diff against HEAD (the former under a banner for now).
    pub(super) fn conflict_open(&mut self, cx: &mut Context<Self>) {
        let Some(cl) = self.conflicts.as_mut().filter(|c| c.shown) else {
            return;
        };
        let Some(path) = self
            .set
            .comparisons
            .get(self.comparison.current)
            .map(|c| c.path.clone())
        else {
            return;
        };
        if self.merge.as_ref().is_some_and(|m| m.path == path)
            || self.picker_v.as_ref().is_some_and(|p| p.o.path == path)
        {
            return;
        }
        if let Some(mut m) = self.merge.take() {
            m.release_syntax();
            cl.merges.insert(m.path.clone(), m);
        }
        // ponytail: a version taken but not saved is dropped on switching files; keep pickers like merges if missed
        self.picker_v = None;
        self.comparison.banner = None;
        cl.confirm = None;
        if let Some(m) = cl.merges.remove(&path) {
            self.merge = Some(m);
            return;
        }
        let Some(file) = cl.snap.file(&path).cloned() else {
            return;
        };
        let root = cl.root.clone();
        // reading the stages and merging can take a moment: off the UI thread, shown if the file is still the one open
        cx.spawn(async move |this, cx| {
            let opened = cx
                .background_executor()
                .spawn(async move { crate::conflict_session::open_file(&root, &file) })
                .await;
            this.update(cx, |v: &mut Viewer, cx| {
                let still = v.conflicts.as_ref().is_some_and(|c| c.shown)
                    && v.set
                        .comparisons
                        .get(v.current)
                        .is_some_and(|c| c.path == path);
                if !still || v.merge.is_some() || v.picker_v.is_some() {
                    return;
                }
                match opened {
                    Ok(Opening::Merge(o)) => {
                        v.merge = Some(MergeState::new(*o, v.view.viewport_rows()))
                    }
                    Ok(Opening::Picker(o)) => {
                        v.picker_v = Some(version_picker::PickerState::new(*o, false))
                    }
                    Err(e) => v.banner = Some(e),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Done saved the file: on to the next unresolved one.
    pub(super) fn conflict_done(&mut self, cx: &mut Context<Self>) {
        if !self.cross_to_unresolved(true, cx) {
            self.toast(
                "No unresolved files left — Mark resolved stages the saved ones".into(),
                cx,
            );
        }
        self.refresh_conflicts(cx);
    }

    /// Cancel: the file's Merge View closes, unsaved changes dropped; its diff against HEAD shows beneath.
    pub(super) fn conflict_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(m) = self.merge.take()
            && let Some(cl) = self.conflicts.as_mut()
        {
            cl.merges.remove(&m.path);
        }
        self.toast(
            "Merge View closed — click the file to open it again".into(),
            cx,
        );
        self.refresh_conflicts(cx);
    }

    /// Open the next (or previous) unresolved file of the Conflict List; false when there is none.
    pub(super) fn cross_to_unresolved(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let Some(cl) = self.conflicts.as_ref().filter(|c| c.shown) else {
            return false;
        };
        let order: Vec<(&str, State)> = self
            .set
            .comparisons
            .iter()
            .map(|c| (c.path.as_str(), cl.state(&c.path)))
            .collect();
        let from = self
            .set
            .comparisons
            .get(self.comparison.current)
            .map(|c| c.path.as_str());
        let Some(i) = next_unresolved(&order, from, forward) else {
            return false;
        };
        let path = order[i].0.to_string();
        self.open(i, Place::Top, cx);
        self.toast(format!("Next unresolved file: {path}"), cx);
        true
    }

    /// The Merge View's Mark resolved button.
    pub(super) fn mark_current_resolved(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.merge.as_ref().map(|m| m.path.clone()) {
            self.mark_resolved(path, cx);
        }
    }

    /// **Mark resolved**: `git add` (`git rm` when the file is gone); conflict markers left ask first.
    pub(super) fn mark_resolved(&mut self, path: String, cx: &mut Context<Self>) {
        if self.conflict_draft_blocks(&path,cx) {return;}
        let Some(cl) = self.conflicts.as_mut() else {
            return;
        };
        let open = self
            .merge
            .as_ref()
            .filter(|m| m.path == path)
            .or(cl.merges.get(&path));
        let dirty = open.is_some_and(|m| m.dirty)
            || self
                .picker_v
                .as_ref()
                .is_some_and(|p| p.o.path == path && p.dirty);
        match crate::conflict_session::mark(
            &cl.root,
            &cl.snap,
            &path,
            dirty,
            cl.confirm.as_deref() == Some(path.as_str()),
        ) {
            Ok(crate::conflict_session::MarkOutcome::Notice(message)) => {
                self.toast(message, cx);
                return;
            }
            Ok(crate::conflict_session::MarkOutcome::Confirm(message)) => {
                cl.confirm = Some(path);
                self.comparison.banner = Some(message);
                cx.notify();
                return;
            }
            Ok(crate::conflict_session::MarkOutcome::Marked) => {
                cl.confirm = None;
                self.comparison.banner = None;
                cl.merges.remove(&path);
                if self.merge.as_ref().is_some_and(|m| m.path == path) {
                    self.merge = None;
                }
                self.toast(format!("Marked {path} resolved"), cx);
            }
            Err(e) => self.comparison.banner = Some(format!("Not marked resolved: {e}")),
        }
        self.refresh_conflicts(cx);
    }

    /// **Re-open** a marked file: its conflict back in the index and git's markers back in the file.
    pub(super) fn reopen_conflict(&mut self, path: String, cx: &mut Context<Self>) {
        if self.conflict_draft_blocks(&path,cx) {return;}
        let Some(cl) = self.conflicts.as_mut() else {
            return;
        };
        match crate::conflict_session::reopen(&cl.root, &path) {
            Ok(()) => {
                cl.merges.remove(&path);
                self.toast(format!("Re-opened {path}"), cx);
            }
            Err(e) => self.comparison.banner = Some(format!("Not re-opened: {e}")),
        }
        self.refresh_conflicts(cx);
    }

    fn conflict_draft_blocks(&mut self,path:&str,cx:&mut Context<Self>)->bool {
        if self.conflicts.as_ref().is_some_and(|cl|self.edits.dirty_path(&cl.root.join(path))) {
            self.edits_may_close(cx);
            self.toast("Save or discard unsaved changes before marking resolved or reopening this file".into(),cx);
            return true;
        }
        false
    }

    /// Esc while Mark resolved waits for confirmation.
    pub(super) fn conflict_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(cl) = self.conflicts.as_mut().filter(|c| c.confirm.is_some()) else {
            return false;
        };
        if ev.keystroke.key != "escape" {
            return false;
        }
        cl.confirm = None;
        self.comparison.banner = None;
        cx.notify();
        true
    }

    /// Re-read the operation in the background: after Mark resolved, and when the window comes back (a
    /// `git add` or `--continue` in a terminal).
    pub fn refresh_conflicts(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.repo.as_ref().map(|r| &r.picker).map(|p| p.root.clone()) else {
            return;
        };
        let filter = self
            .conflicts
            .as_ref()
            .map_or_else(Vec::new, |c| c.filter.clone());
        cx.spawn(async move |this, cx| {
            let snap = cx
                .background_executor()
                .spawn(async move { read(&root, &filter) })
                .await;
            this.update(cx, |v: &mut Viewer, cx| v.apply_conflicts(snap, cx))
                .ok();
        })
        .detach();
    }

    fn apply_conflicts(&mut self, snap: Result<Option<Snapshot>, String>, cx: &mut Context<Self>) {
        let snap = match snap {
            Ok(s) => s,
            Err(_) => return, // keep what's shown; the next refresh tries again
        };
        let Some(snap) = snap else {
            let Some(cl) = self.conflicts.take() else {
                return;
            };
            self.update_entry();
            self.toast("Merge finished".into(), cx);
            if cl.shown {
                // the log again, on the commit that finished it
                self.merge = None;
                self.source_action(SourceAction::AllBranches, cx);
            }
            cx.notify();
            return;
        };
        let unsaved = self.unsaved();
        let Some(root) = self.repo.as_ref().map(|r| &r.picker).map(|p| p.root.clone()) else {
            return;
        };
        let cl = self
            .conflicts
            .get_or_insert_with(|| ConflictList::new(root, Vec::new(), snap.clone()));
        // a Merge View is only good for the stages it was built from
        let stale = |p: &str| snap.file(p).is_none() || cl.snap.file(p) != snap.file(p);
        cl.merges.retain(|p, _| !stale(p));
        let mut changed = false;
        if cl.shown && self.merge.as_ref().is_some_and(|m| stale(&m.path)) {
            self.merge = None;
            changed = true;
        }
        changed |= cl.update(snap, &unsaved);
        let shown = cl.shown;
        self.update_entry();
        if shown && changed {
            self.show_conflicts(cx);
        }
        cx.notify();
    }

    /// The File Navigator row of a Conflicted File: state, name, kind tag.
    pub(super) fn conflict_row(&self, index: usize) -> Option<Vec<(String, u32)>> {
        let cl = self.conflicts.as_ref().filter(|c| c.shown)?;
        let path = &self.set.comparisons.get(index)?.path;
        let open = self
            .merge
            .as_ref()
            .filter(|m| &m.path == path)
            .or(cl.merges.get(path));
        let state = crate::conflict_session::displayed_state(cl.state(path), open.map(|m| &**m));
        let p = &self.palette;
        let (mark, color) = match state {
            State::Unresolved(Some(n)) => (format!("● {n} "), p.status[4]),
            State::Unresolved(None) => ("● ".to_string(), p.status[4]),
            State::Clean => ("○ ".to_string(), p.accent),
            State::Marked => ("✓ ".to_string(), p.status[0]),
        };
        let name = if self.file_search.query().is_empty() {path.rsplit('/').next().unwrap_or(path)} else {path.as_str()};
        let tag = cl
            .tags
            .get(path)
            .map_or(String::new(), |t| format!("  {t}"));
        Some(vec![(mark, color), (name.to_string(), p.fg), (tag, p.dim)])
    }

    /// The File Navigator's header: "N changed files", or where the Conflict List stands.
    pub(super) fn nav_header(&self) -> String {
        match self.conflicts.as_ref().filter(|c| c.shown) {
            Some(cl) if cl.left() == 0 => match cl.snap.op.operation.continue_command() {
                Some(command) => format!("All resolved · {command} ⧉"),
                None => "All resolved".to_string(),
            },
            Some(cl) => format!("Conflicts · {} of {} left", cl.left(), cl.states.len()),
            // Focus names what is compared above its files
            None if let Some(t) = self.layout.focused(self.repo.as_ref()).then(|| self.repo.as_ref().and_then(|r| r.selection_title())).flatten() => t.text,
            // every file of the Change Set, those in collapsed folders too
            None => match self.set.comparisons.len() {
                1 => "1 changed file".to_string(),
                n => format!("{n} changed files"),
            },
        }
    }

    /// A click on the File Navigator's header: copies the command that finishes the operation, once all are marked.
    pub(super) fn nav_header_click(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(cl) = self.conflicts.as_ref().filter(|c| c.shown && c.left() == 0) else {
            return false;
        };
        let Some(command) = cl.snap.op.operation.continue_command() else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(command.to_string()));
        self.toast(format!("Copied: {command}"), cx);
        true
    }

    /// The Navigator's right-click entries for a Conflicted File: Mark resolved, or Re-open once marked.
    pub(super) fn conflict_menu(&self, index: usize) -> Vec<(String, MenuAction)> {
        let Some(cl) = self.conflicts.as_ref().filter(|c| c.shown) else {
            return Vec::new();
        };
        let Some(c) = self.set.comparisons.get(index) else {
            return Vec::new();
        };
        match cl.state(&c.path) {
            State::Marked => vec![(
                crate::conflict_session::ListAct::Reopen.label().to_string(),
                MenuAction::Reopen(c.path.clone()),
            )],
            _ => vec![(
                crate::conflict_session::ListAct::MarkResolved
                    .label()
                    .to_string(),
                MenuAction::MarkResolved(c.path.clone()),
            )],
        }
    }

    /// The window's close button with unsaved Merge Views of other files: asks once.
    pub fn conflicts_may_close(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(cl) = self.conflicts.as_mut() else {
            return true;
        };
        let mut unsaved: Vec<&str> = cl
            .merges
            .values()
            .filter(|m| m.dirty)
            .map(|m| m.path.as_str())
            .collect();
        if unsaved.is_empty() {
            cl.close_asked = false;
            return true;
        }
        if cl.close_asked {
            return true;
        }
        unsaved.sort();
        cl.close_asked = true;
        self.comparison.banner = Some(format!(
            "Unsaved changes in {} — close again to discard them",
            unsaved.join(", ")
        ));
        cx.notify();
        false
    }
}

#[cfg(all(test, feature = "gui-test"))]
mod gui_tests {
    use super::super::merge_view::gui_tests::{event, fixture};
    use super::*;
    use gpui::TestAppContext;
    fn git(root: &std::path::Path, args: &[&str], ok: bool) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(status.success(), ok);
    }
    #[test]
    fn gui_conflict_list_real_menu_clicks_confirm_stage_and_reopen_git() {
        let mut cx = TestAppContext::single();
        cx.skip_drawing();
        let (_pair, viewer) = fixture(&mut cx);
        let dir = tempfile::Builder::new().prefix(".lenz-conflict-draft-").tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let d = dir.path();
        git(d, &["init", "-q", "-b", "main"], true);
        git(d, &["config", "user.name", "Lenz test"], true);
        git(d, &["config", "user.email", "test@example.invalid"], true);
        std::fs::write(d.join("f.txt"), b"base\n").unwrap();
        git(d, &["add", "."], true);
        git(d, &["commit", "-qm", "base"], true);
        git(d, &["checkout", "-qb", "feature"], true);
        std::fs::write(d.join("f.txt"), b"theirs\n").unwrap();
        git(d, &["commit", "-qam", "theirs"], true);
        git(d, &["checkout", "-q", "main"], true);
        std::fs::write(d.join("f.txt"), b"ours\n").unwrap();
        git(d, &["commit", "-qam", "ours"], true);
        git(d, &["merge", "-q", "feature"], false);
        viewer.update(&mut cx, |v, cx| {
            v.merge = None;
            v.repo = Some(crate::repository_session::RepositorySession::with_picker(crate::repository_session::Picker::new(d.into(), false)));
            v.start_conflicts(read(d, &[]).unwrap().unwrap(), Vec::new(), cx);
        });
        cx.executor().run_until_parked();
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone()))
            .unwrap();
        let click = MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(20.), px(10.)),
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        };
        let menu_click = |cx: &mut TestAppContext| {
            window
                .update(cx, |v, w, cx| {
                    v.show_menu(Menu {
                        x: 0.,
                        y: 0.,
                        items: v.conflict_menu(v.comparison.current),
                        hover: None,
                    });
                    v.on_mouse_down(&click, w, cx);
                })
                .unwrap();
        };
        menu_click(&mut cx);
        assert!(viewer.read_with(&cx, |v, _| {
            v.comparison.banner.as_deref().unwrap().contains("markers")
        }));
        window.update(&mut cx,|v,w,cx| {
            assert!(v.conflicts.as_ref().unwrap().confirming());
            let _=v.render(w,cx);let edge=v.diff_x()+v.merge_widths(v.panes.viewport[0]-v.diff_x())[0]+GUTTER_W/2.0;
            assert!(!v.begin_resize(edge,v.top()-hdr_h()/2.0));
            assert!(v.resize_handles().is_empty());
        }).unwrap();
        viewer.update(&mut cx, |v, cx| {
            assert!(v.conflict_key(&event("escape", false, false, false), cx))
        });
        viewer.update(&mut cx, |v, cx| {
            v.merge_key(&event("left", false, true, true), cx);
        });
        menu_click(&mut cx);
        assert_eq!(
            crate::conflicts::list(d).unwrap().len(),
            1,
            "unsaved choice must not stage"
        );
        assert!(
            viewer.read_with(&cx, |v, _| v.conflict_row(v.comparison.current).unwrap()[0]
                .0
                .contains("● 0")),
            "GUI rows show the shared unsaved resolution count"
        );

        viewer.update(&mut cx, |v, cx| {
            v.merge_key(&event("s", true, false, false), cx);
        });
        assert!(
            viewer.read_with(&cx, |v, _| v.conflict_row(v.comparison.current).unwrap()[0]
                .0
                .contains("○")),
            "GUI rows become clean immediately after Save"
        );
        menu_click(&mut cx);
        cx.executor().run_until_parked();
        assert!(crate::conflicts::list(d).unwrap().is_empty());
        assert_eq!(std::fs::read(d.join("f.txt")).unwrap(), b"theirs\n");
        assert!(viewer.read_with(&cx, |v, _| v.nav_header().contains("All resolved")));
        // The marked file's HEAD preview is editable. Its whole-file conflict
        // actions must neither replace a pending draft nor stage stale disk.
        viewer.update(&mut cx,|v,cx| {
            let i=v.current;let (lp,rp)=v.paths(i);
            let doc=Arc::new(Doc::load(&v.set,i,&lp,&rp,false).unwrap());
            v.view=doc.view(&doc.line_diff(&v.options),30.);
            let generation=v.generation;v.loaded.insert(i,Loaded::new(doc,generation));
            v.edit_current(crate::edit_session::EditCommand::Apply{block:0,onto:Side::Right},cx);
            assert!(v.edits.dirty(i));
        });
        let staged=crate::git::run(d,&["show",":f.txt"]).unwrap();
        menu_click(&mut cx);
        viewer.update(&mut cx,|v,cx|v.mark_resolved("f.txt".into(),cx));
        assert_eq!(std::fs::read(d.join("f.txt")).unwrap(),b"theirs\n");
        assert_eq!(crate::git::run(d,&["show",":f.txt"]).unwrap(),staged);
        assert!(crate::conflicts::list(d).unwrap().is_empty());
        assert!(viewer.read_with(&cx,|v,_|v.edits.dirty(v.current)&&v.write_error.as_ref().unwrap().contains("Unsaved")));
        viewer.update(&mut cx,|v,cx|v.edit_current(crate::edit_session::EditCommand::Discard,cx));
        menu_click(&mut cx);
        cx.executor().run_until_parked();
        assert_eq!(crate::conflicts::list(d).unwrap().len(), 1);
        assert!(viewer.read_with(&cx, |v, _| v.merge.as_ref().unwrap().m.unresolved() == 1));
    }

    #[test]
    fn gui_picker_unsaved_binary_choice_refuses_stage_until_real_save() {
        let mut cx = TestAppContext::single();
        cx.skip_drawing();
        let (_pair, viewer) = fixture(&mut cx);
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        git(d, &["init", "-q", "-b", "main"], true);
        git(d, &["config", "user.name", "Lenz test"], true);
        git(d, &["config", "user.email", "test@example.invalid"], true);
        std::fs::write(d.join("binary"), b"\0base\xff").unwrap();
        git(d, &["add", "."], true);
        git(d, &["commit", "-qm", "base"], true);
        git(d, &["checkout", "-qb", "feature"], true);
        std::fs::write(d.join("binary"), b"\0theirs\xfe").unwrap();
        git(d, &["commit", "-qam", "theirs"], true);
        git(d, &["checkout", "-q", "main"], true);
        std::fs::write(d.join("binary"), b"\0ours\xff").unwrap();
        git(d, &["commit", "-qam", "ours"], true);
        git(d, &["merge", "-q", "feature"], false);
        viewer.update(&mut cx, |v, cx| {
            v.merge = None;
            v.repo = Some(crate::repository_session::RepositorySession::with_picker(crate::repository_session::Picker::new(d.into(), false)));
            v.start_conflicts(read(d, &[]).unwrap().unwrap(), Vec::new(), cx);
        });
        cx.executor().run_until_parked();
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone()))
            .unwrap();
        window
            .update(&mut cx, |v, w, cx| {
                v.on_key(&event("2", false, false, false), w, cx)
            })
            .unwrap();
        assert!(viewer.read_with(&cx, |v, _| v.picker_v.as_ref().unwrap().dirty));
        let stage = |cx: &mut TestAppContext| {
            window
                .update(cx, |v, w, cx| {
                    v.show_menu(Menu {
                        x: 0.,
                        y: 0.,
                        items: v.conflict_menu(v.comparison.current),
                        hover: None,
                    });
                    v.on_mouse_down(
                        &MouseDownEvent {
                            button: MouseButton::Left,
                            position: point(px(20.), px(10.)),
                            modifiers: Default::default(),
                            click_count: 1,
                            first_mouse: false,
                        },
                        w,
                        cx,
                    );
                })
                .unwrap();
        };
        stage(&mut cx);
        assert_eq!(
            crate::conflicts::list(d).unwrap().len(),
            1,
            "unsaved whole-version choice must not stage old disk bytes"
        );
        assert_eq!(std::fs::read(d.join("binary")).unwrap(), b"\0ours\xff");
        window
            .update(&mut cx, |v, w, cx| {
                v.on_key(&event("s", true, false, false), w, cx)
            })
            .unwrap();
        assert_eq!(std::fs::read(d.join("binary")).unwrap(), b"\0theirs\xfe");
        stage(&mut cx);
        cx.executor().run_until_parked();
        assert!(crate::conflicts::list(d).unwrap().is_empty());
        let staged = std::process::Command::new("git")
            .args(["show", ":binary"])
            .current_dir(d)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(staged.status.success());
        assert_eq!(staged.stdout, b"\0theirs\xfe");
    }
}
