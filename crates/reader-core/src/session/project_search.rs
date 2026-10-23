//! One project-query continuation uses the existing bounded Search result slot.
use super::*;
use std::collections::VecDeque;

pub(super) struct ProjectQuery { pub id: u64, paths: VecDeque<Arc<str>>, step: usize, active: Vec<Arc<str>> }
const PATHS_PER_STEP: usize = 32;

impl Session {
    fn captured_project_sources(&self, root: &Path) -> std::collections::HashMap<PathBuf, Arc<crate::text::TextSnapshot>> {
        let mut captured = std::collections::HashMap::new();
        for (buffer, entry) in &self.state.sources.buffers {
            let Some(snapshot) = entry.snapshot.as_ref().filter(|_| entry.pinned() || self.state.tabs.iter().any(|t| t.loaded && t.buffer == Some(*buffer))) else { continue };
            for path in self.state.sources.aliases.iter().filter(|(_, id)| **id == *buffer).map(|(path, _)| path)
                .chain(self.state.tabs.iter().filter(|t| t.buffer == Some(*buffer)).flat_map(|t| std::iter::once(&t.source).chain(t.canonical.iter())))
                .chain(std::iter::once(&snapshot.source().to_path_buf())) {
                if path.starts_with(root) { captured.insert(path.clone(), snapshot.clone()); }
            }
        }
        captured
    }

    pub(super) fn start_project_search(&mut self, mut results: Results, paths: Vec<Arc<str>>) {
        let id = results.id;
        let Some((root, _)) = results.root.as_ref() else { return };
        results.captured = self.captured_project_sources(root);
        let pattern = results.pattern.clone();
        self.state.results = Some(results);
        self.project_query = Some(ProjectQuery { id, paths: VecDeque::new(), step: 0, active: Vec::new() });
        let fence = self.local_result_fence(crate::analysis::ResultSlot::Search, crate::analysis::ResultStage::Search, id, id, None, None);
        if self.analysis.request_result(fence.clone(), Priority::Visible, || move |lease: crate::analysis::Lease| {
            lease.send(Completion::ProjectListed { id, paths: search::select_files(paths, &pattern) });
        }).is_err() { self.result_failed(&fence, "Search analysis capacity is full".into()); }
    }

    pub(super) fn project_listed(&mut self, id: u64, paths: Result<Vec<Arc<str>>, String>) -> Effect {
        let Some(work) = self.project_query.as_mut().filter(|q| q.id == id) else { return Effect::default() };
        let Some(results) = self.state.results.as_mut().filter(|r| r.id == id) else { return Effect::default() };
        match paths {
            Ok(paths) => {
                results.files = paths.len();
                let Some((root, _)) = results.root.as_ref() else { return Effect::default() };
                let selected: std::collections::HashSet<_> = paths.iter().map(|path| root.join(path.as_ref())).collect();
                results.captured.retain(|path, _| selected.contains(path));
                work.paths = paths.into();
                self.continue_project_search();
            }
            Err(error) => { results.done = true; results.error = Some(error); self.project_query = None; }
        }
        Effect { repaint: true, copy: None }
    }

    fn continue_project_search(&mut self) {
        let Some(work) = self.project_query.as_mut() else { return };
        let Some(results) = self.state.results.as_ref().filter(|r| r.id == work.id) else { self.project_query = None; return };
        let Some((root, _)) = &results.root else { return };
        let Some(path) = work.paths.pop_front() else {
            if let Some(results) = self.state.results.as_mut() { results.done = true; }
            self.project_query = None;
            return;
        };
        work.step += 1;
        work.active = vec![path.clone()];
        let (id, step, root) = (work.id, work.step, root.clone());
        let query = lenz_content::find::Query { text: results.query.clone(), options: results.options };
        let snapshot = results.captured.get(&root.join(path.as_ref())).cloned();
        let fence = self.local_result_fence(crate::analysis::ResultSlot::Search, crate::analysis::ResultStage::Search, id, id, Some(step), snapshot.as_deref());
        if let Some(snapshot) = snapshot {
            let loaded = self.state.sources.buffers.get(&snapshot.key().buffer).map(|e| e.document.clone()).unwrap_or_default();
            let project = fence.project;
            if self.analysis.request_result_document(fence.clone(), Priority::Visible, || (crate::analysis::Input::new(snapshot, loaded, project, false), move |doc: Arc<Document>, lease: crate::analysis::Lease| {
                let found = search::search_text(&query, &path, doc.text());
                lease.send(Completion::ProjectFound { id, step, found });
            })).is_err() { self.result_failed(&fence, "Search analysis capacity is full".into()); }
        } else {
            let mut paths = vec![path];
            if let Some(work) = self.project_query.as_mut() {
                while paths.len() < PATHS_PER_STEP && work.paths.front().is_some_and(|path| !self.state.results.as_ref().is_some_and(|r| r.captured.contains_key(&root.join(path.as_ref())))) {
                    if let Some(path) = work.paths.pop_front() { paths.push(path); }
                }
            }
            if let Some(work) = self.project_query.as_mut() { work.active = paths.clone(); }
            let latest = self.search_latest.clone();
            if self.analysis.request_result(fence.clone(), Priority::Visible, || move |lease: crate::analysis::Lease| {
                let mut combined = search::Found::default();
                search::search_files(&root, &paths, &query, id, &latest, &mut |found| {
                    combined.hits.extend(found.hits); combined.sources.extend(found.sources);
                    combined.searched = found.searched; combined.failed = found.failed;
                    combined.limited = found.limited; combined.error = found.error;
                    lease.wanted()
                });
                lease.send(Completion::ProjectFound { id, step, found: combined });
            }).is_err() { self.result_failed(&fence, "Search analysis capacity is full".into()); }
        }
    }

    pub(super) fn project_found(&mut self, id: u64, step: usize, mut found: search::Found) -> Effect {
        let Some(work) = self.project_query.as_ref().filter(|q| q.id == id && q.step == step) else { return Effect::default() };
        let Some(results) = self.state.results.as_mut().filter(|r| r.id == id) else { return Effect::default() };
        if let Some((root, _)) = &results.root {
            for path in &work.active {
                if !found.hits.iter().any(|hit| hit.path == *path) { results.captured.remove(&root.join(path.as_ref())); }
            }
        }
        let remaining = search::RESULT_LIMIT.saturating_sub(results.hits.len());
        found.limited |= found.hits.len() > remaining || (found.hits.len() >= remaining && !work.paths.is_empty());
        found.hits.truncate(remaining);
        found.searched += results.searched;
        found.failed += results.failed;
        found.done = work.paths.is_empty() || found.limited || found.error.is_some();
        let done = found.done;
        let effect = self.complete_source(Completion::Found { id, found }, None);
        if done { self.project_query = None; } else { self.continue_project_search(); }
        effect
    }
}
