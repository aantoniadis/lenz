//! Name navigation shared by both adapters. Ranking uses the existing inventory and declaration index;
//! no file is loaded and no language server is started just to search.
use std::{path::PathBuf, sync::Arc};
use lenz_content::goto::LineColumn;
use crate::{index::Indexed, project::Entry, session::{Action, Command, CommandId}};

pub const LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope { #[default] All, Files, Symbols, Actions, Recent }
impl Scope {
    pub const ALL: [Self; 4] = [Self::All, Self::Files, Self::Symbols, Self::Actions];
    pub fn label(self) -> &'static str { match self { Self::All => "All", Self::Files => "Files", Self::Symbols => "Symbols", Self::Actions => "Actions", Self::Recent => "Recent Files" } }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind { File, Symbol, Action }
impl Kind {
    pub fn label(self) -> &'static str { match self { Self::File => "File", Self::Symbol => "Symbol", Self::Action => "Action" } }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub detail: String,
    pub kind: Kind,
    pub disabled: Option<&'static str>,
    pub on: Option<bool>,
    pub shortcut: Option<String>,
    pub(crate) target: Target,
}
impl Row {
    /// Action identity for adapter-specific shortcut presentation.
    pub fn action(&self) -> Option<CommandId> { match &self.target { Target::Action(id) => Some(*id), _ => None } }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target { Open {path: PathBuf, at: Option<LineColumn>}, Recent {path: PathBuf, at: Option<LineColumn>}, Action(CommandId) }
#[derive(Debug)]
pub enum Chosen { Command(Command), Action(CommandId) }
#[derive(Clone, Debug)]
pub struct Search {
    pub scope: Scope,
    pub query: String,
    pub rows: Vec<Row>,
    /// Identity of the displayed rows, checked again before accepting a result.
    pub revision: u64,
    pub pending: bool,
    pub(crate) request: u64,
    pub(crate) dirty: bool,
    pub(crate) matched: usize,
    pub(crate) message: Option<String>,
}

pub(crate) struct Snapshot {
    pub root: Option<PathBuf>,
    pub files: Vec<Arc<[Entry]>>,
    pub symbols: Vec<(Arc<str>, Arc<[Indexed]>)>,
    pub recent: Vec<PathBuf>,
    pub actions: Vec<Action>,
}
pub(crate) fn rank(snapshot: Snapshot, scope: Scope, query: &str, current: impl Fn() -> bool) -> Option<(Vec<Row>, usize)> {
    rank_with_total(snapshot, scope, query, current, None)
}
pub(crate) fn rank_with_total(snapshot: Snapshot, scope: Scope, query: &str, current: impl Fn() -> bool, symbol_total: Option<usize>) -> Option<(Vec<Row>, usize)> {
    if !current() { return None; }
    let mut ranked = Vec::new();
    let mut matched = 0;
    let mut seen = std::collections::HashSet::new();
    let (file_query, at) = lenz_content::goto::split_path_line(query);
    // The mixed list keeps word-start matches only; a single scope stays fully fuzzy. ponytail: the file count past the
    // ranked cap still includes scattered matches; filter inside project::rank if that total must be exact.
    let loose = |q: &str, text: &str| scope != Scope::All || lenz_content::fuzzy::word_starts(q, text);
    if scope == Scope::Recent {
        for (i, path) in snapshot.recent.iter().take(LIMIT).enumerate() {
            if !current() { return None; }
            let rel = snapshot.root.as_ref().and_then(|root| path.strip_prefix(root).ok()).unwrap_or(path);
            let detail = rel.to_string_lossy();
            if let Some(score) = lenz_content::fuzzy::score(file_query, &detail) {
                if !seen.insert(path.clone()) { continue; }
                let mut row = file_row(path.clone(), detail.into_owned(), at);
                row.target = Target::Recent {path:path.clone(),at};
                ranked.push((if file_query.is_empty() {1_000_000-i as i64} else {score},row));
                matched += 1;
            }
        }
    }
    if matches!(scope, Scope::All | Scope::Files) {
        for (i, path) in snapshot.recent.iter().take(LIMIT).enumerate() {
            if snapshot.root.as_ref().is_some_and(|root| !path.starts_with(root)) { continue; }
            let rel = snapshot.root.as_ref().and_then(|root| path.strip_prefix(root).ok()).unwrap_or(path);
            let detail = rel.to_string_lossy();
            if let Some(score) = lenz_content::fuzzy::score(file_query, &detail).filter(|_| loose(file_query, &detail)) {
                if !seen.insert(path.clone()) { continue; }
                ranked.push((if query.is_empty() { 1_000_000 - i as i64 } else { score }, file_row(path.clone(), detail.into_owned(), at)));
                matched += 1;
            }
        }
        if let Some(root) = &snapshot.root {
            let (files, total) = if file_query.is_empty() {
                let limit = if scope == Scope::All { 20 } else { LIMIT };
                (snapshot.files.iter().flat_map(|b| b.iter().map(|e| e.path.clone())).take(limit).collect(), snapshot.files.iter().map(|b| b.len()).sum())
            } else { crate::project::rank(&snapshot.files, file_query, &current)? };
            matched += total;
            for path in files {
                let absolute = root.join(&*path);
                if !seen.insert(absolute.clone()) || !loose(file_query, &path) { matched = matched.saturating_sub(1); continue; }
                let score = lenz_content::fuzzy::score(file_query, &path).unwrap_or(0);
                ranked.push((score, file_row(absolute, path.to_string(), at)));
            }
        }
    }
    if matches!(scope, Scope::All | Scope::Symbols) && !query.is_empty() && let Some(root) = &snapshot.root {
        let (symbols, total) = crate::index::rank(&snapshot.symbols, query, &current)?;
        matched += symbol_total.unwrap_or(total);
        for (path, item) in symbols {
            let label = item.decl.name.to_string();
            if !loose(query, &label) { matched = matched.saturating_sub(1); continue; }
            let score = lenz_content::fuzzy::score(query, &label).unwrap_or(0);
            ranked.push((score, Row {label, detail: format!("{} · {}:{}", item.decl.kind.label(), path, item.line + 1), kind: Kind::Symbol, disabled: None, on: None, shortcut: None,
                target: Target::Open {path: root.join(&*path), at: Some(LineColumn {line: item.line.saturating_add(1), column: None})}}));
        }
    }
    if matches!(scope, Scope::All | Scope::Actions) {
        for action in snapshot.actions {
            let score = lenz_content::fuzzy::score(query, action.label).or_else(|| lenz_content::fuzzy::score(query, &format!("{} {}", action.id.group().label(), action.label)));
            if let Some(score) = score.filter(|_| loose(query, action.label) || loose(query, &format!("{} {}", action.id.group().label(), action.label))) {
                matched += 1;
                ranked.push((score, Row {label: action.label.into(), detail: action.id.group().label().into(), kind: Kind::Action, disabled: action.disabled, on: action.on, shortcut: action.shortcut.clone(), target: Target::Action(action.id)}));
            }
        }
    }
    if !current() { return None; }
    ranked.sort_by(|a,b| b.0.cmp(&a.0).then(a.1.kind.cmp(&b.1.kind)).then(a.1.label.cmp(&b.1.label)).then(a.1.detail.cmp(&b.1.detail)));
    // Commands are few and every one must stay reachable by scrolling; files and symbols are capped.
    if scope != Scope::Actions { ranked.truncate(LIMIT); }
    Some((ranked.into_iter().map(|(_,row)| row).collect(), matched))
}

fn file_row(path: PathBuf, detail: String, at: Option<LineColumn>) -> Row {
    Row {label: path.file_name().map_or_else(|| detail.clone(), |s| s.to_string_lossy().into_owned()), detail, kind: Kind::File, disabled: None, on: None, shortcut: None, target: Target::Open {path, at}}
}

pub use lenz_content::double_shift::DoubleShift;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    fn snapshot() -> Snapshot {
        Snapshot {root: Some("/project".into()), files: vec![vec![Entry::new("src/NetworkParserEngine.rs"),Entry::new("test/NetworkParserEngine.rs"),Entry::new("src/Other.rs")].into()], symbols: Vec::new(), recent: vec!["/project/src/Other.rs".into()], actions: vec![Action {id: CommandId::SoftWrap,label: "Soft Wrap",disabled: None,on: Some(false),shortcut: None}]}
    }
    #[test]
    fn filename_subsequence_disambiguates_paths_and_preserves_line_column() {
        let (rows, count) = rank(snapshot(), Scope::Files, "NPE:12:3", || true).unwrap();
        assert_eq!(count, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "NetworkParserEngine.rs");
        assert_eq!(rows[0].action(), None);
        assert_ne!(rows[0].detail, rows[1].detail);
        assert!(matches!(rows[0].target, Target::Open {at: Some(LineColumn {line:12,column:Some(3)}),..}));
    }
    #[test]
    fn actions_are_fuzzy_and_empty_all_prefers_recent_files() {
        let (rows, _) = rank(snapshot(), Scope::Actions, "sftwrp", || true).unwrap();
        assert_eq!(rows[0].label, "Soft Wrap");
        assert_eq!(rows[0].action(), Some(CommandId::SoftWrap));
        assert_eq!(rows[0].on, Some(false));
        let (rows, _) = rank(snapshot(), Scope::All, "", || true).unwrap();
        assert_eq!(rows[0].label, "Other.rs");
        assert!(rows.iter().any(|r| r.kind == Kind::Action));
        assert!(rank(snapshot(), Scope::All, "", || false).is_none());
        // Everywhere lists only word-start matches; the Files and Actions scopes stay fuzzy.
        assert!(rank(snapshot(), Scope::All, "sftwrp", || true).unwrap().0.is_empty());
        let (rows, _) = rank(snapshot(), Scope::All, "sw", || true).unwrap();
        assert_eq!(rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["Soft Wrap"]);
        let (rows, count) = rank(snapshot(), Scope::All, "npe", || true).unwrap();
        assert_eq!((rows.len(), count), (2, 2));
        assert_eq!(rank(snapshot(), Scope::Files, "etw", || true).unwrap().0.len(), 2);
        assert!(rank(snapshot(), Scope::All, "etw", || true).unwrap().0.is_empty());
    }
    #[test]
    #[ignore = "release-only ranking and cancellation benchmark"]
    fn navigation_perf() {
        use lenz_content::highlight::{Declaration,DeclKind};
        let files: Arc<[Entry]> = (0..100_000).map(|i| Entry::new(&format!("src/module{i}/ReaderSearchPanel{i}.rs"))).collect::<Vec<_>>().into();
        let symbols: Arc<[Indexed]> = (0..10_000).map(|i| {
            let name = format!("ReaderSearchPanel{i}");
            Indexed::new(Declaration {kind:DeclKind::Function,name:name.into(),name_range:0..1,range:0..1,first_line:i,last_line:i,depth:0,parent:None},i,0)
        }).collect::<Vec<_>>().into();
        let make = || Snapshot {root:Some("/project".into()),files:vec![files.clone()],symbols:vec![(Arc::from("symbols.rs"),symbols.clone())],recent:Vec::new(),actions:Vec::new()};
        let mut old = Vec::new(); let mut file = Vec::new(); let mut all = Vec::new();
        for trial in 0..12 {
            let start = Instant::now();
            let (paths,matched) = crate::project::rank(std::slice::from_ref(&files),"RSP",||true).unwrap();
            let a = start.elapsed().as_secs_f64()*1000.0;
            assert_eq!(matched,100_000); assert_eq!(paths.len(),crate::project::FINDER_LIMIT);
            let start = Instant::now(); let (rows,matched) = rank(make(),Scope::Files,"RSP",||true).unwrap(); let b = start.elapsed().as_secs_f64()*1000.0;
            assert_eq!(matched,100_000);assert_eq!(rows.len(),LIMIT);assert!(rows.iter().all(|r|r.kind==Kind::File));
            let start = Instant::now(); let (rows,matched) = rank(make(),Scope::All,"RSP",||true).unwrap(); let c = start.elapsed().as_secs_f64()*1000.0;
            assert_eq!(matched,110_000);assert_eq!(rows.len(),LIMIT);assert!(rows.iter().any(|r|r.kind==Kind::Symbol));
            if trial>0 {old.push(a);file.push(b);all.push(c);}
        }
        let calls=std::cell::Cell::new(0usize);let start=Instant::now();
        assert!(rank(make(),Scope::All,"RSP",||{let n=calls.get();calls.set(n+1);n<5}).is_none());
        let cancelled=start.elapsed().as_secs_f64()*1000.0;
        println!("NAVIGATION_PERF {}",serde_json::json!({"files":100000,"declarations":10000,"old_files_ms":old,"new_files_ms":file,"all_ms":all,"cancelled_ms":cancelled,"cancellation_checks":calls.get()}));
    }
}
