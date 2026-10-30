//! Keeping a shown draft's change markers current: one settled comparison per buffer, fenced by the draft version and
//! the baseline it was asked for, so a late result is never published over newer text.
use super::*;
use crate::markers::{Baseline, Blocks, HeadBlob, MarkerState, Markers, Tracked};
use crate::text::{BufferId, TextSnapshot};
use lenz_content::branches::FileStatus;

/// HEAD's blob as a comparison was given it: its commit, its label and its text.
type Head = (String, String, Arc<Document>);

/// What a comparison compares the draft with, decided when it is asked for.
enum Source {
    /// HEAD's blob at the file's path, `cached` while HEAD has not moved since it was read.
    Head { file: PathBuf, branch: Option<String>, cached: Option<Head> },
    /// No committed version at this path: every line is added.
    New { label: &'static str },
    /// The confirmed last saved version.
    Saved(Arc<TextSnapshot>, SemanticSource),
}

/// One draft's lines compared with its baseline, as the worker sends them back.
pub struct Marked {
    /// The baseline it compared against, named so the display never leaves it implicit.
    pub baseline: Baseline,
    pub(crate) blocks: Blocks,
    pub(crate) baseline_text: Arc<TextSnapshot>,
    pub(crate) hunks: Vec<crate::markers::Hunk>,
    /// HEAD's commit, label and text when the comparison read them (kept while HEAD does not move).
    pub(crate) head: Option<Head>,
}

impl State {
    /// Whether the file is in the session's repository, and its status against HEAD as last read. `None` while the
    /// repository or its statuses are still being looked up: guessing a baseline then would name the wrong one.
    fn marker_repo(&self, tab: &Tab) -> Option<(bool, Option<FileStatus>)> {
        let path = tab.canonical.as_deref().unwrap_or(&tab.source);
        match (self.repo.as_ref(), self.git_status.as_ref()) {
            (None, _) if !self.repo_checked() => None,
            (None, _) => Some((false, None)),
            (Some(_), None) => None,
            (Some(_), Some(statuses)) => Some((statuses.contains(path), statuses.of(path))),
        }
    }
}

impl Session {
    /// Compares the shown drafts again where their version, baseline or repository state changed: `settle` starts the
    /// comparison now (the debounce woke us), else schedules one once the draft rests.
    pub(super) fn refresh_markers(&mut self, settle: bool) {
        let shown = [self.state.active, self.state.other].into_iter().flatten().collect::<Vec<_>>();
        // Baselines can hold a whole snapshot or HEAD blob. Inactive tabs must not retain them outside the cache.
        let visible: Vec<_> = shown.iter().filter_map(|id| self.state.tabs.iter().find(|tab| tab.id == *id).and_then(|tab| tab.buffer)).collect();
        let retired: Vec<_> = self.state.marker_sets.keys().filter(|buffer| !visible.contains(buffer)).copied().collect();
        for buffer in retired {
            self.state.marker_sets.remove(&buffer);
            self.analysis.cancel_result(crate::analysis::ResultSlot::Markers(buffer));
        }
        for id in shown {
            let Some(tab) = self.state.tabs.iter().find(|tab| tab.id == id).filter(|tab| tab.is_file() && tab.loaded && tab.buffer.is_some()) else { continue };
            let Some((in_repo, status)) = self.state.marker_repo(tab) else { continue };
            let Some(view) = self.state.view(id) else { continue };
            let snapshot = view.snapshot().clone();
            let tracked = Tracked {
                key: snapshot.key(),
                path: tab.canonical.clone().unwrap_or_else(|| tab.source.clone()),
                in_repo,
                status,
                epoch: self.markers_epoch,
                saved: self.state.confirmed_baseline(snapshot.key().buffer).map(|baseline| SemanticSource::new(baseline)),
            };
            let buffer = tracked.key.buffer;
            if self.state.marker_sets.get(&buffer).is_some_and(|markers| markers.tracked == tracked) {
                // The same draft and baseline as last time: nothing to compare again.
            } else {
                let running = self.state.marker_sets.get(&buffer).is_some_and(|markers| markers.request != 0);
                match self.state.marker_sets.get_mut(&buffer) {
                    // The comparison for the old text is dropped: nothing it found may be published now.
                    Some(markers) => { markers.tracked = tracked.clone(); markers.state = MarkerState::Pending; markers.request = 0; }
                    None => { self.state.marker_sets.insert(buffer, Markers::new(tracked.clone())); }
                }
                // A comparison already running is for other text; its result can no longer be published.
                if running { self.analysis.cancel_result(crate::analysis::ResultSlot::Markers(buffer)); }
                self.markers_debounce.rest(buffer);
            }
            if settle && self.state.marker_sets.get(&buffer).is_some_and(|markers| markers.pending() && markers.request == 0) {
                self.start_markers(id, snapshot);
            }
        }
        if self.inspected_change.as_ref().is_some_and(|i| !self.inspected_change_current(i)) { self.inspected_change = None; }
    }

    /// Asks for one comparison of tab `id`'s draft against its baseline, off the UI thread.
    fn start_markers(&mut self, id: TabId, snapshot: Arc<TextSnapshot>) {
        let buffer = snapshot.key().buffer;
        let Some(markers) = self.state.marker_sets.get(&buffer) else { return };
        let tracked = markers.tracked.clone();
        let cached = markers.head.as_ref().filter(|head| head.epoch == self.markers_epoch).map(|head| (head.sha.clone(), head.label.clone(), head.document.clone()));
        let source = match (tracked.in_repo, tracked.status) {
            (true, Some(FileStatus::Untracked)) => Some(Source::New { label: "new file (untracked)" }),
            (true, Some(FileStatus::Added)) => Some(Source::New { label: "new file (added to Git)" }),
            (true, _) => Some(Source::Head {
                file: tracked.path.clone(),
                // The session's own repository names its branch; a file of another one is named by its commit alone.
                branch: self.state.repo.as_ref().filter(|repo| tracked.path.starts_with(&repo.root)).map(|repo| repo.head.label()),
                cached,
            }),
            // Outside Git the confirmed last saved version is the baseline; a document never read has none.
            (false, _) => self.state.confirmed_baseline(buffer).map(|saved| Source::Saved(saved.clone(), SemanticSource::new(&snapshot))),
        };
        let Some(source) = source else {
            if let Some(markers) = self.state.marker_sets.get_mut(&buffer) { markers.state = MarkerState::Failed("No saved version to compare with".into()); }
            return;
        };
        self.markers_requests += 1;
        let request = self.markers_requests;
        let project = self.project_generation.load(Ordering::Acquire);
        let source_identity = SemanticSource::new(&snapshot);
        let loaded = self.state.sources.buffers.get(&buffer).map(|entry| entry.document.clone()).unwrap_or_default();
        let input = crate::analysis::Input::new(snapshot, loaded, project, self.is_shown(id));
        let fence = crate::analysis::ResultFence {
            slot: crate::analysis::ResultSlot::Markers(buffer),
            project,
            request,
            stage: crate::analysis::ResultStage::Markers,
            result_set: None,
            node: None,
            source: Some(source_identity),
        };
        let work = move |document: Arc<Document>, lease: crate::analysis::Lease| {
            // Sent only while this comparison is still the wanted one, so a superseded result never publishes.
            lease.send(Completion::Marked { request, buffer, result: compare(&source, &document, &lease) });
        };
        match self.analysis.request_result_document(fence, Priority::Background, || (input, work)) {
            Ok(_) => {
                if let Some(markers) = self.state.marker_sets.get_mut(&buffer) { markers.request = request; }
            }
            Err(()) => {
                if let Some(markers) = self.state.marker_sets.get_mut(&buffer) { markers.state = MarkerState::Failed("Background work is busy; the markers return after it".into()); }
            }
        }
    }

    /// A comparison ended: publish it for the exact draft and baseline it was asked for, never over newer text.
    pub(super) fn marked(&mut self, request: u64, buffer: BufferId, result: Result<Marked, String>) -> Effect {
        let Some(markers) = self.state.marker_sets.get_mut(&buffer).filter(|markers| markers.request == request) else { return Effect::default() };
        markers.request = 0;
        match result {
            Ok(Marked { baseline, blocks, head, baseline_text, hunks }) => {
                let Some(source) = self.state.sources.buffers.get(&buffer).and_then(|entry| entry.snapshot.as_ref()).map(|snapshot| SemanticSource::new(snapshot)) else { return Effect::default() };
                markers.published = Some(crate::markers::Published { source, baseline, blocks, baseline_text, hunks });
                markers.state = MarkerState::Ready;
                if let Some((sha, label, document)) = head {
                    markers.head = Some(HeadBlob { sha, label, document, epoch: self.markers_epoch });
                }
            }
            Err(why) => markers.state = MarkerState::Failed(why),
        }
        Effect { repaint: true, copy: None, ..Effect::default() }
    }

    /// HEAD, the statuses or the repository were read again: the shown drafts' baselines may have moved.
    pub(super) fn markers_moved(&mut self) {
        self.markers_epoch += 1;
        self.refresh_markers(false);
    }
}

/// Compares `document` with `source`, returning the named baseline, its blocks and HEAD's blob when it was read.
fn compare(source: &Source, document: &Document, lease: &crate::analysis::Lease) -> Result<Marked, String> {
    let current = lease.cancellation();
    match source {
        Source::New { label } => {
            let empty = Document::decode(document.source().to_owned(),0,b"");
            Ok(mark(Baseline::New { label }, Arc::new(TextSnapshot::from_document(document_snapshot_buffer(),&empty)), &empty, document, None))
        }
        Source::Saved(saved, current_source) => compare_saved(saved, current_source, document, current),
        Source::Head { file, branch, cached } => {
            if let Some((sha,label,blob)) = cached {
                return Ok(mark(Baseline::Head {label:label.clone()},Arc::new(TextSnapshot::from_document(document_snapshot_buffer(),blob)),blob,document,Some((sha.clone(),label.clone(),blob.clone()))));
            }
            let (root,path) = lenz_content::blame::repository(file)?;
            let sha = lenz_content::branches::head_commit(&root).ok_or("HEAD is unreadable")?;
            let label = format!("HEAD ({})",short(&sha,branch.as_deref()));
            match lenz_content::branches::blob_at_head(&root,&path)? {
                Some(bytes) => {
                    let blob=Arc::new(Document::decode(file.clone(),0,&bytes));
                    if !current() { return Err("The comparison was superseded".into()); }
                    Ok(mark(Baseline::Head {label:label.clone()},Arc::new(TextSnapshot::from_document(document_snapshot_buffer(),&blob)),&blob,document,Some((sha,label,blob.clone()))))
                }
                None => {
                    let label=if lenz_content::branches::tracks(&root,&path) {"new file (not in HEAD)"}else{"new file (untracked)"};
                    let empty=Document::decode(file.clone(),0,b"");
                    Ok(mark(Baseline::New {label},Arc::new(TextSnapshot::from_document(document_snapshot_buffer(),&empty)),&empty,document,None))
                }
            }
        }
    }
}

fn document_snapshot_buffer() -> BufferId { BufferId(0) }

fn compare_saved(saved: &Arc<TextSnapshot>, current_source: &SemanticSource, document: &Document, current: impl Fn() -> bool) -> Result<Marked, String> {
    if !current() { return Err("The comparison was superseded".into()); }
    // A saved version already names these exact immutable bytes. Keep its baseline without building another
    // full document or allocating the line diff's workspace for an unchanged file.
    if current_source.matches(saved) {
        return Ok(Marked { baseline: Baseline::Saved, blocks: Blocks::default(), baseline_text: saved.clone(), hunks: Vec::new(), head: None });
    }
    let left = saved.materialize_while(current).ok_or("The comparison was superseded")?;
    Ok(mark(Baseline::Saved, saved.clone(), &left, document, None))
}

fn mark(baseline: Baseline, baseline_text: Arc<TextSnapshot>, left: &Document, right: &Document, head: Option<Head>) -> Marked {
    // Undo or an equal HEAD blob can have different source identities while still comparing identical text.
    if left.text() == right.text() {
        return Marked { baseline, blocks: Blocks::default(), baseline_text, hunks: Vec::new(), head };
    }
    let left_lines=if matches!(baseline,Baseline::New{..}){Vec::new()}else{lenz_engine::split_lines(left.text())};
    let changes=lenz_engine::compare_lines(&left_lines,&lenz_engine::split_lines(right.text()),&lenz_engine::Options {highlight:lenz_engine::HighlightLevel::Line,..Default::default()});
    let mut blocks=crate::markers::from_changes(&changes);
    let last=u32::try_from(right.line_count().saturating_sub(1)).unwrap_or(u32::MAX);
    blocks.clamp_deletions(last);
    let offset=|doc:&Document,line:u32| if (line as usize)<doc.line_count(){doc.line_range(line as usize).start}else{doc.text().len()};
    let hunks=changes.iter().filter(|c|!c.ignored).map(|c| {
        let mut l=offset(left,c.left.start)..offset(left,c.left.end);
        let mut r=offset(right,c.right.start)..offset(right,c.right.end);
        // The final empty line represents an ending: include its preceding line when the byte range is empty.
        if (!c.left.is_empty() && l.is_empty()) || (!c.right.is_empty() && r.is_empty())
            || (l.is_empty() && l.end==left.text().len() && l.end>0 && !left.text().ends_with('\n'))
            || (r.is_empty() && r.end==right.text().len() && r.end>0 && !right.text().ends_with('\n')) {
            l.start=offset(left,c.left.start.saturating_sub(1));r.start=offset(right,c.right.start.saturating_sub(1));
        }
        crate::markers::Hunk{left:l,right:r,lines:if c.right.is_empty(){c.right.start.min(last)..c.right.end.min(last)}else{c.right.clone()}}
    }).collect();
    Marked{baseline,blocks,baseline_text,hunks,head}
}

/// `main · 1a2b3c4`, or the commit alone for a file of another repository.
fn short(sha: &str, branch: Option<&str>) -> String {
    let sha = sha.get(..7).unwrap_or(sha);
    branch.map_or_else(|| sha.to_string(), |branch| format!("{branch} · {sha}"))
}

impl Session {
    fn inspected_change_current(&self, inspection: &crate::markers::Inspection) -> bool {
        (inspection.baseline != Baseline::Saved || self.state.confirmed_baseline(inspection.source.key.buffer).is_some_and(|b|SemanticSource::new(&inspection.baseline_text).matches(b)))
            && self.state.active == Some(inspection.tab) && self.state.source(inspection.tab).is_some_and(|s| inspection.source.matches(s))
            && self.state.markers(inspection.tab).is_some_and(|m| !m.pending() && m.failure().is_none() && m.tracked.epoch == inspection.epoch && m.tracked.epoch == self.markers_epoch
                && m.published.as_ref().is_some_and(|p| p.source == inspection.source && p.baseline == inspection.baseline))
    }
    fn marker_hunk_at(&self, tab: TabId, line: usize) -> Option<(&crate::markers::Markers, &crate::markers::Hunk)> {
        let m=self.state.markers(tab)?;
        if m.pending() || m.failure().is_some() { return None; }
        let p=m.published.as_ref()?;
        if !self.state.source(tab).is_some_and(|s|p.source.matches(s)) { return None; }
        let line=u32::try_from(line).ok()?;
        let at=p.hunks.partition_point(|h|h.lines.start<=line);
        let h=p.hunks.get(at.checked_sub(1)?)?;
        (line<h.lines.end || h.lines.is_empty() && line==h.lines.start).then_some((m,h))
    }
    pub(super) fn change_action_disabled(&self,id:CommandId)->Option<&'static str> {
        if id==CommandId::InspectChange {
            let Some(tab)=self.state.active else{return Some("No file is open")};
            let line=self.state.view(tab).map(|v|v.snapshot().line_of(v.tab().selection().map_or(0,|s|s.head))).unwrap_or(0);
            return self.marker_hunk_at(tab,line).is_none().then_some("No settled marked change at the caret; wait for markers or move to one");
        }
        let Some(i)=self.inspected_change.as_ref().filter(|i|self.inspected_change_current(i)) else { return Some("Inspect a current marked change first") };
        match id {
            CommandId::ShowChangeDiff => self.state.comparison.is_some().then_some("A comparison is already open; close it first"),
            CommandId::RevertChange => self.state.view(i.tab).and_then(|v|v.edit_disabled()).or((i.hunk.left.len()>4*1024*1024).then_some("Change exceeds 4 MiB; use the editable comparison")),
            _=>None,
        }
    }
    pub fn inspected_change_text(&self)->Option<String> {
        let i=self.inspected_change.as_ref().filter(|i|self.inspected_change_current(i))?;
        let end=i.baseline_text.floor_char_boundary(i.hunk.left.start.saturating_add(65536).min(i.hunk.left.end));
        let prior=i.baseline_text.copy_range(i.hunk.left.start..end).unwrap_or_default();
        let cut=if end<i.hunk.left.end {"\n[preview limited to 64 KiB]"}else{""};
        Some(format!("Change against {}\nPrior content ({} bytes):\n{}{}\n\nRevert edits the shared draft as one undoable action. Save writes it to disk.{}",i.baseline.label(),i.hunk.left.len(),if prior.is_empty(){"[empty]"}else{&prior},cut,self.change_action_disabled(CommandId::RevertChange).map_or(String::new(),|why|format!("\nRevert unavailable: {why}"))))
    }
    pub(super) fn inspect_change(&mut self,at:Option<(TabId,usize)>)->Effect {
        let at=at.or_else(|| {let tab=self.state.active?;let v=self.state.view(tab)?;Some((tab,v.snapshot().line_of(v.tab().selection().map_or(0,|s|s.head))))});
        let Some((tab,line))=at else{return Effect::default()};
        let Some((m,h))=self.marker_hunk_at(tab,line) else{return Effect::default()};
        let Some(p)=m.published.as_ref() else{return Effect::default()};
        let guard=match &p.baseline {
            Baseline::Head{..}=>m.head.as_ref().map(|h|crate::markers::Guard::Head{path:m.tracked.path.clone(),sha:h.sha.clone()}),
            Baseline::New{..}=>Some(crate::markers::Guard::New{path:m.tracked.path.clone()}),
            Baseline::Saved=>None,
        };
        self.inspected_change=Some(crate::markers::Inspection{tab,source:p.source.clone(),epoch:m.tracked.epoch,baseline:p.baseline.clone(),baseline_text:p.baseline_text.clone(),hunk:h.clone(),guard});
        Effect {repaint:true,..Effect::default()}
    }
    pub(super) fn show_change_diff(&mut self)->Effect {
        if self.change_action_disabled(CommandId::ShowChangeDiff).is_some(){return Effect::default();}
        let Some(i)=self.inspected_change.take()else{return Effect::default()};
        self.state.active=Some(i.tab);
        self.open_baseline_comparison(i.baseline_text,Some(i.baseline.label().to_owned()))
    }
    pub(super) fn revert_change(&mut self)->Effect {
        if let Some(why)=self.change_action_disabled(CommandId::RevertChange){self.state.messages.push(why.into());return Effect{repaint:true,..Effect::default()};}
        let Some(i)=self.inspected_change.take()else{return Effect::default()};
        if let Some(guard)=i.guard.clone() {
            self.semantic_seq+=1;let request=self.semantic_seq;let tx=self.tx.clone();
            self.reverting_change=Some((request,i));
            self.jobs.submit(Priority::Visible,move||{
                let unchanged=match guard {
                    crate::markers::Guard::Head{path,sha}=>lenz_content::blame::repository(&path).ok().and_then(|(root,_)|lenz_content::branches::head_commit(&root)).is_some_and(|now|now==sha),
                    crate::markers::Guard::New{path}=>lenz_content::blame::repository(&path).ok().is_some_and(|(root,file)|!lenz_content::branches::in_head(&root,&file)),
                };
                let _=tx.send_blocking(Completion::ChangeChecked{request,unchanged});
            });
            Effect{repaint:true,..Effect::default()}
        }else{self.apply_change_revert(i)}
    }
    pub(super) fn change_checked(&mut self,request:u64,unchanged:bool)->Effect {
        if self.reverting_change.as_ref().is_none_or(|(r,_)|*r!=request){return Effect::default();}
        let Some((_,i))=self.reverting_change.take()else{return Effect::default()};
        if !unchanged {self.markers_moved();self.state.messages.push("The change baseline moved; inspect the refreshed markers before reverting".into());return Effect{repaint:true,..Effect::default()};}
        if !self.inspected_change_current(&i){self.state.messages.push("The change moved while checking; inspect the current markers before reverting".into());return Effect{repaint:true,..Effect::default()};}
        self.apply_change_revert(i)
    }
    fn apply_change_revert(&mut self,i:crate::markers::Inspection)->Effect {
        let Some(text)=i.baseline_text.copy_range(i.hunk.left)else{return Effect::default()};
        self.edit(i.tab,i.source.key,vec![crate::text::TextEdit{range:i.hunk.right,text:text.into()}],crate::text::EditKind::Replace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_saved_snapshot_does_not_materialize_another_full_document() {
        let document = Document::decode(PathBuf::from("note.txt"), 0, b"one\ntwo\n");
        let saved = Arc::new(TextSnapshot::from_document(BufferId(1), &document));
        let source = SemanticSource::new(&saved);
        let before = TextSnapshot::conversion_calls_on_thread_for_test();
        let marked = compare_saved(&saved, &source, &document, || true).unwrap();
        assert_eq!(TextSnapshot::conversion_calls_on_thread_for_test(), before, "an identical source version needs no baseline materialization or full line comparison");
        assert!(Arc::ptr_eq(&marked.baseline_text, &saved));
        assert!(marked.hunks.is_empty());
        assert_eq!(marked.blocks.summary(), "no changes");
        assert!(matches!(marked.baseline, Baseline::Saved));
        assert!(compare_saved(&saved, &source, &document, || false).is_err(), "cancellation still wins for an unchanged source");
    }

    #[test]
    fn matching_buffer_and_version_cannot_hide_a_different_source_identity() {
        let original = Document::decode(PathBuf::from("note.txt"), 0, b"before\n");
        let saved = Arc::new(TextSnapshot::from_document(BufferId(1), &original));
        let changed = Document::decode(PathBuf::from("note.txt"), 0, b"after\n");
        let current = TextSnapshot::from_document(BufferId(1), &changed);
        assert_eq!(saved.key(), current.key());
        let before = TextSnapshot::conversion_calls_on_thread_for_test();
        let marked = compare_saved(&saved, &SemanticSource::new(&current), &changed, || true).unwrap();
        assert_eq!(TextSnapshot::conversion_calls_on_thread_for_test(), before + 1);
        assert!(!marked.hunks.is_empty());
        assert_eq!(marked.blocks.summary(), "1 modified");
    }

    #[test]
    fn a_shared_document_in_another_buffer_is_not_the_saved_source_version() {
        let document = Document::decode(PathBuf::from("note.txt"), 0, b"same bytes\n");
        let saved = Arc::new(TextSnapshot::from_document(BufferId(1), &document));
        let other = TextSnapshot::from_document(BufferId(2), &document);
        let before = TextSnapshot::conversion_calls_on_thread_for_test();
        let marked = compare_saved(&saved, &SemanticSource::new(&other), &document, || true).unwrap();
        assert_eq!(TextSnapshot::conversion_calls_on_thread_for_test(), before + 1);
        assert!(marked.hunks.is_empty());
    }
}
