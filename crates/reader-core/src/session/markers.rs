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
    Saved(Arc<TextSnapshot>),
}

/// One draft's lines compared with its baseline, as the worker sends them back.
pub struct Marked {
    /// The baseline it compared against, named so the display never leaves it implicit.
    pub baseline: Baseline,
    pub(crate) blocks: Blocks,
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
        // Closed buffers keep no markers.
        self.state.marker_sets.retain(|buffer, _| self.state.sources.buffers.contains_key(buffer));
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
            (false, _) => self.state.confirmed_baseline(buffer).map(|saved| Source::Saved(saved.clone())),
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
            Ok(Marked { baseline, blocks, head }) => {
                let Some(source) = self.state.sources.buffers.get(&buffer).and_then(|entry| entry.snapshot.as_ref()).map(|snapshot| SemanticSource::new(snapshot)) else { return Effect::default() };
                markers.published = Some(crate::markers::Published { source, baseline, blocks });
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
    let right = lenz_engine::split_lines(document.text());
    match source {
        Source::New { label } => Ok(Marked { baseline: Baseline::New { label }, blocks: crate::markers::blocks(&[], &right), head: None }),
        Source::Saved(saved) => {
            let left = saved.materialize_while(&current).ok_or("The comparison was superseded")?;
            let left = lenz_engine::split_lines(left.text());
            Ok(Marked { baseline: Baseline::Saved, blocks: crate::markers::blocks(&left, &right), head: None })
        }
        Source::Head { file, branch, cached } => {
            if let Some((sha, label, blob)) = cached {
                let left = lenz_engine::split_lines(blob.text());
                let blocks = crate::markers::blocks(&left, &right);
                return Ok(Marked { baseline: Baseline::Head { label: label.clone() }, blocks, head: Some((sha.clone(), label.clone(), blob.clone())) });
            }
            let (root, path) = lenz_content::blame::repository(file)?;
            let sha = lenz_content::branches::head_commit(&root).ok_or("HEAD is unreadable")?;
            let label = format!("HEAD ({})", short(&sha, branch.as_deref()));
            match lenz_content::branches::blob_at_head(&root, &path)? {
                Some(bytes) => {
                    let blob = Document::decode(file.clone(), 0, &bytes);
                    if !current() { return Err("The comparison was superseded".into()); }
                    let left = lenz_engine::split_lines(blob.text());
                    let blocks = crate::markers::blocks(&left, &right);
                    Ok(Marked { baseline: Baseline::Head { label: label.clone() }, blocks, head: Some((sha, label, Arc::new(blob))) })
                }
                // Git has no file at this path: a new file, named as such rather than shown as a modification.
                None => {
                    let label = if lenz_content::branches::tracks(&root, &path) { "new file (not in HEAD)" } else { "new file (untracked)" };
                    Ok(Marked { baseline: Baseline::New { label }, blocks: crate::markers::blocks(&[], &right), head: None })
                }
            }
        }
    }
}

/// `main · 1a2b3c4`, or the commit alone for a file of another repository.
fn short(sha: &str, branch: Option<&str>) -> String {
    let sha = sha.get(..7).unwrap_or(sha);
    branch.map_or_else(|| sha.to_string(), |branch| format!("{branch} · {sha}"))
}
