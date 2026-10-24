//! One project-query continuation uses the existing bounded Search result slot.
use super::*;
use std::collections::VecDeque;

pub(super) struct ProjectQuery { pub id: u64, paths: VecDeque<Arc<str>>, step: usize, active: Vec<Arc<str>>, declarations: Option<Declarations> }
type SavedDeclarations = (Arc<[index::Indexed]>, Option<search::DiskSource>);
struct Declarations { saved: std::collections::HashMap<Arc<str>, SavedDeclarations>, best: index::Ranked }
const PATHS_PER_STEP: usize = 32;

impl Session {
    pub(super) fn mark_draft_results_stale(&mut self, current: &crate::text::TextSnapshot) {
        let Some(results) = self.state.results.as_mut() else { return };
        let key = |path: &Path| -> Option<Arc<str>> {
            match &results.root {
                Some((root, _)) => Some(path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/").into()),
                None => Some(path.to_string_lossy().into()),
            }
        };
        for (path, snapshot) in &results.captured {
            if snapshot.key().buffer == current.key().buffer && let Some(path) = key(path) { results.stale.insert(path); }
        }
        if let Some(path) = key(current.source()).filter(|path| results.disk_sources.contains_key(path)) { results.stale.insert(path); }
    }

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

    pub(super) fn start_project_search(&mut self, mut results: Results, mut paths: Vec<Arc<str>>) {
        let id = results.id;
        let Some((root, _)) = results.root.as_ref() else { return };
        results.captured = self.captured_project_sources(root);
        let declarations = (results.kind == ResultsKind::Declarations).then(|| Declarations { saved: self.state.index.snapshot().into_iter().map(|(path, items)| { let source = self.state.index.source(&path); (path, (items, source)) }).collect(), best: Default::default() });
        if let Some(declarations) = &declarations {
            paths.retain(|path| declarations.saved.contains_key(path) || index::DeclIndex::wants(path) && results.captured.contains_key(&root.join(path.as_ref())));
        }
        let stage = if declarations.is_some() { crate::analysis::ResultStage::Declarations } else { crate::analysis::ResultStage::Search };
        let pattern = results.pattern.clone();
        self.state.results = Some(results);
        self.project_query = Some(ProjectQuery { id, paths: VecDeque::new(), step: 0, active: Vec::new(), declarations });
        let fence = self.local_result_fence(crate::analysis::ResultSlot::Search, stage, id, id, None, None);
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
        let declarations = work.declarations.is_some();
        let stage = if declarations { crate::analysis::ResultStage::Declarations } else { crate::analysis::ResultStage::Search };
        let fence = self.local_result_fence(crate::analysis::ResultSlot::Search, stage, id, id, Some(step), snapshot.as_deref());
        if declarations { self.project_declaration_step(id, step, path, snapshot, fence); return; }
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
    fn project_declaration_step(&mut self, id: u64, step: usize, path: Arc<str>, snapshot: Option<Arc<crate::text::TextSnapshot>>, fence: crate::analysis::ResultFence) {
        let Some(results) = self.state.results.as_ref() else { return };
        let Some((root, _)) = results.root.as_ref() else { return };
        let root = root.clone();
        let query = results.query.clone();
        let Some(work) = self.project_query.as_mut() else { return };
        let Some(declarations) = &mut work.declarations else { return };
        let best = std::mem::take(&mut declarations.best);
        if let Some(snapshot) = snapshot {
            let loaded = self.state.sources.buffers.get(&snapshot.key().buffer).map(|e| e.document.clone()).unwrap_or_default();
            let project = fence.project;
            if self.analysis.request_result_document(fence.clone(), Priority::Visible, || (crate::analysis::Input::new(snapshot, loaded, project, false), move |doc: Arc<Document>, lease: crate::analysis::Lease| {
                let mut failed = 0;
                let mut errors = false;
                let next = match index::index_document(&doc, None, lease.cancellation()) {
                    FileIndex::Done { items, syntax_errors, .. } => {
                        errors = syntax_errors;
                        index::rank(&[(path, items)], &query, lease.cancellation())
                    }
                    FileIndex::TooLarge | FileIndex::Unreadable => { failed = 1; Some(Default::default()) }
                };
                if let Some(next) = next { lease.send(Completion::ProjectRanked { id, step, best: index::merge_ranked(best, next, &query), found: search::Found { searched: 1, failed, syntax_errors: errors, ..Default::default() } }); }
            })).is_err() { self.result_failed(&fence, "Declaration analysis capacity is full".into()); }
        } else {
            let mut paths = vec![path];
            while paths.len() < PATHS_PER_STEP && work.paths.front().is_some_and(|path| !results.captured.contains_key(&root.join(path.as_ref()))) {
                if let Some(path) = work.paths.pop_front() { paths.push(path); }
            }
            work.active = paths.clone();
            let mut sources = std::collections::HashMap::new();
            let files: Vec<_> = paths.iter().filter_map(|path| {
                let (items, source) = declarations.saved.remove(path)?;
                if let Some(source) = source { sources.insert(path.clone(), source); }
                Some((path.clone(), items))
            }).collect();
            let searched = paths.len();
            if self.analysis.request_result(fence.clone(), Priority::Visible, || move |lease: crate::analysis::Lease| {
                if let Some(next) = index::rank(&files, &query, lease.cancellation()) {
                    lease.send(Completion::ProjectRanked { id, step, best: index::merge_ranked(best, next, &query), found: search::Found { searched, sources, ..Default::default() } });
                }
            }).is_err() { self.result_failed(&fence, "Declaration analysis capacity is full".into()); }
        }
    }

    pub(super) fn project_ranked(&mut self, id: u64, step: usize, best: index::Ranked, found: search::Found) -> Effect {
        let Some(work) = self.project_query.as_mut().filter(|q| q.id == id && q.step == step) else { return Effect::default() };
        let Some(declarations) = work.declarations.as_mut() else { return Effect::default() };
        let Some(results) = self.state.results.as_mut().filter(|r| r.id == id) else { return Effect::default() };
        results.searched += found.searched; results.failed += found.failed; results.syntax_errors |= found.syntax_errors;
        results.disk_sources.extend(found.sources);
        let selected: std::collections::HashSet<_> = best.0.iter().map(|(path, _)| path.clone()).collect();
        results.disk_sources.retain(|path, _| selected.contains(path));
        if let Some((root, _)) = &results.root {
            for path in &work.active {
                if !selected.contains(path) { results.captured.remove(&root.join(path.as_ref())); }
            }
        }
        if work.paths.is_empty() {
            results.hits = best.0.into_iter().map(|(path, item)| search::Hit::declaration(path, item.line as usize, item.line_start as usize, &item.decl)).collect();
            results.limited = best.1 > index::DECL_LIMIT;
            results.done = true;
            results.rebuild_rows();
            self.project_query = None;
            if !results.hits.is_empty() { return self.select_result(0); }
        } else { declarations.best = best; self.continue_project_search(); }
        Effect { repaint: true, copy: None }
    }

}
