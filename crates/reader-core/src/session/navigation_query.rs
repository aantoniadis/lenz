//! Independent modal symbol queries share source admission and the bounded declaration records.
use super::*;
use std::collections::{HashMap, VecDeque};
use crate::navigation::{self, Snapshot};

type Saved = (Arc<[index::Indexed]>, Option<search::DiskSource>);
pub(super) struct NavigationQuery {
    pub request: u64,
    step: usize,
    paths: VecDeque<Arc<str>>,
    saved: HashMap<Arc<str>, Saved>,
    captured: HashMap<PathBuf, search::CapturedSource>,
    sources: HashMap<Arc<str>, search::DiskSource>,
    best: index::Ranked,
    input: Option<Snapshot>,
    root: PathBuf,
    query: String,
    scope: navigation::Scope,
    files: usize,
    indexed: usize,
    ready: bool,
    active: Option<Arc<str>>,
}

impl Session {
    pub(super) fn start_navigation_query(&mut self, request: u64, files: usize, indexed: usize, mut input: Snapshot, scope: navigation::Scope, query: String) {
        let Some(root) = input.root.clone() else { return };
        let captured = self.captured_project_sources(&root);
        let saved: HashMap<_, _> = std::mem::take(&mut input.symbols).into_iter().map(|(path, items)| {
            let source = self.state.index.source(&path); (path, (items, source))
        }).collect();
        let mut paths: Vec<_> = saved.keys().cloned().collect();
        for path in captured.keys().filter_map(|path| path.strip_prefix(&root).ok()).map(|path| Arc::<str>::from(path.to_string_lossy().replace('\\', "/"))) {
            if DeclIndex::wants(&path) && !saved.contains_key(&path) { paths.push(path); }
        }
        self.navigation_query = Some(NavigationQuery { request, step: 0, paths: paths.into(), saved, captured, sources: HashMap::new(), best: Default::default(), input: Some(input), root, query, scope, files, indexed, ready: false, active: None });
        self.continue_navigation_query();
    }

    fn navigation_fence(&self, request: u64, stage: crate::analysis::ResultStage, node: Option<usize>) -> crate::analysis::ResultFence {
        crate::analysis::ResultFence { slot: crate::analysis::ResultSlot::Navigation, project: self.project_generation.load(Ordering::Acquire), request, stage, result_set: Some(request), node, source: None }
    }

    fn continue_navigation_query(&mut self) {
        let Some(work) = self.navigation_query.as_mut() else { return };
        let request = work.request;
        work.step += 1;
        let step = work.step;
        let Some(path) = work.paths.pop_front() else {
            let Some(mut input) = work.input.take() else { return };
            let best = std::mem::take(&mut work.best);
            let total = best.1;
            input.symbols = best.0.into_iter().map(|(path, item)| (path, Arc::from([item]))).collect();
            work.saved.clear();
            let (scope, query, files, indexed) = (work.scope, work.query.clone(), work.files, work.indexed);
            let fence = self.navigation_fence(request, crate::analysis::ResultStage::NavigationRank, Some(step));
            if self.analysis.request_result(fence.clone(), Priority::Visible, || move |lease: crate::analysis::Lease| {
                let found = navigation::rank_with_total(input, scope, &query, lease.cancellation(), Some(total));
                lease.send(Completion::Navigation { request, files, indexed, found });
            }).is_err() { self.result_failed(&fence, "Symbol search capacity is full".into()); }
            return;
        };
        work.active = Some(path.clone());
        let snapshot = work.captured.get(&work.root.join(path.as_ref())).and_then(|source| source.snapshot.clone());
        let best = std::mem::take(&mut work.best);
        let query = work.query.clone();
        let cached = snapshot.as_ref().and_then(|source| self.state.index.scoped(&path, source));
        if let Some(snapshot) = snapshot.filter(|_| cached.is_none()) {
            work.saved.remove(&path);
            let entry = self.state.sources.buffers.get(&snapshot.key().buffer);
            let loaded = entry.map(|entry| entry.document.clone()).unwrap_or_default();
            let stamp = entry.filter(|entry| entry.saved.as_ref().is_some_and(|saved| saved.matches(&snapshot))).and_then(|entry| entry.stamp);
            let fence = self.navigation_fence(request, crate::analysis::ResultStage::NavigationRank, Some(step));
            let project = fence.project;
            if self.analysis.request_result_document(fence.clone(), Priority::Visible, || (crate::analysis::Input::new(snapshot, loaded, project, false), move |doc: Arc<Document>, lease: crate::analysis::Lease| {
                let cache = index::index_document(&doc, stamp, lease.cancellation());
                let files = match &cache { FileIndex::Done { items, .. } => vec![(path.clone(), items.clone())], _ => Vec::new() };
                if let Some(next) = index::rank(&files, &query, lease.cancellation()) {
                    lease.send(Completion::NavigationOverlay { request, step, best: index::merge_ranked(best, next, &query), found: search::Found::default(), cache: Some((path, cache)) });
                }
            })).is_err() { self.result_failed(&fence, "Symbol analysis capacity is full".into()); }
        } else {
            let mut files = Vec::new();
            let mut sources = HashMap::new();
            if let Some((items, _)) = cached { files.push((path.clone(), items)); work.saved.remove(&path); }
            else if let Some((items, source)) = work.saved.remove(&path) {
                if let Some(source) = source { sources.insert(path.clone(), source); }
                files.push((path.clone(), items));
            }
            while files.len() < 32 && work.paths.front().is_some_and(|path| !work.captured.contains_key(&work.root.join(path.as_ref()))) {
                let Some(path) = work.paths.pop_front() else { break };
                if let Some((items, source)) = work.saved.remove(&path) {
                    if let Some(source) = source { sources.insert(path.clone(), source); }
                    files.push((path, items));
                }
            }
            let fence = self.navigation_fence(request, crate::analysis::ResultStage::NavigationRank, Some(step));
            if self.analysis.request_result(fence.clone(), Priority::Visible, || move |lease: crate::analysis::Lease| {
                if let Some(next) = index::rank(&files, &query, lease.cancellation()) {
                    lease.send(Completion::NavigationOverlay { request, step, best: index::merge_ranked(best, next, &query), found: search::Found { sources, ..Default::default() }, cache: None });
                }
            }).is_err() { self.result_failed(&fence, "Symbol ranking capacity is full".into()); }
        }
    }

    pub(super) fn navigation_overlay(&mut self, request: u64, step: usize, best: index::Ranked, found: search::Found, cache: Option<(Arc<str>, FileIndex)>) -> Effect {
        let Some(work) = self.navigation_query.as_mut().filter(|work| work.request == request && work.step == step) else { return Effect::default() };
        if let Some((path, cache)) = cache
            && let Some(source) = work.captured.get(&work.root.join(path.as_ref()))
            && let Some(snapshot) = &source.snapshot
            && Self::captured_result_current(&self.state, snapshot, &work.root.join(path.as_ref()))
        { self.state.index.add_scoped(path, cache, source.identity.clone()); }
        let selected: std::collections::HashSet<_> = best.0.iter().map(|(path, _)| path.clone()).collect();
        work.sources.extend(found.sources);
        work.sources.retain(|path, _| selected.contains(path));
        if let Some(path) = &work.active
            && !selected.contains(path)
            && let Some(source) = work.captured.get_mut(&work.root.join(path.as_ref())) { source.snapshot = None; }
        work.best = best;
        self.continue_navigation_query();
        Effect { repaint: true, copy: None }
    }

    pub(super) fn navigation_ready(&mut self, request: u64) {
        let selected: std::collections::HashSet<_> = self.state.navigation.as_ref().into_iter().flat_map(|navigation| &navigation.rows)
            .filter(|row| row.kind == navigation::Kind::Symbol).filter_map(|row| match &row.target { navigation::Target::Open { path, .. } => Some(path.clone()), _ => None }).collect();
        if let Some(work) = self.navigation_query.as_mut().filter(|work| work.request == request) {
            work.ready = true;
            for (path, source) in &mut work.captured { if !selected.contains(path) { source.snapshot = None; } }
            work.sources.retain(|path, _| selected.contains(&work.root.join(path.as_ref())));
        }
    }

    pub(super) fn activate_navigation(&mut self, revision: u64, index: usize) -> Effect {
        let Some(navigation) = self.state.navigation.as_ref().filter(|n| n.revision == revision && !n.pending) else { return Effect::default() };
        let Some(row) = navigation.rows.get(index) else { return Effect::default() };
        let navigation::Target::Open { path, at: Some(_) } = &row.target else { return Effect::default() };
        let path = path.clone();
        let request = navigation.request;
        let Some(work) = self.navigation_query.as_ref().filter(|work| work.ready && work.request == request) else { return self.navigation_refused("The symbol query changed; search again".into()) };
        let snapshot = work.captured.get(&path).and_then(|source| source.snapshot.clone());
        let source = path.strip_prefix(&work.root).ok().and_then(|relative| work.sources.get(relative.to_string_lossy().as_ref())).cloned();
        if snapshot.as_ref().is_some_and(|snapshot| !Self::captured_result_current(&self.state, snapshot, &path)) {
            return self.navigation_refused("This symbol's source changed; search again".into());
        }
        if snapshot.is_none() && source.is_none() { return self.navigation_refused("This symbol's source is unavailable; search again".into()); }
        let fence = self.navigation_fence(request, crate::analysis::ResultStage::NavigationValidate, Some(index));
        if let Some(navigation) = &mut self.state.navigation { navigation.pending = true; navigation.message = None; }
        if let Some(snapshot) = snapshot {
            let loaded = self.state.sources.buffers.get(&snapshot.key().buffer).map(|entry| entry.document.clone()).unwrap_or_default();
            let project = fence.project;
            if self.analysis.request_result_document(fence.clone(), Priority::Visible, || (crate::analysis::Input::new(snapshot, loaded, project, false), move |doc: Arc<Document>, lease: crate::analysis::Lease| {
                lease.send(Completion::NavigationValidated { request, revision, index, result: Ok((None, doc)) });
            })).is_err() { return self.result_failed(&fence, "Symbol validation capacity is full".into()); }
        } else {
            let versions = self.versions.clone();
            if self.analysis.request_result(fence.clone(), Priority::Visible, || move |lease: crate::analysis::Lease| {
                let result = read(&path).map_err(|_| "The symbol file could not be read; search again".to_string()).and_then(|(stamp, bytes)| {
                    let doc = Arc::new(Document::decode(path, versions.fetch_add(1, Ordering::Relaxed) + 1, &bytes));
                    if source.is_some_and(|source| source.stamp == stamp && source.digest == search::decoded_digest(doc.text().as_bytes())) { Ok((Some(stamp), doc)) }
                    else { Err("This symbol's source changed; search again".into()) }
                });
                lease.send(Completion::NavigationValidated { request, revision, index, result });
            }).is_err() { return self.result_failed(&fence, "Symbol validation capacity is full".into()); }
        }
        Effect { repaint: true, copy: None }
    }

    pub(super) fn navigation_refused(&mut self, message: String) -> Effect {
        if let Some(navigation) = &mut self.state.navigation { navigation.pending = false; navigation.message = Some(message); }
        Effect { repaint: true, copy: None }
    }

    pub(super) fn navigation_validated(&mut self, request: u64, revision: u64, index: usize, result: Result<(Option<Stamp>, Arc<Document>), String>) -> Effect {
        let Some(navigation) = self.state.navigation.as_ref().filter(|n| n.request == request && n.revision == revision) else { return Effect::default() };
        let Some(row) = navigation.rows.get(index) else { return Effect::default() };
        let navigation::Target::Open { path, at: Some(at) } = &row.target else { return Effect::default() };
        let (path, at) = (path.clone(), *at);
        let snapshot = self.navigation_query.as_ref().and_then(|work| work.captured.get(&path)).and_then(|source| source.snapshot.as_ref());
        if snapshot.is_some_and(|snapshot| !Self::captured_result_current(&self.state, snapshot, &path)) { return self.navigation_refused("This symbol's source changed; search again".into()); }
        let (stamp, doc) = match result { Ok(result) => result, Err(message) => return self.navigation_refused(message) };
        if Self::changed_result_source(&self.state, &path, &doc, None) {
            return self.navigation_refused("This symbol's current source changed; search again".into());
        }
        self.command(Command::CloseSearchEverywhere);
        let hit = search::Hit { path: path.to_string_lossy().into(), line: at.line.saturating_sub(1) as usize, range: 0..0, text: "".into(), decl: None, origin: None };
        self.open_at(path, &hit, stamp, doc)
    }
}
