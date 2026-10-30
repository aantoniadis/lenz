//! A temporary Reader view of the confirmed baseline and the ordinary shared draft.
use super::*;
use crate::text::{BufferId, TextSnapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonSide { Saved, Current, Disk, Baseline, Marked, Head }
impl ComparisonSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "Last saved (read-only)",
            Self::Marked => "New file baseline (read-only)",
            Self::Head => "HEAD snapshot (read-only)",
            Self::Current => "Current draft",
            Self::Disk => "Disk (read-only)",
            Self::Baseline => "Confirmed baseline (read-only)",
        }
    }
    /// Why a side cannot be edited.
    pub fn read_only(self) -> Option<&'static str> {
        match self {
            Self::Saved => Some("The last saved version is read-only"),
            Self::Marked | Self::Head => Some("The marked change baseline is read-only"),
            Self::Disk => Some("The disk version is read-only"),
            Self::Baseline => Some("The confirmed baseline is read-only"),
            Self::Current => None,
        }
    }
}
/// What a comparison compares: the confirmed baseline with the draft, or a file that changed on disk with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonKind {
    /// The last confirmed save (or its absence) against the current draft.
    Saved,
    Marked,
    /// The file as it was on disk when the external-change dialog read it against the current draft, with the
    /// confirmed baseline shown beside it.
    Conflict,
}
pub(super) struct Before { active: Option<TabId>, other: Option<TabId>, focused: u8, pub(super) split: Option<Split> }
pub struct SavedComparison {
    kind: ComparisonKind,
    marker: Option<(Arc<TextSnapshot>, String)>,
    saved: TabId,
    current: TabId,
    /// The conflict comparison's third side: the confirmed baseline the draft was edited from.
    context: Option<TabId>,
    context_buffer: Option<BufferId>,
    context_source: Option<SemanticSource>,
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
    pub fn kind(&self) -> ComparisonKind { self.kind }
    pub fn baseline_label(&self) -> &str { self.marker.as_ref().map_or("last saved", |(_,label)|label.as_str()) }
    pub fn saved_tab(&self) -> TabId { self.saved }
    pub fn current_tab(&self) -> TabId { self.current }
    /// The confirmed baseline side of a conflict comparison (`None` for the saved comparison).
    pub fn context_tab(&self) -> Option<TabId> { self.context }
    pub fn pending(&self) -> bool { self.pending }
    pub fn changes(&self) -> &[lenz_engine::ChangeBlock] { &self.changes }
    pub fn line_endings_differ(&self) -> bool { self.line_endings_differ }
    pub fn failure(&self) -> Option<&str> { self.failure.as_deref() }
    pub(super) fn matches(&self, state: &State) -> bool {
        let Some(current) = state.source(self.current) else { return false };
        self.text.matches(current) && self.marker.as_ref().map(|(text,_)|text).or_else(|| state.confirmed_baseline(current.key().buffer)).is_some_and(|baseline| self.baseline.matches(baseline))
    }
    /// A changed row or the boundary of a deletion/insertion. Sorted blocks permit bounded lookup.
    pub fn changed_line(&self, side: ComparisonSide, line: usize) -> bool {
        if self.pending || self.failure.is_some() { return false; }
        let range = |change: &lenz_engine::ChangeBlock| match side {
            ComparisonSide::Saved | ComparisonSide::Disk | ComparisonSide::Marked | ComparisonSide::Head => change.left.clone(),
            ComparisonSide::Current => change.right.clone(),
            // The baseline is a third side: it is shown as it is, with no changed rows of its own.
            ComparisonSide::Baseline => 0..0,
        };
        if side == ComparisonSide::Baseline { return false; }
        let at = self.changes.partition_point(|change| range(change).start as usize <= line);
        at.checked_sub(1).is_some_and(|at| { let r = range(&self.changes[at]); line < r.end as usize || r.is_empty() && line == r.start as usize })
    }
}
impl State {
    pub fn saved_comparison(&self) -> Option<&SavedComparison> { self.comparison.as_ref() }
    pub(super) fn saved_buffer(&self, buffer: BufferId) -> bool { self.comparison.as_ref().is_some_and(|c| c.buffer == buffer || c.context_buffer == Some(buffer)) }
    pub(super) fn confirmed_baseline(&self, buffer: BufferId) -> Option<&Arc<TextSnapshot>> {
        let entry = self.sources.buffers.get(&buffer)?;
        entry.draft.as_ref().map(|d| &d.baseline).or(entry.snapshot.as_ref())
    }
    pub fn comparison_changed_line(&self, tab: TabId, line: usize) -> bool {
        let Some(comparison) = self.comparison.as_ref() else { return false };
        let side = if tab == comparison.saved {
            if comparison.kind == ComparisonKind::Conflict { ComparisonSide::Disk } else if comparison.kind == ComparisonKind::Marked { ComparisonSide::Marked } else { ComparisonSide::Saved }
        } else if tab == comparison.current { ComparisonSide::Current }
        else if Some(tab) == comparison.context { ComparisonSide::Baseline }
        else { return false };
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
        self.open_baseline_comparison(baseline,None)
    }
    pub(super) fn open_baseline_comparison(&mut self, baseline: Arc<TextSnapshot>, marker_label: Option<String>)->Effect {
        let Some(tab)=self.state.active() else{return Effect::default()};
        let Some(current)=self.state.source(tab.id).cloned() else{return Effect::default()};
        self.next_tab += 1; let saved = TabId(self.next_tab);
        self.next_tab += 1; let current_id = TabId(self.next_tab);
        let mut left = tab.duplicate(saved, 0);
        let mut right = tab.duplicate(current_id, 1);
        self.open_seq += 1;
        self.pending_goto = None;
        if self.state.asking.take().is_some() { self.state.semantics.cancel(); }
        self.cancel_documentation();
        self.state.sources.next += 1; let buffer = BufferId(self.state.sources.next);
        left.buffer = Some(buffer); left.comparison_side = Some(match marker_label.as_deref(){Some("last saved")|None=>ComparisonSide::Saved,Some(label) if label.starts_with("HEAD (")=>ComparisonSide::Head,_=>ComparisonSide::Marked});
        left.stamp = None; left.blame = None; left.markdown_source = true; left.new_content();
        left.top_line = left.top_line.min(baseline.line_count() - 1);
        let clamped = left.selection().map(|s| Selection { anchor: baseline.floor_char_boundary(s.anchor.min(baseline.len_bytes())), head: baseline.floor_char_boundary(s.head.min(baseline.len_bytes())) });
        left.place_selection(clamped);
        right.comparison_side = Some(ComparisonSide::Current); right.markdown_source = true;
        let before = Before { active: self.state.active, other: self.state.other, focused: self.state.focused, split: self.state.settings.split };
        self.state.sources.buffers.insert(buffer, CurrentSource { read_only: false, untitled: false, recovery: None, document: Default::default(), snapshot: Some(Arc::new(baseline.with_buffer(buffer))), stamp: None, saved: None, draft: None, save: Default::default() });
        self.comparison_seq += 1;
        self.state.comparison = Some(SavedComparison { kind: if marker_label.is_some(){ComparisonKind::Marked}else{ComparisonKind::Saved}, marker: marker_label.map(|label|(baseline.clone(),label)), saved, current: current_id, context: None, context_buffer: None, context_source: None, buffer, before, baseline: SemanticSource::new(&baseline), baseline_backing: Arc::downgrade(&baseline), text: SemanticSource::new(&current), request: self.comparison_seq, pending: true, submitted: false, changes: Vec::new(), line_endings_differ: false, failure: None });
        self.state.tabs.extend([left, right]);
        self.state.settings.split = Some(Split::Columns); self.state.focused = 1;
        self.state.active = Some(current_id); self.state.other = Some(saved);
        self.mark_shown(current_id, current.key().version); self.mark_shown(saved, baseline.key().version);
        self.decorate(current_id); self.decorate(saved);
        Effect { repaint: true, copy: None, ..Effect::default() }
    }
    /// The external-change comparison: the file as the dialog read it, the current draft, and the confirmed
    /// baseline the draft was edited from. Only the draft's side accepts edits.
    pub(super) fn compare_conflict(&mut self, disk: &crate::reconcile::Disk) -> Effect {
        if self.state.comparison.is_some() { return Effect::default(); }
        let Some(tab) = self.state.active() else { return Effect::default() };
        let Some(current) = self.state.source(tab.id).cloned() else { return Effect::default() };
        let Some(baseline) = self.state.confirmed_baseline(current.key().buffer).cloned() else { return Effect::default() };
        self.next_tab += 1; let left = TabId(self.next_tab);
        self.next_tab += 1; let context = TabId(self.next_tab);
        self.next_tab += 1; let right = TabId(self.next_tab);
        let mut disk_tab = tab.duplicate(left, 0);
        let mut baseline_tab = tab.duplicate(context, 0);
        let mut draft_tab = tab.duplicate(right, 1);
        self.open_seq += 1;
        self.pending_goto = None;
        if self.state.asking.take().is_some() { self.state.semantics.cancel(); }
        self.cancel_documentation();
        self.state.sources.next += 1; let disk_buffer = BufferId(self.state.sources.next);
        self.state.sources.next += 1; let baseline_buffer = BufferId(self.state.sources.next);
        let sides = [(left, ComparisonSide::Disk, disk_buffer, disk.text.clone()), (context, ComparisonSide::Baseline, baseline_buffer, baseline.clone())];
        for (id, side, buffer, text) in sides {
            let t = if id == left { &mut disk_tab } else { &mut baseline_tab };
            t.buffer = Some(buffer);
            t.comparison_side = Some(side);
            t.stamp = None; t.blame = None; t.markdown_source = true; t.new_content();
            t.top_line = t.top_line.min(text.line_count() - 1);
            let clamped = t.selection().map(|s| Selection { anchor: text.floor_char_boundary(s.anchor.min(text.len_bytes())), head: text.floor_char_boundary(s.head.min(text.len_bytes())) });
            t.place_selection(clamped);
        }
        draft_tab.comparison_side = Some(ComparisonSide::Current); draft_tab.markdown_source = true;
        let before = Before { active: self.state.active, other: self.state.other, focused: self.state.focused, split: self.state.settings.split };
        for (buffer, text) in [(disk_buffer, &disk.text), (baseline_buffer, &baseline)] {
            let snapshot = Arc::new(text.with_buffer(buffer));
            self.state.sources.buffers.insert(buffer, CurrentSource { read_only: false, untitled: false, recovery: None, document: Default::default(), snapshot: Some(snapshot), stamp: None, saved: None, draft: None, save: Default::default() });
        }
        self.comparison_seq += 1;
        self.state.comparison = Some(SavedComparison {
            kind: ComparisonKind::Conflict, marker: None, saved: left, current: right, context: Some(context), context_buffer: Some(baseline_buffer),
            context_source: Some(SemanticSource::new(&baseline)), buffer: disk_buffer, before,
            baseline: SemanticSource::new(&baseline), baseline_backing: Arc::downgrade(&baseline), text: SemanticSource::new(&current),
            request: self.comparison_seq, pending: true, submitted: false, changes: Vec::new(), line_endings_differ: false, failure: None,
        });
        self.state.tabs.extend([disk_tab, baseline_tab, draft_tab]);
        self.state.settings.split = Some(Split::Columns); self.state.focused = 1;
        self.state.active = Some(right); self.state.other = Some(left);
        self.mark_shown(right, current.key().version); self.mark_shown(left, disk.text.key().version); self.mark_shown(context, baseline.key().version);
        self.decorate(right); self.decorate(left); self.decorate(context);
        Effect { repaint: true, copy: None, ..Effect::default() }
    }

    pub(super) fn close_saved_comparison(&mut self) -> Effect {
        let Some(comparison) = self.state.comparison.take() else { return Effect::default() };
        self.state.tabs.retain(|tab| Some(tab.id) != Some(comparison.saved) && tab.id != comparison.current && Some(tab.id) != comparison.context);
        self.state.sources.buffers.remove(&comparison.buffer);
        self.analysis.forget(comparison.buffer);
        if let Some(buffer) = comparison.context_buffer {
            self.state.sources.buffers.remove(&buffer);
            self.analysis.forget(buffer);
        }
        // The dialog that opened a conflict comparison is shown again; the draft is still undecided.
        if comparison.kind == ComparisonKind::Conflict
            && let Some(pending) = self.reconcile.as_mut()
        { pending.comparing = false; }
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
        let (conflict, context) = (comparison.kind == ComparisonKind::Conflict, comparison.context);
        let Some(current) = self.state.source(current_id).cloned() else { return self.close_saved_comparison().repaint };
        let Some(baseline) = comparison.marker.as_ref().map(|(text,_)|text.clone()).or_else(||self.state.confirmed_baseline(current.key().buffer).cloned()) else { return self.close_saved_comparison().repaint };
        let baseline_changed = !comparison.baseline.matches(&baseline);
        let rebind = !comparison.baseline_backing.ptr_eq(&Arc::downgrade(&baseline));
        // A conflict comparison's disk side is the version the dialog read: only the baseline side follows a save.
        let left_changed = baseline_changed && !conflict;
        let changed = !comparison.text.matches(&current) || left_changed;
        if changed || rebind || (conflict && baseline_changed) {
            if left_changed || rebind {
                let snapshot = Arc::new(baseline.with_buffer(buffer));
                if let Some(entry) = self.state.sources.buffers.get_mut(&buffer) { entry.snapshot = Some(snapshot.clone()); entry.document = Default::default(); }
                if !baseline_changed { self.analysis.rebind_backing(snapshot, self.project_generation.load(Ordering::Acquire)); }
            }
            if let (true, Some(context_id), Some(context_buffer)) = (conflict && (baseline_changed || rebind), context, self.state.comparison.as_ref().and_then(|c| c.context_buffer)) {
                let snapshot = Arc::new(baseline.with_buffer(context_buffer));
                if let Some(entry) = self.state.sources.buffers.get_mut(&context_buffer) { entry.snapshot = Some(snapshot.clone()); entry.document = Default::default(); }
                if let Some(t) = self.tab_mut(context_id) {
                    t.new_content();
                    t.top_line = t.top_line.min(baseline.line_count() - 1);
                    let clamped = t.selection().map(|s| Selection { anchor: baseline.floor_char_boundary(s.anchor.min(baseline.len_bytes())), head: baseline.floor_char_boundary(s.head.min(baseline.len_bytes())) });
                    t.place_selection(clamped);
                }
                if let Some(c) = &mut self.state.comparison { c.context_source = Some(SemanticSource::new(&baseline)); }
                self.decorate(context_id);
            }
            if changed {
                self.comparison_seq += 1;
                if let Some(c) = &mut self.state.comparison { c.request = self.comparison_seq; c.baseline = SemanticSource::new(&baseline); c.text = SemanticSource::new(&current); c.pending = true; c.submitted = false; c.changes.clear(); c.line_endings_differ = false; c.failure = None; }
                if left_changed && let Some(t) = self.tab_mut(saved_id) { t.new_content(); t.top_line = t.top_line.min(baseline.line_count() - 1); let clamped = t.selection().map(|s| Selection { anchor: baseline.floor_char_boundary(s.anchor.min(baseline.len_bytes())), head: baseline.floor_char_boundary(s.head.min(baseline.len_bytes())) }); t.place_selection(clamped); }
                if left_changed { self.decorate(saved_id); }
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
