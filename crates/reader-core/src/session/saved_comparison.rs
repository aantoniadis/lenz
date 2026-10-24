//! A temporary Reader view of the confirmed baseline and the ordinary shared draft.
use super::*;
use crate::text::{BufferId, TextSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonSide { Saved, Current }
impl ComparisonSide {
    pub fn label(self) -> &'static str { match self { Self::Saved => "Last saved (read-only)", Self::Current => "Current draft" } }
}
pub(super) struct Before { active: Option<TabId>, other: Option<TabId>, focused: u8, pub(super) split: Option<Split> }
pub struct SavedComparison {
    saved: TabId,
    current: TabId,
    buffer: BufferId,
    pub(super) before: Before,
    baseline: SemanticSource,
    baseline_backing: std::sync::Weak<TextSnapshot>,
    text: SemanticSource,
    pub(super) request: u64,
    pub(super) pending: bool,
    submitted: bool,
    pub(super) changes: Vec<lenz_engine::ChangeBlock>,
    pub(super) line_endings_differ: bool,
    pub(super) failure: Option<String>,
}
impl SavedComparison {
    pub fn saved_tab(&self) -> TabId { self.saved }
    pub fn current_tab(&self) -> TabId { self.current }
    pub fn pending(&self) -> bool { self.pending }
    pub fn changes(&self) -> &[lenz_engine::ChangeBlock] { &self.changes }
    pub fn line_endings_differ(&self) -> bool { self.line_endings_differ }
    pub fn failure(&self) -> Option<&str> { self.failure.as_deref() }
    pub(super) fn matches(&self, state: &State) -> bool {
        let Some(current) = state.source(self.current) else { return false };
        self.text.matches(current) && state.confirmed_baseline(current.key().buffer).is_some_and(|baseline| self.baseline.matches(baseline))
    }
    /// A changed row or the boundary of a deletion/insertion. Sorted blocks permit bounded lookup.
    pub fn changed_line(&self, side: ComparisonSide, line: usize) -> bool {
        if self.pending || self.failure.is_some() { return false; }
        let range = |change: &lenz_engine::ChangeBlock| match side { ComparisonSide::Saved => change.left.clone(), ComparisonSide::Current => change.right.clone() };
        let at = self.changes.partition_point(|change| range(change).start as usize <= line);
        at.checked_sub(1).is_some_and(|at| { let r = range(&self.changes[at]); line < r.end as usize || r.is_empty() && line == r.start as usize })
    }
}
impl State {
    pub fn saved_comparison(&self) -> Option<&SavedComparison> { self.comparison.as_ref() }
    pub(super) fn saved_buffer(&self, buffer: BufferId) -> bool { self.comparison.as_ref().is_some_and(|c| c.buffer == buffer) }
    fn confirmed_baseline(&self, buffer: BufferId) -> Option<&Arc<TextSnapshot>> {
        let entry = self.sources.buffers.get(&buffer)?;
        entry.draft.as_ref().map(|d| &d.baseline).or(entry.snapshot.as_ref())
    }
    pub fn comparison_changed_line(&self, tab: TabId, line: usize) -> bool {
        let Some(comparison) = self.comparison.as_ref() else { return false };
        let side = if tab == comparison.saved { ComparisonSide::Saved }
            else if tab == comparison.current { ComparisonSide::Current } else { return false };
        comparison.changed_line(side, line)
    }
}
impl Session {
    pub(super) fn compare_saved_disabled(&self) -> Option<&'static str> {
        if self.state.comparison.is_some() { return Some("A saved comparison is already open"); }
        let Some(view) = self.state.active().and_then(|t| self.state.view(t.id)) else { return Some("No loaded file is open") };
        if view.tab().untitled() { return Some(super::UNTITLED_NO_FILE); }
        view.intrinsic_edit_disabled()
    }
    pub(super) fn compare_saved(&mut self) -> Effect {
        if let Some(why) = self.compare_saved_disabled() { self.state.messages.push(why.into()); return Effect { repaint: true, copy: None, ..Effect::default() }; }
        let Some(tab) = self.state.active() else { return Effect::default() };
        let Some(current) = self.state.source(tab.id).cloned() else { return Effect::default() };
        let Some(baseline) = self.state.confirmed_baseline(current.key().buffer).cloned() else { return Effect::default() };
        self.next_tab += 1; let saved = TabId(self.next_tab);
        self.next_tab += 1; let current_id = TabId(self.next_tab);
        let mut left = tab.duplicate(saved, 0);
        let mut right = tab.duplicate(current_id, 1);
        self.open_seq += 1;
        self.pending_goto = None;
        if self.state.asking.take().is_some() { self.state.semantics.cancel(); }
        self.cancel_documentation();
        self.state.sources.next += 1; let buffer = BufferId(self.state.sources.next);
        left.buffer = Some(buffer); left.comparison_side = Some(ComparisonSide::Saved);
        left.stamp = None; left.blame = None; left.markdown_source = true; left.new_content();
        left.top_line = left.top_line.min(baseline.line_count() - 1);
        let clamped = left.selection().map(|s| Selection { anchor: baseline.floor_char_boundary(s.anchor.min(baseline.len_bytes())), head: baseline.floor_char_boundary(s.head.min(baseline.len_bytes())) });
        left.place_selection(clamped);
        right.comparison_side = Some(ComparisonSide::Current); right.markdown_source = true;
        let before = Before { active: self.state.active, other: self.state.other, focused: self.state.focused, split: self.state.settings.split };
        self.state.sources.buffers.insert(buffer, CurrentSource { read_only: false, untitled: false, document: Default::default(), snapshot: Some(Arc::new(baseline.with_buffer(buffer))), stamp: None, saved: None, draft: None, save: Default::default() });
        self.comparison_seq += 1;
        self.state.comparison = Some(SavedComparison { saved, current: current_id, buffer, before, baseline: SemanticSource::new(&baseline), baseline_backing: Arc::downgrade(&baseline), text: SemanticSource::new(&current), request: self.comparison_seq, pending: true, submitted: false, changes: Vec::new(), line_endings_differ: false, failure: None });
        self.state.tabs.extend([left, right]);
        self.state.settings.split = Some(Split::Columns); self.state.focused = 1;
        self.state.active = Some(current_id); self.state.other = Some(saved);
        self.mark_shown(current_id, current.key().version); self.mark_shown(saved, baseline.key().version);
        self.decorate(current_id); self.decorate(saved);
        Effect { repaint: true, copy: None, ..Effect::default() }
    }
    pub(super) fn close_saved_comparison(&mut self) -> Effect {
        let Some(comparison) = self.state.comparison.take() else { return Effect::default() };
        self.state.tabs.retain(|tab| tab.id != comparison.saved && tab.id != comparison.current);
        self.state.sources.buffers.remove(&comparison.buffer);
        self.analysis.forget(comparison.buffer);
        self.state.active = comparison.before.active.filter(|id| self.state.tabs.iter().any(|t| t.id == *id));
        self.state.other = comparison.before.other.filter(|id| self.state.tabs.iter().any(|t| t.id == *id));
        self.state.focused = comparison.before.focused; self.state.settings.split = comparison.before.split;
        self.current.store(self.state.active.and_then(|id| self.state.source(id)).map_or(0, |s| s.key().version), Ordering::Relaxed);
        self.other_current.store(self.state.other.and_then(|id| self.state.source(id)).map_or(0, |s| s.key().version), Ordering::Relaxed);
        self.invalidate_result_analysis();
        self.synced = 0;
        Effect { repaint: true, copy: None, ..Effect::default() }
    }
    pub(super) fn refresh_saved_comparison(&mut self) -> bool {
        let Some(comparison) = self.state.comparison.as_ref() else { return false };
        let (saved_id, current_id, buffer) = (comparison.saved, comparison.current, comparison.buffer);
        let Some(current) = self.state.source(current_id).cloned() else { return self.close_saved_comparison().repaint };
        let Some(baseline) = self.state.confirmed_baseline(current.key().buffer).cloned() else { return self.close_saved_comparison().repaint };
        let baseline_changed = !comparison.baseline.matches(&baseline);
        let changed = !comparison.text.matches(&current) || baseline_changed;
        let rebind = !comparison.baseline_backing.ptr_eq(&Arc::downgrade(&baseline));
        if changed || rebind {
            if baseline_changed || rebind {
                let snapshot = Arc::new(baseline.with_buffer(buffer));
                if let Some(entry) = self.state.sources.buffers.get_mut(&buffer) { entry.snapshot = Some(snapshot.clone()); entry.document = Default::default(); }
                if !baseline_changed { self.analysis.rebind_backing(snapshot, self.project_generation.load(Ordering::Acquire)); }
            }
            if changed {
                self.comparison_seq += 1;
                if let Some(c) = &mut self.state.comparison { c.request = self.comparison_seq; c.baseline = SemanticSource::new(&baseline); c.text = SemanticSource::new(&current); c.pending = true; c.submitted = false; c.changes.clear(); c.line_endings_differ = false; c.failure = None; }
                if baseline_changed && let Some(t) = self.tab_mut(saved_id) { t.new_content(); t.top_line = t.top_line.min(baseline.line_count() - 1); let clamped = t.selection().map(|s| Selection { anchor: baseline.floor_char_boundary(s.anchor.min(baseline.len_bytes())), head: baseline.floor_char_boundary(s.head.min(baseline.len_bytes())) }); t.place_selection(clamped); }
                if baseline_changed { self.decorate(saved_id); }
            }
            if let Some(c) = &mut self.state.comparison { c.baseline_backing = Arc::downgrade(&baseline); }
        }
        if self.state.comparison.as_ref().is_none_or(|c| !c.pending || c.submitted) { return changed; }
        let project = self.project_generation.load(Ordering::Acquire);
        let mut documents = Vec::with_capacity(2);
        for id in [saved_id, current_id] {
            let Some(view) = self.state.view(id) else { return changed };
            let snapshot = view.snapshot().clone();
            let loaded = self.state.sources.buffers.get(&snapshot.key().buffer).map(|entry| entry.document.clone()).unwrap_or_default();
            let admitted = self.analysis.ensure(crate::analysis::Input::new(snapshot.clone(), loaded, project, true), Priority::Visible);
            let failure = self.analysis.source_failure(&snapshot, project).or((!admitted).then_some("Comparison admission is full; close and try again"));
            if let Some(why) = failure {
                if let Some(c) = &mut self.state.comparison { c.pending = false; c.failure = Some(why.into()); }
                return true;
            }
            documents.push(self.analysis.ready(&snapshot, project));
        }
        let [Some(left), Some(right)] = documents.as_slice() else { return changed };
        let (left, right) = (left.clone(), right.clone());
        let Some(c) = self.state.comparison.as_ref() else { return changed };
        let request = c.request;
        let fence = crate::analysis::ResultFence { slot: crate::analysis::ResultSlot::SavedComparison, project: self.project_generation.load(Ordering::Acquire), request, stage: crate::analysis::ResultStage::SavedComparison, result_set: None, node: None, source: Some(c.text.clone()) };
        let admitted = self.analysis.request_result(fence, Priority::Visible, || move |lease| {
            if !lease.wanted() { return; }
            let changes = lenz_engine::compare_lines(&lenz_engine::split_lines(left.text()), &lenz_engine::split_lines(right.text()), &lenz_engine::Options::default());
            if lease.wanted() { lease.send(Completion::SavedCompared { request, changes, line_endings_differ: lenz_engine::line_ending(left.text()) != lenz_engine::line_ending(right.text()) }); }
        });
        if let Some(c) = &mut self.state.comparison { match admitted {
            Ok(_) => c.submitted = true,
            Err(()) => { c.pending = false; c.failure = Some("Comparison admission is full; close and try again".into()); }
        } }
        changed
    }
}
