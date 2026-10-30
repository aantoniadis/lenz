//! Explicit, cancellable syntax checking over the project's captured inventory, one bounded file per job.
use super::*;
use crate::semantics::diagnostics::{PublishedDiagnostic, Severity};
use std::{collections::VecDeque, io::Read};

pub(super) struct Work { id: u64, paths: VecDeque<Arc<str>>, step: usize, shell_setting: Option<String>, stats: Stats }
#[derive(Default)]
struct Stats { visited: usize, checked: usize, unsupported: usize, large: usize, failed: usize, limited: usize, skipped: usize }
#[derive(Debug)]
pub struct Reply { id: u64, step: usize, path: Arc<str>, source: Option<SemanticSource>, output: Output }
#[derive(Debug)]
enum Output {
    Checked { items: Vec<Problem>, disk: Option<search::DiskSource>, limited: bool },
    Unsupported,
    Large,
    Failed,
}
#[derive(Debug)]
struct Problem { line: usize, column: usize, message: String, severity: Severity }

impl Session {
    pub(super) fn check_project_syntax(&mut self) -> Effect {
        if self.disabled(CommandId::CheckProjectSyntax).is_some() { return Effect::default(); }
        let Some(project) = &self.state.project else { return Effect::default() };
        let (root, generation, paths, skipped) = (project.root().to_owned(), project.generation(), project.paths(), project.skipped());
        self.search_seq += 1;
        let id = self.search_seq;
        self.search_latest.store(id, Ordering::Relaxed);
        let mut results = self.new_results(id, ResultsKind::ProjectSyntax, String::new(), Scope::Project, Some((root.clone(), generation)), paths.len(), false);
        results.captured = self.captured_project_sources(&root);
        results.query_paths = paths.iter().cloned().collect();
        results.provider = Some(format!("0/{} files; starting; Cancel Project Syntax Check stops", paths.len()));
        self.replace_results(results);
        self.project_query = None;
        self.syntax_scan = Some(Work { id, paths: paths.into(), step: 0, shell_setting: self.state.syntax_shell_setting(), stats: Stats { skipped, ..Default::default() } });
        self.continue_project_syntax();
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn cancel_project_syntax(&mut self, reason: &str) -> Effect {
        let Some(work) = self.syntax_scan.take() else { return Effect::default() };
        self.analysis.cancel_result(crate::analysis::ResultSlot::Search);
        self.syntax_scan_summary(&work, reason);
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn sync_project_syntax(&mut self) {
        let Some(work) = self.syntax_scan.as_ref() else { return };
        if !self.state.results.as_ref().is_some_and(|r| r.id == work.id && r.kind == ResultsKind::ProjectSyntax) {
            self.syntax_scan = None;
        } else if self.state.project.as_ref().is_none_or(|p| self.state.results.as_ref().and_then(|r| r.root.as_ref()).is_none_or(|(root, generation)| root != p.root() || *generation != p.generation())) {
            self.cancel_project_syntax("Stopped; project root changed; rerun to update");
        } else if self.state.results.as_ref().is_some_and(|r| r.done || r.error.is_some() || !r.stale.is_empty()) {
            self.cancel_project_syntax("Stopped; sources changed or checking failed; rerun to update");
        }
    }

    fn syntax_scan_summary(&mut self, work: &Work, status: &str) {
        let Some(results) = self.state.results.as_mut().filter(|r| r.id == work.id) else { return };
        let s = &work.stats;
        results.searched = s.visited;
        results.failed = s.failed;
        results.provider = Some(format!("{status}; {}/{} files, {} checked, {} unsupported, {} too large, {} unreadable/changed, {} limited, {} inventory skips; syntax only", s.visited, results.files, s.checked, s.unsupported, s.large, s.failed, s.limited, s.skipped));
        results.done = status != "Checking";
    }

    fn continue_project_syntax(&mut self) {
        let Some(work) = self.syntax_scan.as_mut() else { return };
        let Some(results) = self.state.results.as_ref().filter(|r| r.id == work.id) else { return };
        let Some((root, _)) = results.root.as_ref() else { return };
        let Some(path) = work.paths.pop_front() else {
            let Some(work) = self.syntax_scan.take() else { return };
            let limited = self.state.results.as_ref().is_some_and(|r| r.limited);
            self.syntax_scan_summary(&work, if limited { "Stopped at problem limit" } else { "Complete over captured inventory" });
            return;
        };
        work.step += 1;
        let (id, step, root, shell_setting) = (work.id, work.step, root.clone(), work.shell_setting.clone());
        let snapshot = results.captured.get(&root.join(path.as_ref())).and_then(|s| s.snapshot.clone());
        let source = snapshot.as_ref().map(|s| SemanticSource::new(s));
        let fence = self.local_result_fence(crate::analysis::ResultSlot::Search, crate::analysis::ResultStage::ProjectSyntax, id, id, Some(step), snapshot.as_deref());
        let admitted = if let Some(snapshot) = snapshot {
            let loaded = self.state.sources.buffers.get(&snapshot.key().buffer).map(|e| e.document.clone()).unwrap_or_default();
            let project = fence.project;
            self.analysis.request_result_document(fence.clone(), Priority::Background, || (crate::analysis::Input::new(snapshot, loaded, project, false), move |doc: Arc<Document>, lease: crate::analysis::Lease| {
                if let Some(output) = check_document(&doc, None, shell_setting.as_deref(), lease.cancellation()) { lease.send(Completion::ProjectSyntax(Reply { id, step, path, source, output })); }
            }))
        } else {
            self.analysis.request_result(fence.clone(), Priority::Background, || move |lease: crate::analysis::Lease| {
                let output = check_file(&root, &path, shell_setting.as_deref(), lease.cancellation());
                if let Some(output) = output { lease.send(Completion::ProjectSyntax(Reply { id, step, path, source: None, output })); }
            })
        };
        if admitted.is_err() { self.result_failed(&fence, "Project syntax analysis capacity is full".into()); }
    }

    pub(super) fn project_syntax_checked(&mut self, reply: Reply) -> Effect {
        let Some(work) = self.syntax_scan.as_ref().filter(|w| w.id == reply.id && w.step == reply.step) else { return Effect::default() };
        let Some(results) = self.state.results.as_ref().filter(|r| r.id == work.id && r.kind == ResultsKind::ProjectSyntax) else { return Effect::default() };
        let Some((root, generation)) = results.root.as_ref() else { return Effect::default() };
        if self.state.project.as_ref().is_none_or(|p| p.generation() != *generation || p.root() != root) { return Effect::default(); }
        let full = root.join(reply.path.as_ref());
        // A draft opened or edited while a disk check ran always wins; never cache the disk report over it.
        if let Some(current) = Self::target_source(&self.state, &full) {
            if reply.source.as_ref().is_none_or(|source| !source.matches(current)) { return self.cancel_project_syntax("Stopped; an open draft changed; rerun to update"); }
        } else if reply.source.is_some() { return self.cancel_project_syntax("Stopped; a captured draft changed; rerun to update"); }
        let output = match reply.output {
            Output::Checked { disk: Some(ref disk), .. } if Stamp::of(&full) != Some(disk.stamp) => Output::Failed,
            output => output,
        };
        let Some(mut work) = self.syntax_scan.take() else { return Effect::default() };
        work.stats.visited += 1;
        match output {
            Output::Checked { items, disk, limited } => {
                work.stats.checked += usize::from(!limited);
                work.stats.limited += usize::from(limited);
                let known: Vec<_> = items.iter().filter(|d| d.severity == Severity::Error).map(|d| PublishedDiagnostic { line: d.line, message: d.message.clone(), severity: d.severity }).collect();
                if known.is_empty() { self.state.syntax_locations.remove(&full); }
                else if self.state.syntax_locations.len() < 1024 || self.state.syntax_locations.contains_key(&full) { self.state.syntax_locations.insert(full.clone(), known); }
                let Some(results) = self.state.results.as_mut() else { return Effect::default() };
                if let Some(disk) = disk { results.disk_sources.insert(reply.path.clone(), disk); }
                let remaining = search::RESULT_LIMIT.saturating_sub(results.hits.len());
                results.limited |= items.len() > remaining;
                results.hits.extend(items.into_iter().take(remaining).map(|d| search::Hit { path: reply.path.clone(), line: d.line, range: d.column..d.column,
                    text: format!("{}: {}", d.severity.label(), d.message).into(), decl: None, origin: None }));
                if remaining == 0 || results.hits.len() == search::RESULT_LIMIT { results.limited = true; work.paths.clear(); }
                if !results.hits.iter().any(|h| h.path == reply.path) {
                    results.disk_sources.remove(&reply.path);
                    if let Some(source) = results.captured.get_mut(&full) { source.snapshot = None; }
                }
                results.rebuild_rows();
            }
            Output::Unsupported => work.stats.unsupported += 1,
            Output::Large => work.stats.large += 1,
            Output::Failed => work.stats.failed += 1,
        }
        self.syntax_scan_summary(&work, "Checking");
        self.syntax_scan = Some(work);
        self.continue_project_syntax();
        Effect { repaint: true, ..Effect::default() }
    }
}

fn check_document(doc: &Document, disk: Option<search::DiskSource>, shell_setting: Option<&str>, current: impl Fn() -> bool + Send + 'static) -> Option<Output> {
    if doc.is_large() { return Some(Output::Large); }
    // SQL's native checker requires the chosen dialect; generic tree-sitter recovery is not dialect validation.
    if !diagnostics_work::syntax_supported(doc, shell_setting) { return Some(Output::Unsupported); }
    let Some(language) = doc.language() else { return Some(Output::Unsupported) };
    if doc.source_meta().is_binary() || doc.source_meta().is_lossy() { return Some(Output::Unsupported); }
    let syntax = highlight::parse_with(language, doc.text().clone(), highlight::Want::default(), current)?;
    let Some(errors) = syntax.errors else { return Some(Output::Unsupported) };
    let limited = errors.len() == 256 || errors.iter().any(|d| d.limited);
    let items = errors.into_iter().map(|d| {
        let line = doc.line_of(d.range.start);
        Problem { line, column: d.range.start - doc.line_range(line).start, message: diagnostics_work::compact_message(&d.message),
            severity: if d.limited { Severity::Information } else { Severity::Error } }
    }).collect();
    Some(Output::Checked { items, disk, limited })
}

fn check_file(root: &Path, path: &str, shell_setting: Option<&str>, current: impl Fn() -> bool + Send + Clone + 'static) -> Option<Output> {
    if !current() { return None; }
    let source = root.join(path);
    let canonical = match source.canonicalize() { Ok(p) if p.starts_with(root) => p, _ => return Some(Output::Failed) };
    if !canonical.metadata().is_ok_and(|m| m.is_file()) { return Some(Output::Failed); }
    let mut file = match std::fs::File::open(&canonical) { Ok(f) => f, Err(_) => return Some(Output::Failed) };
    let before = match file.metadata() { Ok(m) if m.is_file() => Stamp::from(&m), _ => return Some(Output::Failed) };
    if before.len > crate::document::LARGE_BYTES as u64 { return Some(Output::Large); }
    let mut bytes = Vec::with_capacity(before.len as usize);
    let mut chunk = [0u8; 65_536];
    loop {
        if !current() { return None; }
        let capacity = (crate::document::LARGE_BYTES + 1 - bytes.len()).min(chunk.len());
        match file.read(&mut chunk[..capacity]) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
            Err(_) => return Some(Output::Failed),
        }
        if bytes.len() > crate::document::LARGE_BYTES { return Some(Output::Large); }
    }
    if file.metadata().ok().as_ref().map(Stamp::from) != Some(before) || Stamp::of(&source) != Some(before) || source.canonicalize().ok().as_ref() != Some(&canonical) { return Some(Output::Failed); }
    // Avoid raster/SVG decoding for a syntax scan; image source has no supported grammar here.
    let binary = lenz_content::encoding::looks_binary(&bytes);
    if lenz_content::image::detect_file(&source.to_string_lossy(), &bytes, binary).is_some() { return Some(Output::Unsupported); }
    let doc = Document::decode(source.clone(), 0, &bytes);
    let disk = search::DiskSource::new(before, doc.text().as_bytes());
    let output = check_document(&doc, Some(disk), shell_setting, current.clone())?;
    if !current() { return None; }
    if Stamp::of(&source) != Some(before) { return Some(Output::Failed); }
    Some(output)
}
