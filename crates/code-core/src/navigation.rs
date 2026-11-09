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
    /// Search Everywhere: Letters in File Names is on.
    pub name_letters: bool,
    /// Search Everywhere: Rank by Open and Recent Files is on.
    pub nearby: bool,
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
    // Where the reader works: the open and recent files (the active one first), relative to the project.
    let nearby = match &snapshot.root {
        Some(root) if snapshot.nearby => crate::project::Nearby::new(snapshot.recent.iter().filter_map(|p| p.strip_prefix(root).ok()?.to_str())),
        _ => crate::project::Nearby::default(),
    };
    let lowered = file_query.to_lowercase();
    let file_score = |rel: &str| crate::project::file_score(&lowered, &rel.to_lowercase(), rel.len(), &nearby, rel).map(|s| s + nearby.recent_boost(rel));
    // A pasted path to an existing file, anywhere on disk (absolute, or `~/` for home; `:line[:column]` kept), is the
    // first row, inside the project or not.
    if matches!(scope, Scope::All | Scope::Files) && let Some(path) = pasted_file(file_query) {
        seen.insert(path.clone());
        let detail = path.to_string_lossy().into_owned();
        ranked.push((i64::MAX, file_row(path, detail, at)));
        matched += 1;
    }
    // With Letters in File Names on, files there also match a query whose characters appear in order in the file name,
    // letters left out (`i_summary` finds `invoice_summary.csv`): a file name is short, so that is not noise.
    let name_letters = snapshot.name_letters;
    let loose_file = |q: &str, path: &str| loose(q, path) || name_letters && lenz_content::fuzzy::name_subsequence(q, path);
    if scope == Scope::Recent {
        for (i, path) in snapshot.recent.iter().take(LIMIT).enumerate() {
            if !current() { return None; }
            let rel = snapshot.root.as_ref().and_then(|root| path.strip_prefix(root).ok()).unwrap_or(path);
            let detail = crate::archive::Location::from_uri(path).map_or_else(|| rel.to_string_lossy(), |location| std::borrow::Cow::Owned(location.display_path()));
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
            if snapshot.root.as_ref().is_some_and(|root| !crate::archive::physical_path(path).starts_with(root)) { continue; }
            let rel = snapshot.root.as_ref().and_then(|root| path.strip_prefix(root).ok()).unwrap_or(path);
            let detail = crate::archive::Location::from_uri(path).map_or_else(|| rel.to_string_lossy(), |location| std::borrow::Cow::Owned(location.display_path()));
            if let Some(score) = file_score(&detail).filter(|_| loose_file(file_query, &detail)) {
                if !seen.insert(path.clone()) { continue; }
                ranked.push((if query.is_empty() { 1_000_000 - i as i64 } else { score }, file_row(path.clone(), detail.into_owned(), at)));
                matched += 1;
            }
        }
        if let Some(root) = &snapshot.root {
            let (files, total) = if file_query.is_empty() {
                let limit = if scope == Scope::All { 20 } else { LIMIT };
                (snapshot.files.iter().flat_map(|b| b.iter().map(|e| e.path.clone())).take(limit).collect(), snapshot.files.iter().map(|b| b.len()).sum())
            } else { crate::project::rank_near(&snapshot.files, file_query, &nearby, &current)? };
            matched += total;
            for path in files {
                let absolute = root.join(&*path);
                if !seen.insert(absolute.clone()) || !loose_file(file_query, &path) { matched = matched.saturating_sub(1); continue; }
                let score = file_score(&path).unwrap_or(0);
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

/// The existing file a pasted query names: an absolute path or one from `~/`, trimmed of spaces and the quotes a shell or
/// a file manager adds (`'…'`, `"…"`), with `.` and `..` resolved. `None` for anything else, a folder or a missing file.
fn pasted_file(query: &str) -> Option<PathBuf> {
    let text = query.trim().trim_matches(|c| c == '\'' || c == '"').trim();
    let path = match text.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest),
        None if text.starts_with('/') => PathBuf::from(text),
        None => return None,
    };
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => { clean.pop(); }
            std::path::Component::CurDir => {}
            other => clean.push(other),
        }
    }
    clean.is_file().then_some(clean)
}

fn file_row(path: PathBuf, detail: String, at: Option<LineColumn>) -> Row {
    let archive = crate::archive::Location::from_uri(&path);
    let label = archive.as_ref().and_then(|location| location.member_name()).and_then(|name| std::path::Path::new(name).file_name())
        .map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.file_name().map_or_else(|| detail.clone(), |name| name.to_string_lossy().into_owned()));
    Row {label, detail, kind: Kind::File, disabled: None, on: None, shortcut: None, target: Target::Open {path, at}}
}

pub use lenz_content::double_shift::DoubleShift;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    fn snapshot() -> Snapshot {
        Snapshot {root: Some("/project".into()), files: vec![vec![Entry::new("src/NetworkParserEngine.rs"),Entry::new("test/NetworkParserEngine.rs"),Entry::new("src/Other.rs")].into()], symbols: Vec::new(), recent: vec!["/project/src/Other.rs".into()], actions: vec![Action {id: CommandId::SoftWrap,label: "Soft Wrap",disabled: None,on: Some(false),shortcut: None}], name_letters: false, nearby: false}
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
    fn everywhere_finds_files_by_letters_in_order_in_their_name_when_that_is_on() {
        let snap = |name_letters| Snapshot {root: Some("/project".into()), files: vec![vec![Entry::new("billing/invoice_summary.csv"), Entry::new("billing/summary.csv"), Entry::new("sums/invoice.csv")].into()], symbols: Vec::new(), recent: Vec::new(), actions: Vec::new(), name_letters, nearby: false};
        for query in ["i_summary", "invc_smry"] {
            assert!(rank(snap(false), Scope::All, query, || true).unwrap().0.is_empty(), "off by default: {query}");
            let (rows, _) = rank(snap(true), Scope::All, query, || true).unwrap();
            assert_eq!(rows.first().map(|r| r.label.as_str()), Some("invoice_summary.csv"), "{query}");
        }
        assert_eq!(rank(snap(false), Scope::All, "invoice_sum", || true).unwrap().0.first().map(|r| r.label.as_str()), Some("invoice_summary.csv"), "word starts as before");
        // Letters spread over the folders and the name are still only for the Files scope.
        assert!(rank(snap(true), Scope::All, "smivc", || true).unwrap().0.is_empty());
        assert_eq!(rank(snap(true), Scope::Files, "smivc", || true).unwrap().0.len(), 1);
    }
    #[test]
    fn with_rank_by_open_files_on_files_near_them_rank_first_and_an_exact_name_leads() {
        let snap = |recent: &[&str], files: &[&str]| Snapshot {root: Some("/project".into()), files: vec![files.iter().map(|f| Entry::new(f)).collect::<Vec<_>>().into()],
            symbols: Vec::new(), recent: recent.iter().map(|r| PathBuf::from(format!("/project/{r}"))).collect(), actions: Vec::new(), name_letters: true, nearby: true};
        let labels = |rows: Vec<Row>| rows.into_iter().map(|r| r.detail).collect::<Vec<_>>();
        let files = ["qa/specs/foo_bar.rb", "dev/app/fizz_bar_helper.rb", "docs/f_alpha.md"];
        // Working nowhere in particular: the closer match first.
        let (rows, _) = rank(snap(&[], &files), Scope::Files, "f_bar", || true).unwrap();
        assert_eq!(labels(rows)[0], "qa/specs/foo_bar.rb");
        // Off (the default), where the reader works changes nothing.
        let (rows, _) = rank(Snapshot {nearby: false, ..snap(&["dev/app/models/user.rb"], &files)}, Scope::Files, "f_bar", || true).unwrap();
        assert_eq!(labels(rows)[0], "qa/specs/foo_bar.rb", "off: by the name alone");
        // On, working under dev/: its slightly weaker match comes first, in the All and Files tabs alike.
        for scope in [Scope::All, Scope::Files] {
            let (rows, _) = rank(snap(&["dev/app/models/user.rb"], &files), scope, "f_bar", || true).unwrap();
            assert_eq!(labels(rows)[..2], ["dev/app/fizz_bar_helper.rb", "qa/specs/foo_bar.rb"], "{scope:?}");
        }
        // A file opened recently counts too, the more recently the more.
        let (rows, _) = rank(snap(&["qa/specs/foo_bar.rb", "dev/app/models/user.rb"], &files), Scope::Files, "f_bar", || true).unwrap();
        assert_eq!(labels(rows)[0], "qa/specs/foo_bar.rb");
        // A name that is the whole query leads wherever it is.
        let with_exact = ["qa/specs/foo_bar.rb", "dev/app/fizz_bar_helper.rb", "qa/f_bar.rb"];
        let (rows, _) = rank(snap(&["dev/app/models/user.rb"], &with_exact), Scope::All, "f_bar", || true).unwrap();
        assert_eq!(labels(rows)[0], "qa/f_bar.rb");
        // A scattered match elsewhere stays below a close one in the open folder, but still listed.
        let (rows, _) = rank(snap(&["dev/app/models/user.rb"], &files), Scope::Files, "f_alpha", || true).unwrap();
        assert_eq!(labels(rows)[0], "docs/f_alpha.md");
    }

    #[test]
    fn a_pasted_path_opens_that_file_from_anywhere_with_its_line() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("elsewhere/notes.md");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, "x\n").unwrap();
        let shown = outside.to_string_lossy().into_owned();
        for (query, at) in [(shown.clone(), None), (format!("  '{shown}' "), None), (format!("{shown}:12"), Some(LineColumn { line: 12, column: None })),
            (format!("{}/elsewhere/../elsewhere/./notes.md:3:4", dir.path().display()), Some(LineColumn { line: 3, column: Some(4) }))] {
            for scope in [Scope::All, Scope::Files] {
                let (rows, count) = rank(snapshot(), scope, &query, || true).unwrap();
                assert!(count >= 1, "{query}");
                assert_eq!(rows[0].label, "notes.md", "{query}");
                assert_eq!(rows[0].detail, shown, "{query}");
                assert!(matches!(&rows[0].target, Target::Open { path, at: a } if *path == outside && *a == at), "{query}: {:?}", rows[0].target);
            }
        }
        // Not a file: a folder, a missing one, a relative name.
        for query in [dir.path().join("elsewhere").to_string_lossy().into_owned(), format!("{shown}.missing"), "elsewhere/notes.md".into()] {
            assert!(rank(snapshot(), Scope::All, &query, || true).unwrap().0.iter().all(|r| r.label != "notes.md"), "{query}");
        }
        assert!(rank(snapshot(), Scope::Symbols, &shown, || true).unwrap().0.is_empty(), "symbols are names, not paths");
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
        let make = || Snapshot {root:Some("/project".into()),files:vec![files.clone()],symbols:vec![(Arc::from("symbols.rs"),symbols.clone())],recent:Vec::new(),actions:Vec::new(),name_letters:false,nearby:false};
        // With the most context ranking uses: ten recent files in different folders.
        let near = || Snapshot {nearby:true,recent:(0..10).map(|i| PathBuf::from(format!("/project/src/module{}/ReaderSearchPanel{}.rs", i * 997, i * 997))).collect(),..make()};
        let mut old = Vec::new(); let mut file = Vec::new(); let mut all = Vec::new(); let mut near_files = Vec::new();
        for trial in 0..12 {
            let start = Instant::now();
            let (paths,matched) = crate::project::rank(std::slice::from_ref(&files),"RSP",||true).unwrap();
            let a = start.elapsed().as_secs_f64()*1000.0;
            assert_eq!(matched,100_000); assert_eq!(paths.len(),crate::project::FINDER_LIMIT);
            let start = Instant::now(); let (rows,matched) = rank(make(),Scope::Files,"RSP",||true).unwrap(); let b = start.elapsed().as_secs_f64()*1000.0;
            assert_eq!(matched,100_000);assert_eq!(rows.len(),LIMIT);assert!(rows.iter().all(|r|r.kind==Kind::File));
            let start = Instant::now(); let (rows,matched) = rank(make(),Scope::All,"RSP",||true).unwrap(); let c = start.elapsed().as_secs_f64()*1000.0;
            assert_eq!(matched,110_000);assert_eq!(rows.len(),LIMIT);assert!(rows.iter().any(|r|r.kind==Kind::Symbol));
            let start = Instant::now(); let (rows,matched) = rank(near(),Scope::Files,"RSP",||true).unwrap(); let d = start.elapsed().as_secs_f64()*1000.0;
            assert_eq!(matched,100_000);assert_eq!(rows.len(),LIMIT);
            if trial>0 {old.push(a);file.push(b);all.push(c);near_files.push(d);}
        }
        let calls=std::cell::Cell::new(0usize);let start=Instant::now();
        assert!(rank(make(),Scope::All,"RSP",||{let n=calls.get();calls.set(n+1);n<5}).is_none());
        let cancelled=start.elapsed().as_secs_f64()*1000.0;
        println!("NAVIGATION_PERF {}",serde_json::json!({"files":100000,"declarations":10000,"old_files_ms":old,"new_files_ms":file,"all_ms":all,"near_files_ms":near_files,"cancelled_ms":cancelled,"cancellation_checks":calls.get()}));
    }
}
