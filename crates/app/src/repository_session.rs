//! Shared repository picker state, source rows, filters and selection semantics.
use crate::{
    changeset::{ChangeSet, Spec},
    history::{self, Branches, Log, Pick, Selection as LogSelection},
    repo,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub struct SourcePalette {
    pub fg: u32,
    pub dim: u32,
    pub accent: u32,
    pub status: [u32; 5],
}
impl Default for SourcePalette {
    fn default() -> Self {
        Self {
            fg: 0xd0d0d0,
            dim: 0x808080,
            accent: 0x70aaff,
            status: [0x70aaff, 0xffffff, 0x70cc70, 0xffffff, 0xff8870],
        }
    }
}
/// Which part of the Repository Window takes the keyboard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pane {
    Sidebar,
    Log,
    Diff,
}

/// How long a restarted log (another branch, a refresh) keeps its last rows on screen while it has none yet.
pub const LOG_HOLD: std::time::Duration = std::time::Duration::from_millis(300);
/// How long a new Change Set waits for its first file before the old comparison leaves the screen.
pub const SWAP_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

pub struct Picker {
    /// False in Focus Mode.
    pub shown: bool,
    pub sidebar: bool,
    pub log_shown: bool,
    pub root: PathBuf,
    pub log: Log,
    pub refs: Option<history::Refs>,
    pub(crate) stream: Option<history::Stream>,
    pub loading: bool,
    /// The log stopped reading at the wanted rows; git has more and resumes as the view nears the end.
    pub paused: bool,
    /// Commits to read beyond the furthest row in view or selected.
    pub window: u64,
    pub error: Option<String>,
    /// First display row in view (row 0 is "Uncommitted changes", row r + 1 is log row r).
    pub scroll: f32,
    /// The selection's anchor and second end, as display rows.
    pub anchor: u32,
    pub second: Option<u32>,
    pub range: bool,
    pub merge_parent: usize,
    pub focus: Pane,
    /// Bumped per pick; stale Change Set loads are dropped.
    pub(crate) pick_generation: u64,
    /// A commit to select once its row arrives (HEAD at startup).
    pub want: Option<String>,
    /// Which branches the log walks.
    pub branches: Branches,
    /// How many sidebar rows fit, as last painted.
    pub sources_visible_rows: f32,
    /// Pick the wanted commit's Change Set once it arrives.
    pub pick_on_want: bool,
    /// The log filter as typed, whether it's being typed, and the log rows it keeps (None: no filter).
    pub filter: String,
    pub filter_editing: bool,
    /// Find candidates without changing the committed log selection or filtering rows.
    pub pane_search: crate::pane_search::PaneSearch,
    pub matches: Option<Vec<u32>>,
    /// Commits a git-side filter found: `path:` (required) and a full-message search (replaces the text match).
    pub(crate) path_ids: Option<std::collections::HashSet<String>>,
    pub(crate) grep_ids: Option<std::collections::HashSet<String>>,
    /// File history: each commit's path for the followed file.
    pub file_paths: Option<(String, std::collections::HashMap<String, String>)>,
    /// Branch compare: the other branch, and three-dot (merge base ⟷ tip) or tip ⟷ tip.
    pub compare: Option<(String, bool)>,
    /// When the log started, to offer writing a commit-graph when the first rows are slow.
    pub(crate) started: Option<std::time::Instant>,
    pub graph_notice: bool,
    pub sources: Sources,
    /// Refreshed on activation; painting the title must not launch a git process.
    pub title_branch: String,
    empty_tree: Option<String>,
    history_generation: u64,
    refresh_generation: u64,
    refresh: Option<PickerRefresh>,
}

/// What the sidebar lists, read when the picker first shows.
#[derive(Default)]
pub struct Sources {
    pub local: usize,
    pub branches: Vec<repo::Branch>,
    pub remotes: Vec<repo::Remote>,
    pub tags: Vec<repo::Tag>,
    pub worktrees: Vec<repo::Worktree>,
    pub stashes: Vec<repo::Stash>,
    /// Open pull requests (when a remote is on GitHub and `gh` works), loaded in the background.
    pub prs: Vec<crate::pr::ListedPr>,
    pub branches_more: bool,
    pub branches_loading: bool,
    pub remotes_more: bool,
    pub remotes_loading: bool,
    pub prs_more: bool,
    pub prs_loading: bool,
    pub details_loading: bool,
    pub loaded: bool,
    /// Collapsed sections, by title.
    pub collapsed: std::collections::HashSet<&'static str>,
    pub show_all_branches: bool,
    pub filter: String,
    pub scroll: f32,
    pub selected: usize,
    /// While an operation is in progress: the Conflicts entry at the very top.
    pub conflicts: Option<ConflictsEntry>,
}

/// The sidebar's Conflicts entry.
pub struct ConflictsEntry {
    /// The operation, what's left, and ours or theirs (`conflict_list::entry_text`).
    pub lines: Vec<String>,
    /// Once every file is marked: the command that finishes the operation, copied by a click on its row.
    pub copy: Option<String>,
    /// The Conflict List is the Change Set shown.
    pub shown: bool,
}

/// What a sidebar row does when picked.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceAction {
    Toggle(&'static str),
    /// The Conflict List.
    Conflicts,
    /// Copy a command to the clipboard.
    CopyCommand(String),
    Local,
    AllBranches,
    /// Walk one ref (full name), selecting its tip.
    Branch(String, String),
    /// Select a tagged commit.
    Tag(String),
    Worktree(PathBuf),
    Stash(String),
    ShowAllBranches,
    MorePrs,
    MoreRemotes,
    /// Open pull request number.
    Pr(u64),
}

pub struct SourceRow {
    pub text: String,
    pub color: u32,
    pub indent: f32,
    pub action: Option<SourceAction>,
    pub current: bool,
}

/// Sections that start collapsed.
/// Commits the log reads beyond the furthest row in view before pausing git.
pub const HISTORY_WINDOW: u64 = 50_000;

const COLLAPSED: [&str; 3] = ["Remotes", "Tags", "Stashes"];
pub const SOURCE_BATCH: usize = 20;

impl Sources {
    pub fn read(root: &Path) -> Sources {
        Self::read_limited(root,SOURCE_BATCH)
    }
    fn read_limited(root:&Path,limit:usize)->Sources {
        let mut sources=Self::read_details(root,SOURCE_BATCH);
        sources.set_branches(repo::branches_limited(root,limit.saturating_add(1)).unwrap_or_default(),limit);
        sources
    }
    fn read_details(root:&Path,remote_limit:usize)->Sources {
        let worktrees = repo::worktrees(root).unwrap_or_default();
        let (remotes,remotes_more)=repo::remotes_limited(root,remote_limit).unwrap_or_default();
        Sources {
            local: worktrees
                .iter()
                .find(|w| w.current)
                .and_then(|w| w.dirty)
                .unwrap_or(0),
            remotes,remotes_more,
            tags: repo::tags(root).unwrap_or_default(),
            worktrees,
            stashes: repo::stashes(root).unwrap_or_default(),
            loaded: true,
            collapsed: COLLAPSED.into_iter().collect(),
            ..Default::default()
        }
    }
    fn set_branches(&mut self,mut branches:Vec<repo::Branch>,limit:usize) {
        self.branches_more=branches.len()>limit;
        branches.truncate(limit);
        self.branches=branches;
        self.branches_loading=false;
    }

    /// Read local inventory plus conflict summary without any GitHub calls.
    pub fn read_repository(root: &Path) -> Sources {
        Self::read_repository_limited(root,SOURCE_BATCH,SOURCE_BATCH)
    }
    fn read_repository_limited(root:&Path,limit:usize,remote_limit:usize)->Sources {
        let mut sources=Self::read_repository_details(root,remote_limit);
        sources.set_branches(repo::branches_limited(root,limit.saturating_add(1)).unwrap_or_default(),limit);
        sources
    }
    fn read_repository_details(root:&Path,remote_limit:usize)->Sources {
        let mut sources=Self::read_details(root,remote_limit);
        sources.conflicts=crate::conflict_session::read(root,&[]).ok().flatten().map(|snapshot| {
            let left=snapshot.files.len();
            ConflictsEntry { lines:crate::conflict_session::entry_text(&snapshot.op,left,left+snapshot.marked.len()),
                copy:snapshot.op.operation.continue_command().filter(|_|left==0).map(str::to_string),shown:false }
        });
        sources
    }
    /// Replace inventory while retaining the user's source controls and cached read-only PR list.
    pub fn refresh_with(&mut self, mut fresh: Sources) {
        fresh.filter=std::mem::take(&mut self.filter);
        fresh.collapsed=std::mem::take(&mut self.collapsed);
        fresh.selected=self.selected;fresh.scroll=self.scroll;fresh.show_all_branches=self.show_all_branches;
        fresh.prs=std::mem::take(&mut self.prs);
        fresh.prs_more=self.prs_more;fresh.prs_loading=self.prs_loading;
        *self=fresh;
    }
    /// The sidebar's rows: section headers with counts, then entries, filtered by `filter`.
    pub fn rows_with_palette(&self, walking: &Branches, p: SourcePalette) -> Vec<SourceRow> {
        let f = self.filter.to_lowercase();
        let keep = |name: &str| f.is_empty() || name.to_lowercase().contains(&f);
        let mut out = Vec::new();
        let header = |out: &mut Vec<SourceRow>, title: &'static str, n: usize| {
            let open = !self.collapsed.contains(title);
            let more=match title{"Branches"=>self.branches_more,"Remotes"=>self.remotes_more,"Pull requests"=>self.prs_more,_=>false};
            out.push(SourceRow {
                text: format!(
                    "{} {}  {n}{}",
                    if open { "▾" } else { "▸" },
                    title.to_uppercase(),if more{"+"}else{""}
                ),
                color: p.dim,
                indent: 0.0,
                action: Some(SourceAction::Toggle(title)),
                current: false,
            });
            open
        };
        let entry = |text: String, color: u32, action: SourceAction, current: bool| SourceRow {
            text,
            color,
            indent: 14.0,
            action: Some(action),
            current,
        };
        if let Some(c) = &self.conflicts {
            out.push(SourceRow {
                text: format!("⚠ {}", c.lines[0]),
                color: p.status[4],
                indent: 0.0,
                action: Some(SourceAction::Conflicts),
                current: c.shown,
            });
            for (i, line) in c.lines.iter().enumerate().skip(1) {
                match &c.copy {
                    // too long for one row: the command on its own, copied by a click
                    Some(command) if i == 1 => {
                        out.push(entry(
                            "All resolved — run".into(),
                            p.dim,
                            SourceAction::Conflicts,
                            c.shown,
                        ));
                        out.push(entry(
                            format!("{command} ⧉"),
                            p.accent,
                            SourceAction::CopyCommand(command.clone()),
                            c.shown,
                        ));
                    }
                    _ => out.push(entry(line.clone(), p.dim, SourceAction::Conflicts, c.shown)),
                }
            }
        }
        out.push(entry(
            if self.details_loading{"± Local changes · loading…".into()}else{format!("± Local changes  {}", self.local)},
            p.status[2],
            SourceAction::Local,
            false,
        ));
        let walking_one = |r: &str| matches!(walking, Branches::One(w) if w == r);
        if header(&mut out, "Branches", self.branches.len()) {
            out.push(entry(
                "All branches".into(),
                p.fg,
                SourceAction::AllBranches,
                matches!(walking, Branches::All),
            ));
            let shown: Vec<_> = self.branches.iter().filter(|b| keep(&b.name)).collect();
            let limit = if self.show_all_branches || !f.is_empty() {
                usize::MAX
            } else {
                20
            };
            for b in shown.iter().take(limit) {
                let refname = format!("refs/heads/{}", b.name);
                let mark = if b.current { "● " } else { "⎇ " };
                out.push(entry(
                    format!("{mark}{}", b.name),
                    if b.current { 0xE0C46C } else { p.fg },
                    SourceAction::Branch(refname.clone(), b.sha.clone()),
                    walking_one(&refname),
                ));
            }
            if self.branches_more || self.branches_loading || shown.len() > limit {
                out.push(entry(
                    if self.branches_loading { "Loading branches…".into() } else { "Load more branches…".into() },
                    p.dim,
                    SourceAction::ShowAllBranches,
                    false,
                ));
            }
        }
        let remote_count: usize = self.remotes.iter().map(|r| r.branches.len()).sum();
        if (remote_count > 0||self.remotes_more||self.remotes_loading) && header(&mut out, "Remotes", remote_count) {
            for r in &self.remotes {
                for b in r.branches.iter().filter(|b| keep(&b.name)) {
                    let refname = format!("refs/remotes/{}", b.name);
                    out.push(entry(
                        format!("⎇ {}", b.name),
                        0x9876AA,
                        SourceAction::Branch(refname.clone(), b.sha.clone()),
                        walking_one(&refname),
                    ));
                }
            }
            if self.remotes_more||self.remotes_loading{
                out.push(entry(if self.remotes_loading{"Loading remote branches…"}else{"Load more remote branches…"}.into(),p.dim,SourceAction::MoreRemotes,false));
            }
        }
        if !self.tags.is_empty() && header(&mut out, "Tags", self.tags.len()) {
            for t in self.tags.iter().filter(|t| keep(&t.name)) {
                out.push(entry(
                    format!("◆ {}", t.name),
                    0x6A8759,
                    SourceAction::Tag(t.commit.clone()),
                    false,
                ));
            }
        }
        if self.worktrees.len() > 1 && header(&mut out, "Worktrees", self.worktrees.len()) {
            for w in &self.worktrees {
                let name = w.branch.clone().unwrap_or_else(|| "(detached)".into());
                let dir = w
                    .path
                    .file_name()
                    .map_or(String::new(), |d| d.to_string_lossy().into_owned());
                let dirty = w.dirty.map_or(String::new(), |n| {
                    if n > 0 {
                        format!("  · {n} changed")
                    } else {
                        String::new()
                    }
                });
                if keep(&name) || keep(&dir) {
                    let mark = if w.current { "● " } else { "⧉ " };
                    out.push(entry(
                        format!("{mark}{name}  {dir}{dirty}"),
                        p.fg,
                        SourceAction::Worktree(w.path.clone()),
                        false,
                    ));
                }
            }
        }
        if !self.stashes.is_empty() && header(&mut out, "Stashes", self.stashes.len()) {
            for st in self.stashes.iter().filter(|st| keep(&st.message)) {
                out.push(entry(
                    format!("stash@{{{}}}  {}", st.index, st.message),
                    p.fg,
                    SourceAction::Stash(st.sha.clone()),
                    false,
                ));
            }
        }
        if (!self.prs.is_empty() || self.prs_more || self.prs_loading) && header(&mut out, "Pull requests", self.prs.len()) {
            for pr in self
                .prs
                .iter()
                .filter(|pr| keep(&pr.title) || keep(&pr.number.to_string()))
            {
                let draft = if pr.is_draft { "  (draft)" } else { "" };
                out.push(entry(
                    format!("#{} {}{draft}", pr.number, pr.title),
                    p.status[0],
                    SourceAction::Pr(pr.number),
                    false,
                ));
            }
            if self.prs_more || self.prs_loading {
                out.push(entry(if self.prs_loading {"Loading pull requests…"}else{"Load more pull requests…"}.into(),p.dim,SourceAction::MorePrs,false));
            }
        }
        if self.details_loading {
            out.push(SourceRow{text:"Loading repository details…".into(),color:p.dim,indent:14.,action:None,current:false});
        }
        out
    }
}

/// Identifies the exact history request, including branch/filter/follow changes and ABA input.
#[derive(Clone)]
pub struct HistoryRequest {
    generation: u64,
    root: PathBuf,
    branches: Branches,
    input: String,
    followed: Option<String>,
}
pub enum HistoryAcceptance {
    Stale,
    Filtered,
    Followed { pick: bool },
    Error(String),
}
impl HistoryRequest {
    pub fn read_filter(&self) -> Result<DeepFilter, String> {
        deep_filter(&self.root, &self.branches, &self.input)
    }
    pub fn read_followed(&self) -> Result<Vec<(String,String)>, String> {
        history::file_history(&self.root, &self.branches, self.followed.as_deref().expect("follow request"))
    }
}
impl Picker {
    fn history_request(&self, followed: Option<String>) -> HistoryRequest {
        HistoryRequest { generation:self.history_generation,root:self.root.clone(),branches:self.branches.clone(),input:self.filter.clone(),followed }
    }
    pub fn begin_deep_filter(&mut self) -> HistoryRequest {
        self.cancel_refresh();
        self.history_request(None)
    }
    pub fn begin_file_history(&mut self,path:String) -> HistoryRequest {
        self.cancel_refresh();self.shown=true;self.focus=Pane::Log;
        self.history_request(Some(path))
    }
    fn history_is_current(&self,request:&HistoryRequest)->bool {
        self.history_generation==request.generation&&self.root==request.root&&self.branches==request.branches&&self.filter==request.input
    }
    pub fn accept_deep_filter(&mut self,request:&HistoryRequest,result:Result<DeepFilter,String>)->HistoryAcceptance {
        if !self.history_is_current(request){return HistoryAcceptance::Stale;}
        match result {
            Ok((paths,grep))=>{self.apply_deep_filter(paths,grep);HistoryAcceptance::Filtered},
            Err(error)=>HistoryAcceptance::Error(error),
        }
    }
    pub fn accept_file_history(&mut self,request:&HistoryRequest,result:Result<Vec<(String,String)>,String>)->HistoryAcceptance {
        if !self.history_is_current(request){return HistoryAcceptance::Stale;}
        let commits=match result{Ok(commits)=>commits,Err(error)=>return HistoryAcceptance::Error(error)};
        self.want=commits.first().map(|(id,_)|id.clone());
        self.file_paths=Some((request.followed.clone().expect("follow request"),commits.into_iter().collect()));
        self.refilter();self.scroll=0.0;self.second=None;
        self.pick_on_want=self.want.is_some();
        let row=self.want.as_deref().and_then(|id|self.log.row_of(id)).and_then(|row|self.display_row(row));
        if let Some(row)=row{self.anchor=row;self.want=None;self.pick_on_want=false;}
        HistoryAcceptance::Followed{pick:row.is_some()}
    }
}

/// A quiet repository refresh preserves semantic commit IDs while streamed rows are replaced.
#[derive(Clone)]
pub struct PickerRefresh {
    generation: u64,
    root: PathBuf,
    branches: Branches,
    input: String,
    followed: Option<String>,
    selection: RefreshSelection,
    history: bool,
    filters_ready: bool,
}
pub struct RefreshFilters {
    deep: Result<DeepFilter, String>,
    followed: Option<Result<FollowedHistory, String>>,
}
impl PickerRefresh {
    pub fn refreshes_history(&self)->bool{self.history}
    pub fn read_filters(&self) -> RefreshFilters {
        RefreshFilters {
            deep: if self.input.trim().is_empty(){Ok((None,None))}else{deep_filter(&self.root,&self.branches,&self.input)},
            followed: self.followed.as_ref().map(|path|history::file_history(&self.root,&self.branches,path).map(|commits|(path.clone(),commits))),
        }
    }
}
impl Picker {
    pub fn refreshing(&self) -> bool { self.refresh.is_some() }
    pub fn cancel_refresh(&mut self) { self.history_generation=self.history_generation.wrapping_add(1);self.refresh=None;self.refresh_generation=self.refresh_generation.wrapping_add(1); }
    pub fn begin_repository_refresh(&mut self) -> PickerRefresh {
        let id=|row|self.log_row(row).map(|row|self.log.id(row));
        let selection=self.refresh.as_ref().filter(|_|self.log.is_empty()).map(|plan|plan.selection.clone())
            .unwrap_or_else(||(id(self.anchor),self.second.and_then(id),self.range,self.merge_parent,self.scroll));
        let history=self.shown||!self.log.is_empty()||self.refresh.as_ref().is_some_and(|plan|plan.history);
        if history{self.restart(self.branches.clone(),None);}
        self.refresh_generation=self.refresh_generation.wrapping_add(1);
        let plan=PickerRefresh{generation:self.refresh_generation,root:self.root.clone(),branches:self.branches.clone(),
            input:self.filter.clone(),followed:self.file_paths.as_ref().map(|(path,_)|path.clone()),selection,history,filters_ready:false};
        self.refresh=Some(plan.clone());plan
    }
    pub fn refresh_is_current(&self,plan:&PickerRefresh)->bool{self.refresh_generation==plan.generation&&self.root==plan.root}
    pub fn apply_repository_inventory(&mut self, inventory: RepositoryInventory) -> Option<String> {
        self.root=inventory.root;self.sources.refresh_with(inventory.sources);self.refs=inventory.refs;
        self.title_branch=self.sources.branches.iter().find(|branch|branch.current).map_or("HEAD",|branch|branch.name.as_str()).into();
        inventory.head
    }
    pub fn apply_refresh_filters(&mut self,plan:&PickerRefresh,filters:RefreshFilters)->Vec<String>{
        if !self.refresh_is_current(plan){return Vec::new();}
        let mut errors=Vec::new();
        match filters.deep{Ok((paths,grep))=>self.apply_deep_filter(paths,grep),Err(error)=>errors.push(error)}
        if let Some(followed)=filters.followed{match followed{
            Ok((path,commits))=>{self.file_paths=Some((path,commits.into_iter().collect()));self.refilter();},
            Err(error)=>errors.push(error),
        }}
        if let Some(refresh)=&mut self.refresh{refresh.filters_ready=true;}
        self.finish_repository_refresh();errors
    }
    fn finish_repository_refresh(&mut self)->bool{
        if self.loading||self.refresh.as_ref().is_none_or(|plan|!plan.filters_ready){return false;}
        let plan=self.refresh.take().expect("ready refresh");
        if plan.history{
            let row=|id:&str|self.log.row_of(id).and_then(|row|self.display_row(row));
            let (anchor,second,range,parent,scroll)=plan.selection;
            let (anchor,second)=(anchor.as_deref().and_then(row).unwrap_or(0),second.as_deref().and_then(row));
            self.anchor=anchor;self.second=second;self.range=range;self.merge_parent=parent;self.scroll=scroll;
        }
        true
    }
}

impl Picker {
    pub fn new(root: PathBuf, shown: bool) -> Self {
        Picker {
            shown,
            sidebar: true,
            log_shown: true,
            root,
            log: Log::default(),
            refs: None,
            stream: None,
            loading: false,
            paused: false,
            window: HISTORY_WINDOW,
            error: None,
            scroll: 0.0,
            anchor: 0,
            second: None,
            range: false,
            merge_parent: 0,
            focus: Pane::Log,
            pick_generation: 0,
            want: None,
            branches: Branches::All,
            sources_visible_rows: 20.0,
            pick_on_want: false,
            filter: String::new(),
            filter_editing: false,
            pane_search: Default::default(),
            matches: None,
            path_ids: None,
            grep_ids: None,
            file_paths: None,
            compare: None,
            started: None,
            graph_notice: false,
            sources: Sources{collapsed:COLLAPSED.into_iter().collect(),..Default::default()},
            title_branch: String::new(),
            empty_tree: None,
            history_generation: 0,
            refresh_generation: 0,
            refresh: None,
        }
    }

    /// The log row at display row `row` (row 0 is "Uncommitted changes").
    pub fn log_row(&self, row: u32) -> Option<u32> {
        let i = row.checked_sub(1)?;
        // a filter's rows always name rows of the current log; checked here, where every caller routes
        match &self.matches {
            Some(m) => m.get(i as usize).copied(),
            None => Some(i),
        }.filter(|&i| i < self.log.len())
    }

    /// The display row of log row `i`, if shown.
    pub fn display_row(&self, i: u32) -> Option<u32> {
        match &self.matches {
            Some(m) => m.binary_search(&i).ok().map(|k| k as u32 + 1),
            None => Some(i + 1),
        }
    }

    pub fn pick(&self, row: u32) -> Pick {
        self.log_row(row).map_or(Pick::Uncommitted, Pick::Commit)
    }

    pub fn rows(&self) -> u32 {
        1 + self
            .matches
            .as_ref()
            .map_or(self.log.len(), |m| m.len() as u32)
    }

    /// Bounded search of the visible history, including rows arriving from the stream.
    /// Candidate motion does not alter `anchor`, ranges, or the displayed Change Set.
    pub fn poll_pane_search(&mut self) -> bool {
        let mut search = std::mem::take(&mut self.pane_search);
        let changed = search.poll(self.rows() as usize, |row, query| {
            let Some(i) = self.log_row(row as u32) else {
                return row == 0 && crate::pane_search::contains("Uncommitted changes", query);
            };
            let (name, email) = self.log.author(i);
            crate::pane_search::contains(self.log.subject(i), query)
                || crate::pane_search::contains(name, query)
                || crate::pane_search::contains(email, query)
                || crate::pane_search::contains(&self.log.id(i), query)
        });
        self.pane_search = search;
        changed
    }

    /// Recompute which log rows the filter keeps.
    /// Leave file history and typed filters, rejecting every outstanding history/filter result.
    pub fn clear_filter_and_history(&mut self)->bool{
        self.cancel_refresh();
        let changed=self.file_paths.take().is_some()||!self.filter.is_empty();
        self.filter.clear();self.path_ids=None;self.grep_ids=None;self.want=None;self.pick_on_want=false;self.refilter();self.scroll=0.0;
        changed
    }
    pub fn set_filter(&mut self, input: String) {
        self.cancel_refresh();
        self.filter = input;
        self.path_ids = None;
        self.grep_ids = None;
        self.refilter();
        self.scroll = 0.0;
    }

    pub fn refilter(&mut self) {
        self.pane_search.invalidate();
        let active = !self.filter.trim().is_empty() || self.file_paths.is_some();
        if !active {
            self.matches = None;
            return;
        }
        let mut f = history::Filter::parse(&self.filter)
            .unwrap_or_else(|_| history::Filter::parse("").expect("empty filter parses"));
        if self.grep_ids.is_some() {
            f.text.clear();
        }
        let id_in = |set: &Option<std::collections::HashSet<String>>, i: u32| {
            set.as_ref().is_none_or(|s| s.contains(&self.log.id(i)))
        };
        let files = self.file_paths.as_ref().map(|(_, m)| m);
        self.matches = Some(
            (0..self.log.len())
                .filter(|&i| {
                    f.matches(&self.log, i) && id_in(&self.path_ids, i) && id_in(&self.grep_ids, i)
                })
                .filter(|&i| files.is_none_or(|m| m.contains_key(&self.log.id(i))))
                .collect(),
        );
    }

    pub fn selection(&self) -> LogSelection {
        match self.second {
            Some(b) if self.range => LogSelection::Range(self.pick(self.anchor), self.pick(b)),
            Some(b) => LogSelection::Two(self.pick(self.anchor), self.pick(b)),
            None => LogSelection::One {
                pick: self.pick(self.anchor),
                parent: self.merge_parent,
            },
        }
    }

    /// Parent count of the single selected commit (0 when not exactly one commit is selected).
    pub fn merge_parents(&self) -> usize {
        match (self.second, self.log_row(self.anchor)) {
            (None, Some(i)) => self.log.parents(i).len(),
            _ => 0,
        }
    }

    pub fn selected(&self, row: u32) -> bool {
        match self.second {
            Some(b) if self.range => (self.anchor.min(b)..=self.anchor.max(b)).contains(&row),
            Some(b) => row == self.anchor || row == b,
            None => row == self.anchor,
        }
    }
}

impl Picker {
    /// Compare the semantic selection, including root commits and followed paths.
    pub fn clipboard_text(&self,command:&RepositoryCommand,row:u32)->Option<String>{
        let row=self.log_row(row)?;let id=self.log.id(row);
        Some(match command{RepositoryCommand::CopyHash=>id,RepositoryCommand::CopySubject=>self.log.subject(row).into(),RepositoryCommand::CopyDetails=>{let(author,email)=self.log.author(row);format!("commit {id}\nAuthor: {author} <{email}>\nDate: {}\n\n{}",self.log.time(row),self.log.subject(row))},_=>return None})
    }
    pub fn comparison_spec(&mut self) -> Result<Spec, String> {
        let empty = match &self.empty_tree {
            Some(empty) => empty.clone(),
            None => {
                let empty = history::empty_tree(&self.root)?;
                self.empty_tree = Some(empty.clone());
                empty
            }
        };
        let revs = self.log.revs(self.selection(), &empty);
        let paths = match (
            &self.file_paths,
            self.log_row(self.second.unwrap_or(self.anchor))
                .or(self.log_row(self.anchor)),
        ) {
            (Some((current, versions)), Some(row)) => {
                vec![versions.get(&self.log.id(row)).unwrap_or(current).clone()]
            }
            (Some((current, _)), None) => vec![current.clone()],
            _ => Vec::new(),
        };
        Ok(Spec::Git {
            cached: false,
            untracked: matches!(
                self.selection(),
                LogSelection::One {
                    pick: Pick::Uncommitted,
                    ..
                }
            ),
            revs,
            paths,
        })
    }

    pub fn restart(&mut self, branches: Branches, want: Option<String>) {
        self.cancel_refresh();
        self.pane_search.cancel();
        self.stream = None;
        self.log = Log::default();
        // the filter's or file history's rows named the old log; rebuilt as the new one streams in
        self.refilter();
        self.loading = false;
        self.paused = false;
        self.error = None;
        self.branches = branches;
        self.pick_on_want = want.is_some();
        self.want = want;
        self.scroll = 0.0;
        self.second = None;
    }

    pub fn start_stream(&mut self) {
        self.started = Some(std::time::Instant::now());
        self.paused = false;
        match history::stream_limited(&self.root, &self.branches, self.demand(0.0)) {
            Ok(stream) => {
                self.stream = Some(stream);
                self.loading = true;
            }
            Err(error) => {
                self.loading = false;
                self.error = Some(error);
            }
        }
    }

    /// Commits the log must have read: the furthest row in view or selected plus the window. While a filter,
    /// search, comparison or wanted commit needs more of history, it rolls a window past what has arrived,
    /// so reading continues until that need is met and then pauses again.
    fn demand(&self, viewport_rows: f32) -> u64 {
        let furthest = self.scroll.max(self.anchor as f32).max(self.second.unwrap_or(0) as f32) + viewport_rows.max(0.0);
        let view = (furthest.max(0.0) as u64).saturating_add(self.window);
        let complete = self.want.is_some() || !self.filter.is_empty() || self.matches.is_some() || self.path_ids.is_some()
            || self.grep_ids.is_some() || self.file_paths.is_some() || self.compare.is_some() || self.pane_search.active();
        if complete { view.max(u64::from(self.log.len()).saturating_add(self.window)) } else { view }
    }

    /// Ask the log for the rows the view now needs. True when a paused log starts reading again,
    /// so a frontend that stopped polling must poll again.
    pub fn resume_stream(&mut self, viewport_rows: f32) -> bool {
        let demand = self.demand(viewport_rows);
        let Some(stream) = self.stream.as_ref() else { return false };
        stream.want(demand);
        if self.paused && demand > u64::from(self.log.len()) {
            self.paused = false;
            self.loading = true;
            return true;
        }
        false
    }

    /// Shared wording for a paused log, e.g. in a title: "50000+ commits, more as you scroll".
    pub fn more_label(&self) -> Option<String> {
        self.paused.then(|| format!("{}+ commits, more as you scroll", self.log.len()))
    }

    /// Drain newly streamed commits. Returns (changed, select newly arrived wanted commit).
    pub fn poll_stream(&mut self, viewport_rows: f32) -> (bool, bool) {
        use std::sync::mpsc::TryRecvError;
        self.resume_stream(viewport_rows);
        let Some(stream) = self.stream.as_ref() else {
            return (false, false);
        };
        let (mut changed, mut finished) = (false, false);
        loop {
            match stream.batches.try_recv() {
                Ok(Ok(batch)) => {
                    self.log.extend(batch);
                    changed = true;
                }
                Ok(Err(error)) => {
                    self.error = Some(error);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    changed = true;
                    break;
                }
            }
        }
        if changed && self.matches.is_some() {
            self.refilter();
        }
        let mut pick = false;
        if let Some(row) = self
            .want
            .as_deref()
            .and_then(|id| self.log.row_of(id))
            .and_then(|row| self.display_row(row))
        {
            self.anchor = row;
            self.second = None;
            self.want = None;
            self.scroll = (row as f32 - viewport_rows / 3.0).max(0.0);
            pick = std::mem::take(&mut self.pick_on_want);
            changed = true;
        }
        if self
            .started
            .is_some_and(|time| time.elapsed().as_secs_f32() > 1.0)
            && self.log.is_empty()
            && !self.graph_notice
        {
            self.started = None;
            self.graph_notice = !history::has_commit_graph(&self.root);
            changed |= self.graph_notice;
        }
        if finished {
            self.stream = None;
            self.loading = false;
            self.paused = false;
            // the whole log has arrived without the wanted commit: it is not in this history
            self.want = None;
            self.pick_on_want = false;
        } else if self.loading && self.stream.as_ref().is_some_and(history::Stream::paused) {
            let demand = self.demand(viewport_rows);
            if demand > u64::from(self.log.len()) {
                // Still needed (a rolling demand outran the last limit): keep reading.
                if let Some(stream) = self.stream.as_ref() { stream.want(demand); }
            } else {
                // Every wanted row has arrived: stop polling until the view needs more.
                self.loading = false;
                self.paused = true;
                changed = true;
            }
        }
        changed |= self.finish_repository_refresh();
        (changed, pick)
    }

    pub fn apply_deep_filter(&mut self, paths: Option<Vec<String>>, grep: Option<Vec<String>>) {
        self.path_ids = paths.map(|ids| ids.into_iter().collect());
        self.grep_ids = grep.map(|ids| ids.into_iter().collect());
        self.refilter();
    }
}

pub type DeepFilter = (Option<Vec<String>>, Option<Vec<String>>);

pub fn deep_filter(
    root: &Path,
    branches: &Branches,
    input: &str,
) -> Result<DeepFilter, String> {
    let filter = history::Filter::parse(input)?;
    let path = filter
        .path
        .as_deref()
        .map(|path| history::path_commits(root, branches, path))
        .transpose()?;
    let grep = (!filter.text.is_empty())
        .then(|| history::grep_commits(root, branches, &filter.text))
        .transpose()?;
    Ok((path, grep))
}

pub fn branch_comparison(left: &str, right: &str, merge_base: bool) -> Spec {
    Spec::Git {
        cached: false,
        revs: if merge_base {
            vec![format!("{left}...{right}")]
        } else {
            vec![left.into(), right.into()]
        },
        paths: Vec::new(),
        untracked: false,
    }
}

pub fn local_spec() -> Spec {
    Spec::Git {
        cached: false,
        revs: vec!["HEAD".into()],
        paths: Vec::new(),
        untracked: true,
    }
}
pub fn index_spec() -> Spec {
    Spec::Git {
        cached: true,
        revs: Vec::new(),
        paths: Vec::new(),
        untracked: false,
    }
}
pub fn stash_spec(sha: &str) -> Spec {
    branch_comparison(&format!("{sha}^1"), sha, false)
}

/// Shared decisions after activating a sidebar row. Adapters schedule/present the resulting work.
pub enum SourcePlan {
    None,
    Conflicts,
    Copy(String),
    Local,
    Restart(Branches, Option<String>),
    Select,
    Worktree(PathBuf),
    Stash(String),
    Pr(u64),
    MoreBranches,
    MorePrs,
    MoreRemotes,
}
impl Picker {
    pub fn source_action(&mut self, action: SourceAction) -> SourcePlan {
        if !matches!(&action,SourceAction::Toggle(_)|SourceAction::ShowAllBranches|SourceAction::MorePrs|SourceAction::MoreRemotes|SourceAction::CopyCommand(_)){self.cancel_refresh();}
        match action {
            SourceAction::Toggle(title) => {
                if !self.sources.collapsed.remove(title) {
                    self.sources.collapsed.insert(title);
                }
                SourcePlan::None
            }
            SourceAction::ShowAllBranches => {
                self.sources.show_all_branches = true;
                SourcePlan::MoreBranches
            }
            SourceAction::MorePrs=>SourcePlan::MorePrs,
            SourceAction::MoreRemotes=>SourcePlan::MoreRemotes,
            SourceAction::Conflicts => {
                self.pick_generation += 1;
                SourcePlan::Conflicts
            }
            SourceAction::CopyCommand(text) => SourcePlan::Copy(text),
            SourceAction::Local => {
                self.anchor = 0;
                self.second = None;
                SourcePlan::Local
            }
            SourceAction::AllBranches => SourcePlan::Restart(
                Branches::All,
                self.sources
                    .branches
                    .iter()
                    .find(|branch| branch.current)
                    .map(|branch| branch.sha.clone()),
            ),
            SourceAction::Branch(reference, sha) => {
                SourcePlan::Restart(Branches::One(reference), Some(sha))
            }
            SourceAction::Tag(commit) => match self
                .log
                .row_of(&commit)
                .and_then(|row| self.display_row(row))
            {
                Some(row) => {
                    self.anchor = row;
                    self.second = None;
                    self.scroll = (row as f32 - 4.0).max(0.0);
                    SourcePlan::Select
                }
                None => SourcePlan::Restart(Branches::All, Some(commit)),
            },
            SourceAction::Worktree(path) => SourcePlan::Worktree(path),
            SourceAction::Stash(sha) => SourcePlan::Stash(sha),
            SourceAction::Pr(number) => SourcePlan::Pr(number),
        }
    }
    pub fn select_row(&mut self, row: u32, extend: bool, two: bool) {
        self.cancel_refresh();self.want=None;self.pick_on_want=false;
        let row = row.min(self.rows().saturating_sub(1));
        if extend { self.second = Some(row); self.range = true; }
        else if two { self.second = (row != self.anchor).then_some(row); self.range = false; }
        else { self.anchor = row; self.second = None; self.merge_parent = 0; }
    }
    pub fn move_selection(&mut self, delta: i32, extend: bool, two: bool) {
        self.cancel_refresh();self.want=None;self.pick_on_want=false;
        let from = if extend || two {
            self.second.unwrap_or(self.anchor)
        } else {
            self.anchor
        };
        let target = (i64::from(from) + i64::from(delta))
            .clamp(0, i64::from(self.rows().saturating_sub(1))) as u32;
        if extend || two {
            self.second = Some(target);
            self.range = extend;
        } else {
            self.anchor = target;
            self.second = None;
            self.merge_parent = 0;
        }
    }
}

/// Effects needing a platform adapter. No GitHub write operation exists.
#[derive(Debug, Clone, PartialEq)]
pub enum RepositoryEffect {
    Copy(String),
    OpenUrl(String),
    OpenWorktree(PathBuf),
    /// Open these revisions (`lenz <revs>` in `root`) in a window or view of their own.
    OpenRevisions { root: PathBuf, revs: Vec<String> },
    OpenFile(usize),
    Conflicts,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RepositoryCommand {
    Focus(Pane),
    TogglePicker,
    ToggleSidebar,
    ToggleLog,
    /// Show exactly these picker panes; neither hides the picker.
    SetPanels { branches: bool, history: bool },
    Source(SourceAction),
    Move {
        delta: i32,
        extend: bool,
        two: bool,
    },
    Select,
    Parent(usize),
    Filter(String),
    ApplyFilter,
    ClearFilter,
    SourceFilter(String),
    SourceMove(i32),
    SourceSelect(usize),
    SelectRow { row: u32, extend: bool, two: bool },
    ContextRow(u32),
    CompareRows(u32,u32),
    Scroll { pane: Pane, rows: f32, visible: f32 },
    Walk(Branches),
    Compare {
        left: String,
        right: String,
        merge_base: bool,
    },
    FileHistory(String),
    WriteGraph,
    Index,
    /// The selected commit's changes in a window or view of their own.
    OpenCommit,
    /// Compare two revisions (a commit with a branch).
    CompareRevs(String, String),
    CopyHash,
    CopySubject,
    CopyDetails,
    CopyUrl,
    OpenUrl,
    ShowInGraph,
    RefreshRepository,
    RefreshPr,
    OpenPr(Option<String>),
    ToggleBody,
    PrDetails(crate::pr_session::DetailsTab),
    CloseOverlay,
    Threads {
        path: String,
        outdated: bool,
    },
    Viewed {
        index: usize,
        jump: bool,
    },
    CheckPrHead,
    /// Collapse or expand the commit message panel (`None`), or open or close one commit of a range.
    ToggleMessage(Option<usize>),
    /// Put the commit message panel's keyboard cursor on this commit of a range.
    MessageCursor(usize),
    /// Files changed for one of the PR's commits, or (`None`) for the whole PR.
    PrCommit(Option<String>),
}

/// Labels and availability used by repository command menus in both adapters.
pub fn command_label(command: &RepositoryCommand) -> &'static str {
    match command {
        RepositoryCommand::Focus(_) => "Focus repository pane",
        RepositoryCommand::TogglePicker => "Show/hide Repository Picker",
        RepositoryCommand::ToggleSidebar => "Show/hide Repository Sidebar",
        RepositoryCommand::ToggleLog => "Show/hide Commit Log",
        RepositoryCommand::SetPanels { .. } => "Show or hide repository panes",
        RepositoryCommand::Source(_) | RepositoryCommand::Select => {
            "Open selected source or commit"
        }
        RepositoryCommand::Move { .. } | RepositoryCommand::SourceMove(_) => "Move selection",
        RepositoryCommand::SourceSelect(_) | RepositoryCommand::SelectRow { .. } => "Open selected source or commit",
        RepositoryCommand::ContextRow(_) => "Focus context commit",
        RepositoryCommand::CompareRows(_,0) => "Compare with working tree",
        RepositoryCommand::CompareRows(_,_) => "Compare selected commits",
        RepositoryCommand::Scroll { .. } => "Scroll repository pane",
        RepositoryCommand::Parent(_) => "Compare against next merge parent",
        RepositoryCommand::Filter(_) => "Filter Commit Log",
        RepositoryCommand::ApplyFilter => "Search full messages and paths",
        RepositoryCommand::ClearFilter => "Clear log filter and file history",
        RepositoryCommand::SourceFilter(_) => "Filter repository sources",
        RepositoryCommand::Walk(_) => "Show branch log",
        RepositoryCommand::Compare { .. } => "Compare branch tips or merge base",
        RepositoryCommand::FileHistory(_) => "Follow file history",
        RepositoryCommand::WriteGraph => "Write Git commit graph",
        RepositoryCommand::Index => "Show Staged Changes",
        RepositoryCommand::OpenCommit => "Open in New Window",
        RepositoryCommand::CompareRevs(..) => "Compare commit with branch",
        RepositoryCommand::CopyHash => "Copy Hash",
        RepositoryCommand::CopySubject => "Copy Subject",
        RepositoryCommand::ShowInGraph => "Show selected commit in graph",
        RepositoryCommand::CopyDetails => "Copy commit details",
        RepositoryCommand::CopyUrl => "Copy commit or PR URL",
        RepositoryCommand::OpenUrl => "Open commit or PR URL",
        RepositoryCommand::RefreshRepository => "Refresh repository sources and comparison",
        RepositoryCommand::RefreshPr => "Refresh pull request",
        RepositoryCommand::OpenPr(_) => "Open pull request",
        RepositoryCommand::ToggleBody => "Show PR description",
        RepositoryCommand::PrDetails(tab)=>tab.command_label(),
        RepositoryCommand::CloseOverlay => "Close PR description or review threads",
        RepositoryCommand::Threads {
            outdated: false, ..
        } => "Show file review threads",
        RepositoryCommand::Threads { outdated: true, .. } => "Show outdated review threads",
        RepositoryCommand::Viewed { .. } => "Toggle personal Viewed Mark",
        RepositoryCommand::CheckPrHead => "Check PR head for updates",
        RepositoryCommand::ToggleMessage(_) => "Show / hide commit message",
        RepositoryCommand::MessageCursor(_) => "Choose commit in message",
        RepositoryCommand::PrCommit(_) => "Show a PR commit's files",
    }
}

type Inbox<T> = std::sync::mpsc::Receiver<Option<T>>;
type Job = Box<dyn FnOnce() + Send>;
type Worker = std::sync::OnceLock<std::sync::mpsc::Sender<Job>>;
pub(crate) fn submit<T: Send + 'static>(
    token: &std::sync::Arc<()>,
    job: impl FnOnce() -> T + Send + 'static,
) -> Inbox<T> {
    static WORKER:Worker=Worker::new();
    submit_on(&WORKER,"lenz-repository",token,job)
}
fn submit_inventory<T:Send+'static>(token:&std::sync::Arc<()>,job:impl FnOnce()->T+Send+'static)->Inbox<T>{
    static WORKER:Worker=Worker::new();
    submit_on(&WORKER,"lenz-inventory",token,job)
}
pub(crate) fn submit_github<T:Send+'static>(token:&std::sync::Arc<()>,job:impl FnOnce()->T+Send+'static)->Inbox<T>{
    static WORKER:Worker=Worker::new();
    submit_on(&WORKER,"lenz-github",token,job)
}
fn submit_pr<T:Send+'static>(token:&std::sync::Arc<()>,job:impl FnOnce()->T+Send+'static)->Inbox<T>{
    static WORKER:Worker=Worker::new();
    submit_on(&WORKER,"lenz-pr-fetch",token,job)
}
fn submit_on<T:Send+'static>(worker:&Worker,name:&str,token:&std::sync::Arc<()>,job:impl FnOnce()->T+Send+'static)->Inbox<T>{
    let worker = worker.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                for job in rx {
                    job();
                }
            })
            .expect("start repository worker");
        tx
    });
    let current = std::sync::Arc::downgrade(token);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    worker
        .send(Box::new(move || {
            let _ = tx.send((current.strong_count() > 0).then(job));
        }))
        .expect("repository worker running");
    rx
}
pub(crate) fn receive<T>(inbox: &mut Option<Inbox<T>>) -> Option<T> {
    let result = inbox.as_ref()?.try_recv();
    match result {
        Ok(result) => {
            *inbox = None;
            result
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            *inbox = None;
            None
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
    }
}

pub struct RepositoryInventory {
    sources: Sources,
    refs: Option<history::Refs>,
    head: Option<String>,
    root: PathBuf,
}
type Inventory = RepositoryInventory;
type RefreshSelection = (Option<String>, Option<String>, bool, usize, f32);
type RefreshedInventory = (PickerRefresh, Inventory, RefreshFilters);
type FollowedHistory = (String, Vec<(String, String)>);
type FollowedResult = (HistoryRequest,Result<Vec<(String,String)>,String>);
type BranchBatch = (usize,Result<Vec<repo::Branch>,String>);
type RemoteBatch = Result<(Vec<repo::Remote>,bool),String>;

pub fn read_inventory(cwd: PathBuf) -> RepositoryInventory {
    read_inventory_limited(cwd,SOURCE_BATCH,SOURCE_BATCH)
}
fn read_inventory_limited(cwd:PathBuf,limit:usize,remote_limit:usize)->RepositoryInventory {
    let root=crate::changeset::toplevel(&cwd).unwrap_or(cwd);
    let head=crate::git::run(&root,&["rev-parse","HEAD"]).ok().map(|bytes|String::from_utf8_lossy(&bytes).trim().into());
    RepositoryInventory{sources:Sources::read_repository_limited(&root,limit,remote_limit),refs:history::refs(&root).ok(),head,root}
}
fn read_startup_inventory(cwd:PathBuf)->RepositoryInventory {
    let root=crate::changeset::toplevel(&cwd).unwrap_or(cwd);
    let mut sources=Sources{loaded:true,details_loading:true,collapsed:COLLAPSED.into_iter().collect(),..Default::default()};
    sources.set_branches(repo::branches_limited(&root,SOURCE_BATCH+1).unwrap_or_default(),SOURCE_BATCH);
    let head=sources.branches.iter().find(|b|b.current).map(|b|b.sha.clone());
    RepositoryInventory{sources,refs:None,head,root}
}
fn read_inventory_details(root:PathBuf)->RepositoryInventory {
    let sources=Sources::read_repository_details(&root,SOURCE_BATCH);
    let refs=history::refs(&root).ok();
    let head=refs.as_ref().and_then(history::Refs::head).map(str::to_string);
    RepositoryInventory{sources,refs,head,root}
}

/// Initial log/default-clean view: grouped local changes, otherwise a pinned HEAD commit.
/// Returns the selected commit ID (`None` for local/unborn history); call off the UI thread.
pub fn initial_repository_comparison(root:&Path)->Result<(ChangeSet,Spec,Option<String>),String>{
    initial_log_comparison(root,&local_spec())
}
pub fn initial_log_branches(spec:&Spec)->Branches{
    match spec{Spec::Git{revs,..}=>revs.last().map_or(Branches::All,|revision|Branches::One(revision.clone())),_=>Branches::All}
}
/// Local changes take precedence; a clean log starts at the requested revision's pinned tip.
pub fn initial_log_comparison(root:&Path,requested:&Spec)->Result<(ChangeSet,Spec,Option<String>),String>{
    let local=repo::local_changes(root,false)?;
    if !local.comparisons.is_empty(){return Ok((local,local_spec(),None));}
    let revision=match requested{Spec::Git{revs,..}=>revs.last().map_or("HEAD",String::as_str),_=>"HEAD"};
    match pinned_commit_comparison(root,revision){
        Ok((set,spec,head))=>Ok((set,spec,Some(head))),
        Err(_) if revision=="HEAD"&&crate::git::run(root,&["rev-parse","--verify","HEAD^{commit}"]).is_err()=>Ok((local,local_spec(),None)),
        Err(error)=>Err(error),
    }
}
/// A commit comparison with both endpoints pinned, including a root commit's empty-tree parent.
pub fn pinned_commit_comparison(root:&Path,revision:&str)->Result<(ChangeSet,Spec,String),String>{
    let head=String::from_utf8_lossy(&crate::git::run(root,&["rev-parse","--verify",&format!("{revision}^{{commit}}")])?).trim().to_string();
    let base=crate::blame::parent_or_empty_tree(root,&head)?;
    let spec=branch_comparison(&base,&head,false);let set=ChangeSet::load(&spec,root)?;
    Ok((set,spec,head))
}

/// Reload repository comparisons with the same grouping in both adapters.
/// A clean log's initial GUI set predates its semantic commit spec; pin it on refresh.
pub fn read_refresh_comparison(root:&Path,mut spec:Spec,selected_commit:Option<&str>,stash:bool)->Result<(ChangeSet,Spec),String>{
    if matches!(&spec,Spec::Git{cached:false,revs,paths,..} if revs.is_empty()&&paths.is_empty()){
        spec=if let Some(commit)=selected_commit{
            branch_comparison(&crate::blame::parent_or_empty_tree(root,commit)?,commit,false)
        }else{local_spec()};
    }
    let set=if spec==local_spec(){repo::local_changes(root,false)?}
        else if let Spec::Git{cached:false,revs,paths,..}=&spec
            && stash&&paths.is_empty()&&revs.len()==2&&revs[0]==format!("{}^1",revs[1]){
            repo::stash_changes(root,&revs[1])?
        }else{ChangeSet::load(&spec,root)?};
    Ok((set,spec))
}

pub struct RepositorySession {
    pub picker: Picker,
    pub pr: Option<crate::pr_session::PrState>,
    pub notice: Option<String>,
    pub overlay: Option<String>,
    pub failed: bool,
    effects: Vec<RepositoryEffect>,
    ready: Option<(std::sync::Arc<ChangeSet>, Spec)>,
    current_spec: Option<Spec>,
    /// Back and Forward over the places this view has been.
    nav: crate::navigation::History,
    /// The file and line on screen, as the adapter last reported them.
    position: (Option<String>, Option<u32>),
    /// The file and line a Back or Forward reopens once its comparison settles.
    nav_restore: Option<crate::place::Restore>,
    /// The last command was a log move: a run of moves is one step.
    moving: bool,
    /// Back or Forward is restoring: its own commands are not steps.
    navigating: bool,
    refreshing: Option<Inbox<RefreshedInventory>>,
    pending_refresh: bool,
    refresh_placement: bool,
    set: Option<std::sync::Arc<ChangeSet>>,
    token: std::sync::Arc<()>,
    pick_token: std::sync::Arc<()>,
    filter_token: std::sync::Arc<()>,
    sources: Option<Inbox<Inventory>>,
    source_details: Option<Inbox<Inventory>>,
    branches: Option<Inbox<BranchBatch>>,
    branch_token: std::sync::Arc<()>,
    branch_limit: usize,
    remotes: Option<Inbox<RemoteBatch>>,
    remote_token:std::sync::Arc<()>,
    remote_limit:usize,
    prs: Option<Inbox<Result<Vec<crate::pr::ListedPr>, crate::pr::PrError>>>,
    pending: Option<Inbox<Result<(ChangeSet, Spec), String>>>,
    opened: Option<Inbox<Result<crate::pr_session::Opened, crate::pr::PrError>>>,
    opening_label: String,
    filter: Option<Inbox<(HistoryRequest,Result<DeepFilter, String>)>>,
    followed: Option<Inbox<FollowedResult>>,
    graph: Option<Inbox<Result<(), String>>>,
    moved: Option<Inbox<Result<bool, crate::pr::PrError>>>,
    initial_follow: Option<String>,
    initial_pick: bool,
    head: Option<String>,
    cleanup_pr: bool,
    /// The log stream started with the window, before the sidebar inventory; the inventory must not restart it.
    stream_early: bool,
    /// The commit message panel: what was read, the read in flight, and whether the panel is collapsed.
    message: Option<crate::commit_message::Messages>,
    message_error: Option<String>,
    message_reading: Option<Inbox<Result<crate::commit_message::Messages, String>>>,
    message_token: std::sync::Arc<()>,
    message_collapsed: bool,
    /// The range commit the keyboard is on.
    message_cursor: usize,
}

/// Keys the focused commit message panel interprets through [`RepositorySession::message_key`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKey { Up, Down, Enter }

/// The commit message panel's state for the comparison on screen.
pub enum MessagePanel<'a> { Loading, Failed(&'a str), Ready(&'a crate::commit_message::Messages) }

impl RepositorySession {
    pub fn open(request: &crate::cli::Request, cwd: PathBuf) -> Self {
        use crate::cli::Launch;
        let follow = match &request.launch {
            Launch::Log { follow } => follow.clone(),
            _ => None,
        };
        let branches = matches!(request.launch, Launch::Log { .. }).then(|| initial_log_branches(&request.spec));
        let mut state = Self::start(cwd, !matches!(request.launch, Launch::Diff), branches, follow);
        if let Launch::Pr(target) = &request.launch {
            state.execute(RepositoryCommand::OpenPr(target.clone()));
        } else if state.picker.shown {
            state.stream_first();
            state.initial_pick = true;
            let root = state.picker.root.clone();
            let requested=request.spec.clone();
            state.pending = Some(submit(&state.pick_token, move || {
                initial_log_comparison(&root,&requested).map(|(set,spec,_)|(set,spec))
            }));
        }
        state
    }

    /// The picker for `cwd` and, when it is shown, the sidebar inventory in the background.
    fn start(cwd: PathBuf, shown: bool, branches: Option<Branches>, follow: Option<String>) -> Self {
        let mut picker = Picker::new(cwd.clone(), shown);
        if let Some(branches) = branches {
            picker.branches = branches;
        }
        let mut state = Self::with_picker(picker);
        if shown {
            state.picker.sources.branches_loading=true;
            state.picker.sources.details_loading=true;
            state.sources = Some(submit_inventory(&state.token, move || read_startup_inventory(cwd)));
            state.initial_follow = follow;
        }
        state
    }

    /// The commit list needs nothing from the sidebar's dozen git queries: show it first. A followed file
    /// waits for the inventory, which resolves the repository root.
    fn stream_first(&mut self) {
        if self.initial_follow.is_none() {
            self.picker.start_stream();
            self.stream_early = true;
        }
    }

    /// [`open`](Self::open) for a frontend that already loaded the first comparison, `set` for `spec`
    /// (the GUI decides "no differences" before it opens a window).
    /// Selects commit `id` in the log: now when it has arrived, else as it streams in; its Change Set follows.
    pub fn want_commit(&mut self, id: String) {
        // HEAD is what the log opens on and is already loading: select its row when it arrives, load nothing more.
        // The loaded refs name HEAD without a git process; only before they arrive (a restore at startup) is git asked.
        let head = match self.picker.refs.as_ref().and_then(|r| r.head()) {
            Some(head) => Some(head.to_string()),
            None => crate::git::run(&self.picker.root, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
                .ok().map(|out| String::from_utf8_lossy(&out).trim().to_string()),
        };
        if head.as_deref() == Some(id.as_str()) {
            self.picker.want = Some(id);
            return;
        }
        if let Some(row) = self.picker.log.row_of(&id).and_then(|row| self.picker.display_row(row)) {
            self.picker.anchor = row;
            self.picker.second = None;
            self.execute(RepositoryCommand::Select);
            return;
        }
        self.picker.want = Some(id);
        self.picker.pick_on_want = true;
    }

    /// No wanted commit is outstanding and no Change Set is on its way: a restored file is looked up in the
    /// Change Set the restore asked for, not the one the window opened with.
    /// Whether Branches (the sidebar) and History (the log) are on screen.
    pub fn panels(&self) -> (bool, bool) {
        let p = &self.picker;
        (p.shown && p.sidebar, p.shown && p.log_shown)
    }
    /// The sidebar's selected source row's action (a branch, a worktree, …).
    fn selected_source(&self) -> Option<SourceAction> {
        let p = &self.picker;
        p.sources.rows_with_palette(&p.branches, SourcePalette::default()).get(p.sources.selected).and_then(|r| r.action.clone())
    }
    /// The worktree selected in the sidebar, for opening it in a view of its own.
    pub fn selected_worktree(&self) -> Result<PathBuf, &'static str> {
        match self.selected_source() { Some(SourceAction::Worktree(path)) => Ok(path), _ => Err("Select a worktree in the repository sidebar") }
    }
    /// The repository command a catalog action runs, for the comparison showing file `path`; `Ok(None)` for the
    /// steps adapters take themselves (Back, Forward, Branches…, the history filter, opening a worktree), `Err` why
    /// it cannot run now. Both frontends.
    pub fn action_command(&self, action: crate::catalog::RepoAction, path: &str) -> Result<Option<RepositoryCommand>, &'static str> {
        use crate::catalog::RepoAction as A;
        use RepositoryCommand as R;
        let branch = || match self.selected_source() { Some(SourceAction::Branch(name, _)) => Ok(name), _ => Err("Select a branch in the repository sidebar") };
        let command = match action {
            A::Back | A::Forward | A::Branches | A::Filter => return Ok(None),
            A::OpenWorktree => return self.selected_worktree().map(|_| None),
            A::Refresh => R::RefreshRepository,
            A::StagedChanges => R::Index,
            A::CopyUrl => R::CopyUrl,
            A::OpenUrl => R::OpenUrl,
            A::LocalChanges => R::Source(SourceAction::Local),
            A::AllBranches => R::Walk(Branches::All),
            A::CurrentBranch => R::Walk(Branches::Current),
            A::CompareBranch { merge_base } => R::Compare { left: "HEAD".into(), right: branch()?, merge_base },
            A::CompareCommit => {
                let p = &self.picker;
                let left = p.log_row(p.anchor).map(|row| p.log.id(row)).ok_or("Select a commit in the history")?;
                R::Compare { left, right: branch()?, merge_base: false }
            }
            A::NextParent => R::Parent(self.picker.merge_parent + 1),
            A::FileHistory if path.is_empty() => return Err("No file is shown"),
            A::FileHistory => R::FileHistory(path.into()),
            A::WriteGraph => R::WriteGraph,
            A::CopyHash => R::CopyHash,
            A::CopySubject => R::CopySubject,
            A::CopyDetails => R::CopyDetails,
            A::ShowInGraph => R::ShowInGraph,
            A::ToggleMessage => R::ToggleMessage(None),
        };
        match self.disabled(&command) { Some(why) => Err(why), None => Ok(Some(command)) }
    }
    /// The shown log restarted (another branch, a refresh) and has no rows yet, for at most [`LOG_HOLD`]: frontends
    /// keep its last rows up meanwhile, so the history never blinks empty between a click and the first new rows.
    pub fn holding_log(&self) -> bool {
        let p = &self.picker;
        p.shown && p.log_shown && p.loading && p.log.is_empty() && p.started.is_some_and(|t| t.elapsed() < LOG_HOLD)
    }
    pub fn settled(&self) -> bool {
        self.picker.want.is_none() && self.pending.is_none() && self.ready.is_none()
    }

    /// Whether the log selected `commit`, the one a restore asked for: a commit the log never reached (amended or
    /// rebased away, still in the reflog) leaves its restore unapplied.
    pub fn restored(&self, commit: &str) -> bool {
        matches!(self.place_source(), Some(crate::place::Source::Log { commit: selected }) if selected == commit)
    }

    /// What the log has selected, for the last position: local changes or the anchor commit.
    pub fn place_source(&self) -> Option<crate::place::Source> {
        if !self.picker.shown {
            return None;
        }
        Some(match self.picker.pick(self.picker.anchor) {
            Pick::Uncommitted => crate::place::Source::Local,
            Pick::Commit(row) => crate::place::Source::Log { commit: self.picker.log.id(row) },
        })
    }

    pub fn open_loaded(root: PathBuf, shown: bool, branches: Option<Branches>, follow: Option<String>, set: std::sync::Arc<ChangeSet>, spec: Spec) -> Self {
        let mut state = Self::start(root, shown, branches, follow);
        if shown {
            state.stream_first();
        }
        state.place_initial(&spec);
        if let Some(root) = set.git_root() {
            state.picker.root = root.to_path_buf();
        }
        state.set = Some(set);
        state.set_spec(spec);
        state
    }

    /// Select the commit the first comparison shows, once the log reaches it.
    fn place_initial(&mut self, spec: &Spec) {
        if let Spec::Git { revs, .. } = spec
            && revs.len() == 2
        {
            let selected = revs.last().cloned();
            let arrived = selected.as_deref().and_then(|id| self.picker.log.row_of(id)).and_then(|row| self.picker.display_row(row));
            self.picker.anchor = arrived.unwrap_or(1);
            // wanted only while its row has not arrived: a want the finished log can no longer meet never clears
            self.picker.want = if arrived.is_some() { None } else { selected };
        }
    }

    /// A new comparison request: earlier loads, the session's and the frontend's, become stale.
    fn new_pick(&mut self) {
        self.picker.pick_generation += 1;
        self.pick_token = std::sync::Arc::new(());
        if self.opened.take().is_some()&&self.notice.as_deref()==Some("Fetching pull request…"){self.notice=None;}
    }
    /// Persistent feedback until the request succeeds, fails, or another comparison supersedes it.
    pub fn pr_loading(&self)->Option<&str>{self.opened.as_ref().map(|_|self.opening_label.as_str())}

    /// Review `found`, whose comparison is already accepted or shown: its commits in the log, its review
    /// threads in the background, its fetch refs removed with the session.
    pub fn adopt_pr(&mut self, found: crate::pr::PullRequest, fetched: crate::pr::Fetched) {
        let range = fetched.commit_range();
        let head = fetched.head.clone();
        self.cleanup_pr = true;
        self.pr = Some(crate::pr_session::PrState::new(found, fetched));
        self.restart(Branches::One(range), Some(head));
        self.picker.pick_on_want = false;
        self.picker.compare = None;
        self.picker.file_paths = None;
        self.picker.shown = true;
        let root = self.picker.root.clone();
        let pr = self.pr.as_mut().expect("adopted PR");
        pr.start_review(&root);
        // a PR opens on its Conversation, as on GitHub; frontends show it in Focus
        pr.execute(crate::pr_session::PrCommand::Show(crate::pr_session::DetailsTab::Conversation));
        self.overlay = None;
        self.notice = None;
    }

    /// A sidebar row's right-click entries both frontends list: the open pull request refreshes from its row.
    pub fn source_menu(&self, row: usize) -> Vec<(&'static str, crate::catalog::Action)> {
        let rows = self.picker.sources.rows_with_palette(&self.picker.branches, SourcePalette::default());
        match rows.get(row).and_then(|r| r.action.clone()) {
            Some(SourceAction::Pr(n)) if self.pr.as_ref().is_some_and(|p| p.pr.number == n) =>
                vec![(command_label(&RepositoryCommand::RefreshPr), crate::catalog::Action::Pr(crate::catalog::PrAction::Refresh))],
            _ => Vec::new(),
        }
    }

    /// The log context menu's commands for `row`, in both adapters; `file` is the file on screen.
    pub fn log_context_commands(row: u32, file: &str) -> Vec<RepositoryCommand> {
        use RepositoryCommand as C;
        vec![C::CopyHash, C::CopySubject, C::CopyDetails, C::ShowInGraph, C::CompareRows(row, 0), C::Select, C::FileHistory(file.into()), C::OpenCommit]
    }

    /// "Compare with Branch…" for log `row`: up to 30 local branches, each comparing the commit with it.
    pub fn branch_targets(&self, row: u32) -> Vec<(String, RepositoryCommand)> {
        let Some(id) = self.picker.log_row(row).map(|i| self.picker.log.id(i)) else { return Vec::new() };
        self.picker.sources.branches.iter().take(30)
            .map(|b| (format!("⎇ {}", b.name), RepositoryCommand::CompareRevs(id.clone(), format!("refs/heads/{}", b.name))))
            .collect()
    }

    /// Why a context-menu command cannot run on log `row`.
    pub fn row_disabled(&self, row: u32, command: &RepositoryCommand) -> Option<&'static str> {
        use RepositoryCommand as C;
        if matches!(command, C::CopyHash | C::CopySubject | C::CopyDetails | C::ShowInGraph | C::OpenCommit) {
            return self.picker.log_row(row).is_none().then_some("No commit selected");
        }
        self.disabled(command)
    }

    /// Run a context-menu command on log `row`: the row is selected first, and copying leaves the selection
    /// as it was.
    pub fn execute_on_row(&mut self, row: u32, command: RepositoryCommand) {
        use RepositoryCommand as C;
        let saved = (self.picker.anchor, self.picker.second, self.picker.range);
        let copying = matches!(command, C::CopyHash | C::CopySubject | C::CopyDetails);
        self.execute(C::ContextRow(row));
        self.execute(command);
        if copying {
            (self.picker.anchor, self.picker.second, self.picker.range) = saved;
        }
    }

    /// Whether this session fetched pull-request refs, which go when the session does.
    pub fn fetched_pr_refs(&self) -> bool {
        self.cleanup_pr
    }

    /// The comparison on screen, when the frontend loaded it itself: refresh reloads this one.
    pub fn showing(&mut self, set: std::sync::Arc<ChangeSet>, spec: Spec) {
        self.set = Some(set);
        self.set_spec(spec);
    }

    /// A session over `picker` with no background work started.
    pub fn with_picker(picker: Picker) -> Self {
        Self {
            picker,
            pr: None,
            notice: None,
            overlay: None,
            failed: false,
            effects: Vec::new(),
            ready: None,
            current_spec: None,
            nav: Default::default(),
            position: (None, None),
            nav_restore: None,
            moving: false,
            navigating: false,
            refreshing: None,
            pending_refresh: false,
            refresh_placement: false,
            set: None,
            token: std::sync::Arc::new(()),
            pick_token: std::sync::Arc::new(()),
            filter_token: std::sync::Arc::new(()),
            sources: None,
            source_details:None,
            branches:None,
            branch_token:std::sync::Arc::new(()),
            branch_limit:SOURCE_BATCH,
            remotes:None,
            remote_token:std::sync::Arc::new(()),
            remote_limit:SOURCE_BATCH,
            prs: None,
            pending: None,
            opened: None,
            opening_label:String::new(),
            filter: None,
            followed: None,
            graph: None,
            moved: None,
            initial_follow: None,
            initial_pick: false,
            head: None,
            cleanup_pr: false,
            stream_early: false,
            message: None,
            message_error: None,
            message_reading: None,
            message_token: std::sync::Arc::new(()),
            message_collapsed: false,
            message_cursor: 0,
        }
    }

    pub fn busy(&self) -> bool {
        self.picker.loading
            || self.sources.is_some()
            || self.source_details.is_some()
            || self.branches.is_some()
            || self.remotes.is_some()
            || self.refreshing.is_some()
            || self.prs.is_some()
            || self.pending.is_some()
            || self.opened.is_some()
            || self.pr.as_ref().is_some_and(|p|p.review_loading())
            || self.filter.is_some()
            || self.followed.is_some()
            || self.graph.is_some()
            || self.moved.is_some()
            || self.message_reading.is_some()
    }
    pub fn comparisons(&self) -> &[crate::changeset::Comparison] {
        self.set.as_ref().map_or(&[], |set| &set.comparisons)
    }
    pub fn take_change_set(&mut self) -> Option<(std::sync::Arc<ChangeSet>, Spec)> {
        self.ready.take()
    }
    pub fn take_refresh_placement(&mut self)->bool{std::mem::take(&mut self.refresh_placement)}
    pub fn take_effects(&mut self) -> Vec<RepositoryEffect> {
        std::mem::take(&mut self.effects)
    }
    pub fn disabled(&self, command: &RepositoryCommand) -> Option<&'static str> {
        match command {
            RepositoryCommand::RefreshPr
            | RepositoryCommand::ToggleBody
            | RepositoryCommand::PrDetails(_)
            | RepositoryCommand::Threads { .. }
            | RepositoryCommand::Viewed { .. }
            | RepositoryCommand::CheckPrHead
            | RepositoryCommand::PrCommit(_)
                if self.pr.is_none() =>
            {
                Some("No pull request is open")
            }
            RepositoryCommand::FileHistory(path) if path.is_empty() => Some("No file is selected"),
            RepositoryCommand::CopyHash
            | RepositoryCommand::CopySubject
            | RepositoryCommand::CopyDetails
            | RepositoryCommand::ShowInGraph
                if self.picker.log_row(self.picker.anchor).is_none() =>
            {
                Some("Select a commit first")
            }
            RepositoryCommand::CompareRows(a,b) if a==b||*a>=self.picker.rows()||*b>=self.picker.rows()=>Some("Select two different valid revisions"),
            RepositoryCommand::Parent(_) if self.picker.merge_parents() < 2 => {
                Some("Selected commit has fewer than two parents")
            }
            RepositoryCommand::ToggleMessage(_) | RepositoryCommand::MessageCursor(_) if self.message_panel().is_none() => Some("This comparison has no commit message"),
            RepositoryCommand::MessageCursor(i) | RepositoryCommand::ToggleMessage(Some(i)) if !matches!(self.message_panel(), Some(MessagePanel::Ready(m)) if *i < m.commits.len()) => Some("No such commit"),
            _ => None,
        }
    }
    fn accept(&mut self, set: ChangeSet, spec: Spec) {
        let set = std::sync::Arc::new(set);
        if let Some(root)=set.git_root(){self.picker.root=root.to_path_buf();}
        self.set = Some(set.clone());
        self.set_spec(spec.clone());
        self.ready = Some((set, spec));
    }
    /// Show `spec` and read its commit messages; a read for an earlier comparison is dropped.
    fn set_spec(&mut self, spec: Spec) {
        let query = crate::commit_message::query(&spec);
        self.current_spec = Some(spec);
        self.message_token = std::sync::Arc::new(());
        self.message_reading = None;
        self.message_error = None;
        // a refresh of the same comparison keeps the panel up until the new read arrives
        if self.message.as_ref().map(|m| &m.query) != query.as_ref() { self.message = None; self.message_cursor = 0; }
        if let Some(query) = query {
            let root = self.picker.root.clone();
            self.message_reading = Some(submit(&self.message_token, move || crate::commit_message::read(&root, &query)));
        }
    }
    /// The commit message panel, or `None` when the comparison has no message (working tree, index, a pull request).
    pub fn message_panel(&self) -> Option<MessagePanel<'_>> {
        if self.pr.is_some() { return None; }
        let query = crate::commit_message::query(self.current_spec.as_ref()?)?;
        Some(match (&self.message, &self.message_error) {
            (Some(m), _) if m.query == query => MessagePanel::Ready(m),
            (_, Some(error)) => MessagePanel::Failed(error),
            _ => MessagePanel::Loading,
        })
    }
    pub fn message_collapsed(&self) -> bool { self.message_collapsed }
    /// The range commit the keyboard is on (0 for a single commit).
    pub fn message_cursor(&self) -> usize { self.message_cursor }
    /// What a key does in the focused panel: `Enter` opens the cursor's commit (a single commit collapses), `Up`/`Down`
    /// move the cursor; `None` for a single commit's arrows, which scroll, or past either end of a range.
    pub fn message_key(&self, key: MessageKey) -> Option<RepositoryCommand> {
        let Some(MessagePanel::Ready(m)) = self.message_panel() else { return None };
        let range = matches!(m.query, crate::commit_message::Query::Range(_)) && !self.message_collapsed;
        match key {
            MessageKey::Enter if range => Some(RepositoryCommand::ToggleMessage(Some(self.message_cursor))),
            MessageKey::Enter => Some(RepositoryCommand::ToggleMessage(None)),
            MessageKey::Up if range && self.message_cursor > 0 => Some(RepositoryCommand::MessageCursor(self.message_cursor - 1)),
            MessageKey::Down if range && self.message_cursor + 1 < m.commits.len() => Some(RepositoryCommand::MessageCursor(self.message_cursor + 1)),
            MessageKey::Up | MessageKey::Down => None,
        }
    }
    /// The panel's lines wrapped to `columns`, for both frontends to draw.
    pub fn message_lines(&self, columns: usize) -> Option<Vec<crate::commit_message::Line>> {
        use crate::commit_message::{Line, LineKind};
        let note = |text: &str| vec![Line { text: text.into(), kind: LineKind::Meta }];
        Some(match self.message_panel()? {
            MessagePanel::Loading => note("Reading the commit message…"),
            MessagePanel::Failed(error) => note(error),
            MessagePanel::Ready(m) => m.lines(self.message_collapsed, columns),
        })
    }
    fn load(&mut self, spec: Spec) {
        self.initial_pick = false;
        self.pending_refresh = false;
        self.new_pick();
        let root = self.picker.root.clone();
        self.opened = None;
        self.pending = Some(submit(&self.pick_token, move || {
            ChangeSet::load(&spec, &root).map(|set| (set, spec))
        }));
    }
    fn load_local(&mut self) {
        self.initial_pick = false;
        self.pending_refresh = false;
        self.new_pick();
        let root = self.picker.root.clone();
        self.pending = Some(submit(&self.pick_token, move || {
            repo::local_changes(&root, false).map(|set| (set, local_spec()))
        }));
    }
    fn pick(&mut self) {
        match self.picker.comparison_spec() {
            // already on screen: drop any other load in flight and keep what is shown, without reloading it
            Ok(spec) if self.current_spec.as_ref() == Some(&spec) && self.ready.is_none() => {
                self.new_pick();
                self.pending = None;
                self.initial_pick = false;
            }
            Ok(spec) => self.load(spec),
            Err(error) => self.notice = Some(error),
        }
    }
    fn restart(&mut self, branches: Branches, want: Option<String>) {
        if self.opened.is_some(){self.new_pick();}
        self.filter_token = std::sync::Arc::new(());
        self.filter = None;
        self.followed = None;
        self.picker.restart(branches, want);
        self.picker.start_stream();
    }
    fn load_branches(&mut self,limit:usize){
        self.branch_token=std::sync::Arc::new(());
        self.branch_limit=limit;
        self.picker.sources.branches_loading=true;
        self.picker.sources.show_all_branches=true;
        let root=self.picker.root.clone();
        self.branches=Some(submit_inventory(&self.branch_token,move||(limit,repo::branches_limited(&root,limit.saturating_add(1)))));
    }
    fn load_more_prs(&mut self){
        if self.prs.is_some() || !self.picker.sources.prs_more{return;}
        let root=self.picker.root.clone();let limit=self.picker.sources.prs.len()+SOURCE_BATCH+1;
        self.picker.sources.prs_loading=true;
        self.prs=Some(submit_github(&self.token,move||crate::pr::Gh::new(&root).list_open(limit)));
    }
    fn load_remotes(&mut self,limit:usize){
        self.remote_token=std::sync::Arc::new(());self.remote_limit=limit;
        self.picker.sources.remotes_loading=true;
        let root=self.picker.root.clone();
        self.remotes=Some(submit_inventory(&self.remote_token,move||repo::remotes_limited(&root,limit)));
    }
    pub fn poll(&mut self) -> bool {
        self.poll_rows(12.0)
    }
    /// [`poll`](Self::poll) with the log's visible rows, which set how far ahead history is read.
    pub fn poll_rows(&mut self, log_rows: f32) -> bool {
        let mut changed = false;
        if let Some((plan,inventory,filters))=receive(&mut self.refreshing)
            && self.picker.refresh_is_current(&plan){
            self.source_details=None;
            self.head=self.picker.apply_repository_inventory(inventory);
            let errors=self.picker.apply_refresh_filters(&plan,filters);
            self.notice=Some(if errors.is_empty(){"Repository refreshed".into()}else{errors.join("\n")});changed=true;
        }
        if let Some(inventory) = receive(&mut self.sources) {
            changed = true;
            // A refresh may have supplied the complete inventory before the first batch arrived.
            if !self.picker.sources.loaded {
                let RepositoryInventory{sources,refs,head,root}=inventory;
                self.picker.root=root;
                self.picker.sources.refresh_with(sources);
                self.picker.refs = refs;
                self.picker.title_branch = self
                    .picker
                    .sources
                    .branches
                    .iter()
                    .find(|branch| branch.current)
                    .map_or("HEAD", |branch| branch.name.as_str())
                    .into();
                self.head = head;
                let details_root=self.picker.root.clone();
                self.source_details=Some(submit_inventory(&self.token,move||read_inventory_details(details_root)));
            }
            if !std::mem::take(&mut self.stream_early) {
                self.picker.start_stream();
            }
            let root = self.picker.root.clone();
            if github_url(&root).is_some() {
                self.picker.sources.prs_more=true;
                self.load_more_prs();
            }
            if !self.picker.sources.filter.is_empty(){self.load_branches(usize::MAX);}
            if let Some(path) = self.initial_follow.take() {
                self.execute(RepositoryCommand::FileHistory(path));
            }
        }
        if let Some(mut inventory)=receive(&mut self.source_details){
            // A branch batch/filter may have completed while worktree status was being read.
            inventory.sources.branches=std::mem::take(&mut self.picker.sources.branches);
            inventory.sources.branches_more=self.picker.sources.branches_more;
            inventory.sources.branches_loading=self.picker.sources.branches_loading;
            self.head=self.picker.apply_repository_inventory(inventory);
            if !self.picker.sources.filter.is_empty()&&self.picker.sources.remotes_more{self.load_remotes(usize::MAX);}
            changed=true;
        }
        if let Some(result)=receive(&mut self.remotes){
            self.picker.sources.remotes_loading=false;
            match result{Ok((remotes,more))=>{self.picker.sources.remotes=remotes;self.picker.sources.remotes_more=more;},Err(e)=>self.notice=Some(e)}
            changed=true;
        }
        if let Some((limit,result))=receive(&mut self.branches){
            self.picker.sources.branches_loading=false;
            match result{Ok(branches)=>self.picker.sources.set_branches(branches,limit),Err(e)=>self.notice=Some(e)}
            changed=true;
        }
        if let Some(result) = receive(&mut self.prs) {
            changed = true;
            self.picker.sources.prs_loading=false;
            match result {
                Ok(mut prs)=>{
                    let limit=self.picker.sources.prs.len()+SOURCE_BATCH;
                    self.picker.sources.prs_more=prs.len()>limit;
                    prs.truncate(limit);
                    self.picker.sources.prs=prs;
                }
                Err(e)=>self.notice=Some(e.to_string()),
            }
        }
        let (stream_changed, pick) = self.picker.poll_stream(log_rows);
        changed |= stream_changed;
        if pick {
            self.pick();
        }
        if let Some(result) = receive(&mut self.pending) {
            changed = true;
            match result {
                Ok((set, spec)) => {
                    if std::mem::take(&mut self.initial_pick) {
                        self.place_initial(&spec);
                    }
                    self.refresh_placement=std::mem::take(&mut self.pending_refresh);
                    self.accept(set, spec);
                }
                Err(error) => {
                    crate::diagnostics::repository_failure(&error);
                    self.notice = Some(error);
                    self.failed = self.set.is_none();
                }
            }
        }
        if let Some(result) = receive(&mut self.opened) {
            changed = true;
            match result {
                Ok(opened) => {
                    self.accept(opened.set, opened.fetched.spec());
                    self.adopt_pr(opened.pr, opened.fetched);
                }
                Err(crate::pr::PrError::NoPullRequest) => {
                    self.notice = Some(
                        "No pull request for this branch — pick one under Pull requests".into(),
                    );
                    let root = self.picker.root.clone();
                    self.initial_pick=true;
                    self.pending = Some(submit(&self.pick_token, move || {
                        pinned_commit_comparison(&root,"HEAD").map(|(set,spec,_)|(set,spec))
                    }));
                    self.picker.anchor = 1;
                }
                Err(error) => {
                    // a mistake the user fixes shows as the notice only; a failure also saves a report
                    if !error.is_user_error() { crate::diagnostics::pull_request_failure(&error); }
                    self.notice = Some(error.to_string());
                    self.failed = self.set.is_none();
                }
            }
        }
        if let Some(pr)=self.pr.as_mut(){changed|=pr.poll_review(self.set.as_ref().map_or(&[],|set|&set.comparisons));}
        if let Some((request,result)) = receive(&mut self.filter) {
            changed=true;
            match self.picker.accept_deep_filter(&request,result){
                HistoryAcceptance::Error(error)=>self.notice=Some(error),
                HistoryAcceptance::Stale|HistoryAcceptance::Filtered|HistoryAcceptance::Followed{..}=>{},
            }
        }
        if let Some((request,result)) = receive(&mut self.followed) {
            changed=true;
            match self.picker.accept_file_history(&request,result){
                HistoryAcceptance::Followed{pick:true}=>self.pick(),
                HistoryAcceptance::Error(error)=>self.notice=Some(error),
                HistoryAcceptance::Stale|HistoryAcceptance::Filtered|HistoryAcceptance::Followed{pick:false}=>{},
            }
        }
        if let Some(result) = receive(&mut self.graph) {
            changed = true;
            self.notice = Some(match result {
                Ok(()) => "Commit graph written: history opens fast from now on".into(),
                Err(error) => {
                    crate::diagnostics::repository_failure(&error);
                    error
                }
            });
        }
        if let Some(result) = receive(&mut self.message_reading) {
            changed = true;
            match result {
                Ok(mut fresh) => {
                    // a refresh keeps the commits that were open
                    if let Some(old) = self.message.as_ref().filter(|m| m.query == fresh.query) {
                        for (open, c) in fresh.open.iter_mut().zip(&fresh.commits) {
                            *open = old.commits.iter().zip(&old.open).any(|(o, was)| *was && o.sha == c.sha);
                        }
                    }
                    self.message_cursor = self.message_cursor.min(fresh.commits.len().saturating_sub(1));
                    self.message = Some(fresh);
                }
                Err(error) => { self.message = None; self.message_error = Some(error); }
            }
        }
        if let Some(result) = receive(&mut self.moved) {
            changed = true;
            if let Ok(true) = result {
                self.notice = Some("PR updated — refresh to fetch the new head".into());
            }
        }
        changed
    }
    fn show_picker(&mut self, shown: bool) {
        self.picker.shown = shown;
        self.picker.focus = if shown { Pane::Log } else { Pane::Diff };
        if shown && !self.picker.sources.loaded && self.sources.is_none() {
            self.picker.sources.branches_loading=true;
            self.picker.sources.details_loading=true;
            // the log streams when the inventory arrives
            let root = self.picker.root.clone();
            self.sources = Some(submit_inventory(&self.token, move || read_startup_inventory(root)));
        } else if shown && self.sources.is_none() && self.picker.stream.is_none() && self.picker.log.is_empty() && !self.picker.loading {
            self.picker.start_stream();
        }
    }
    /// The file and line on screen, for the place Back returns to.
    pub fn set_position(&mut self, file: Option<String>, line: Option<u32>) { self.position = (file, line); }
    /// What the view compares, named for people; `None` before the first Change Set or for plain files.
    pub fn selection_title(&self) -> Option<crate::selection_title::SelectionTitle> {
        let pr = self.pr.as_ref().map(|s| (s.pr.number, s.pr.title.as_str()));
        crate::selection_title::title(self.current_spec.as_ref()?, self.picker.refs.as_ref(), &self.picker.sources.stashes, pr)
    }
    /// One of a PR's commits on screen goes back to the whole PR's Commits tab first.
    fn pr_commit_shown(&self) -> bool { self.pr.as_ref().is_some_and(|p| p.shown_commit.is_some()) }
    pub fn can_go_back(&self) -> bool { self.pr_commit_shown() || self.nav.can_back() }
    pub fn can_go_forward(&self) -> bool { self.nav.can_forward() }
    pub fn go_back(&mut self) {
        if self.pr_commit_shown() {
            self.execute(RepositoryCommand::PrCommit(None));
            if let Some(pr) = self.pr.as_mut() { pr.execute(crate::pr_session::PrCommand::Show(crate::pr_session::DetailsTab::Commits)); }
            return;
        }
        let here = self.here(); if let Some(spot) = self.nav.back(here) { self.go_to(spot); }
    }
    pub fn go_forward(&mut self) { let here = self.here(); if let Some(spot) = self.nav.forward(here) { self.go_to(spot); } }
    /// The file and line a Back or Forward asked to reopen, for the adapter to apply once the comparison settles.
    pub fn take_nav_restore(&mut self) -> Option<crate::place::Restore> { self.nav_restore.take() }

    /// Where the view is now.
    fn here(&self) -> crate::navigation::Spot {
        use crate::navigation::Picked;
        let p = &self.picker;
        let id = |row: u32| p.log_row(row).map(|i| p.log.id(i));
        let picked = if p.anchor == 0 { Some(Picked::Local) }
            else { id(p.anchor).map(|anchor| Picked::Commits { anchor, second: p.second.and_then(id), range: p.range }) };
        crate::navigation::Spot { branches: p.branches.clone(), filter: p.filter.clone(), file_history: p.file_paths.as_ref().map(|(f, _)| f.clone()),
            picked, file: self.position.0.clone(), line: self.position.1 }
    }

    /// Show `spot` again: the log's branches, filter and file history, its selection, then (through the adapter) its file
    /// and line.
    fn go_to(&mut self, spot: crate::navigation::Spot) {
        use crate::navigation::Picked;
        self.navigating = true;
        let now = self.here();
        // a log that restarts below is not yet the one the spot's commit is picked from
        let reloading = (&spot.filter, &spot.file_history, &spot.branches) != (&now.filter, &now.file_history, &now.branches);
        if (&spot.filter, &spot.file_history) != (&now.filter, &now.file_history) {
            self.picker.clear_filter_and_history();
            if !spot.filter.is_empty() { self.picker.set_filter(spot.filter.clone()); }
            if let Some(path) = &spot.file_history { self.execute(RepositoryCommand::FileHistory(path.clone())); }
        }
        if spot.branches != now.branches { self.picker.compare = None; self.restart(spot.branches.clone(), None); }
        let commit = match &spot.picked {
            Some(Picked::Local) => { self.picker.anchor = 0; self.picker.second = None; self.pick(); None }
            Some(Picked::Commits { anchor, .. }) => {
                // already in this log (HEAD included, which want_commit would otherwise wait for): select it now
                match self.picker.log.row_of(anchor).and_then(|row| self.picker.display_row(row)).filter(|_| !reloading) {
                    Some(row) => { self.picker.anchor = row; self.picker.second = None; self.execute(RepositoryCommand::Select); }
                    None => self.want_commit(anchor.clone()),
                }
                Some(anchor.clone())
            }
            None => None,
        };
        self.nav_restore = Some(crate::place::Restore { commit, file: spot.file, line: spot.line });
        self.navigating = false;
    }

    /// Run `command`; a step that changes where the view is (another commit, branch, filter or file history) is
    /// recorded for Back.
    pub fn execute(&mut self, command: RepositoryCommand) {
        use RepositoryCommand as C;
        let step = matches!(&command, C::SelectRow { .. } | C::Move { .. } | C::Select | C::Walk(_) | C::ApplyFilter | C::ClearFilter | C::FileHistory(_)
                | C::Compare { .. } | C::CompareRows(..) | C::Parent(_) | C::OpenPr(_))
            || matches!(&command, C::Source(a) if !matches!(a, SourceAction::Toggle(_) | SourceAction::ShowAllBranches | SourceAction::MorePrs | SourceAction::MoreRemotes | SourceAction::CopyCommand(_)));
        let is_move = matches!(command, C::Move { .. });
        // a step whose effect arrives later (file history, a deep filter, a pull request) counts at once
        let later = matches!(command, C::FileHistory(_) | C::ApplyFilter | C::OpenPr(_) | C::Compare { .. });
        let before = (step && !self.navigating && !(is_move && self.moving)).then(|| self.here());
        self.moving = is_move;
        self.run(command);
        if let Some(before) = before && (later || !before.same_place(&self.here())) { self.nav.record(before); }
    }

    fn run(&mut self, command: RepositoryCommand) {
    crate::diagnostics::breadcrumb(command_label(&command));
        if let Some(reason) = self.disabled(&command) {
            self.notice = Some(reason.into());
            return;
        }
        use RepositoryCommand as C;
        if matches!(&command,C::Move{..}|C::Select|C::SelectRow{..}|C::ContextRow(_)|C::CompareRows(_,_)|C::Walk(_)|C::Compare{..}|C::OpenPr(_)|C::ClearFilter|C::Filter(_)){self.picker.cancel_refresh();}
        match command {
            C::Focus(pane) => self.picker.focus = pane,
            C::TogglePicker => self.show_picker(!self.picker.shown),
            C::ToggleSidebar => {
                self.picker.sidebar = !self.picker.sidebar || !self.picker.shown;
                self.show_picker(true);
                self.picker.focus = if self.picker.sidebar { Pane::Sidebar } else { Pane::Log };
            }
            C::ToggleLog => { self.picker.log_shown = !self.picker.log_shown || !self.picker.shown; self.show_picker(true); }
            C::SetPanels { branches, history } => {
                let ((had_branches, had_history), focus) = (self.panels(), self.picker.focus);
                if !branches && !history { self.show_picker(false); }
                else {
                    self.picker.sidebar = branches;
                    self.picker.log_shown = history;
                    if !self.picker.shown { self.show_picker(true); }
                    // a pane that appears takes the keyboard; otherwise it stays, unless its pane went away
                    self.picker.focus = if history && !had_history { Pane::Log }
                        else if branches && !had_branches { Pane::Sidebar }
                        else if focus == Pane::Log && !history || focus == Pane::Sidebar && !branches { Pane::Diff }
                        else { focus };
                }
            }
            C::Source(action) => match self.picker.source_action(action) {
                SourcePlan::None => {}
                SourcePlan::Conflicts => self.effects.push(RepositoryEffect::Conflicts),
                SourcePlan::Copy(text) => self.effects.push(RepositoryEffect::Copy(text)),
                SourcePlan::Local => self.load_local(),
                // another log: whatever was being compared is over
                SourcePlan::Restart(branches, want) => { self.picker.compare = None; self.restart(branches, want) }
                SourcePlan::Select => self.pick(),
                SourcePlan::Worktree(path) => {
                    self.new_pick();
                    self.pending = Some(submit(&self.pick_token, move || {
                        repo::local_changes(&path, true).map(|set| (set, local_spec()))
                    }));
                }
                SourcePlan::Stash(sha) => {
                    self.new_pick();
                    let root = self.picker.root.clone();
                    self.pending = Some(submit(&self.pick_token, move || {
                        repo::stash_changes(&root, &sha).map(|set| (set, stash_spec(&sha)))
                    }));
                }
                SourcePlan::Pr(number) => self.execute(C::OpenPr(Some(number.to_string()))),
                SourcePlan::MoreBranches=>if !self.picker.sources.branches_loading{self.load_branches(self.branch_limit.saturating_add(SOURCE_BATCH));},
                SourcePlan::MorePrs=>self.load_more_prs(),
                SourcePlan::MoreRemotes=>if !self.picker.sources.remotes_loading{self.load_remotes(self.remote_limit.saturating_add(SOURCE_BATCH));},
            },
            C::Move { delta, extend, two } => {
                self.picker.move_selection(delta, extend, two);
                self.pick();
            }
            C::Select => self.pick(),
            C::Parent(parent) => {
                self.picker.merge_parent = parent % self.picker.merge_parents();
                self.pick();
            }
            C::Filter(input) => {
                self.filter_token = std::sync::Arc::new(());
                self.filter = None;
                self.picker.set_filter(input);
            }
            C::ApplyFilter => {
                let request=self.picker.begin_deep_filter();
                self.filter=Some(submit(&self.filter_token,move || {
                    let result=request.read_filter();(request,result)
                }));
            }
            C::ClearFilter => {
                self.filter_token=std::sync::Arc::new(());self.filter=None;self.followed=None;
                if !self.picker.clear_filter_and_history() && self.picker.compare.take().is_some(){self.restart(Branches::All,self.head.clone());}
            }
            C::SourceFilter(input) => {
                self.picker.sources.filter = input;
                self.picker.sources.selected = 0;
                self.picker.sources.scroll = 0.0;
                if !self.picker.sources.filter.is_empty()&&self.picker.sources.branches_more{self.load_branches(usize::MAX);}
                if !self.picker.sources.filter.is_empty()&&self.picker.sources.remotes_more{self.load_remotes(usize::MAX);}
            }
            C::SourceMove(delta) => {
                let last = self
                    .picker
                    .sources
                    .rows_with_palette(&self.picker.branches, SourcePalette::default())
                    .len()
                    .saturating_sub(1);
                self.picker.sources.selected = (self.picker.sources.selected as i64
                    + i64::from(delta))
                .clamp(0, last as i64) as usize;
            }
            C::SourceSelect(index) => {
                let rows = self.picker.sources.rows_with_palette(&self.picker.branches, SourcePalette::default());
                if let Some(row) = rows.get(index) {
                    let action = row.action.clone();
                    self.picker.sources.selected = index;
                    self.picker.focus = Pane::Sidebar;
                    if let Some(action) = action { self.execute(C::Source(action)); }
                }
            }
            C::SelectRow { row, extend, two } => {
                self.picker.focus = Pane::Log;
                self.picker.select_row(row, extend, two);
                self.pick();
            }
            C::Scroll { pane, rows, visible } => {
                let max = match pane {
                    Pane::Sidebar => self.picker.sources.rows_with_palette(&self.picker.branches, SourcePalette::default()).len() as f32,
                    Pane::Log => self.picker.rows() as f32,
                    Pane::Diff => return,
                };
                let scroll = match pane { Pane::Sidebar => &mut self.picker.sources.scroll, Pane::Log => &mut self.picker.scroll, Pane::Diff => return };
                *scroll = (*scroll + rows).clamp(0.0, (max-visible.max(1.0)).max(0.0));
            }
            C::Walk(branches) => {
                self.picker.compare = None;
                self.restart(branches, None);
            }
            C::ContextRow(row)=>{if row<self.picker.rows(){self.picker.focus=Pane::Log;self.picker.select_row(row,false,false);}},
            C::CompareRows(a,b)=>{if a<self.picker.rows()&&b<self.picker.rows(){self.picker.select_row(a,false,false);self.picker.select_row(b,false,true);self.pick();}},
            C::Compare {
                left,
                right,
                merge_base,
            } => {
                self.picker.compare = Some((right.clone(), merge_base));
                self.restart(Branches::One(format!("{left}...{right}")), None);
                // the diff reads from the other branch to `left` (the current one): what this branch adds, like a PR
                self.load(branch_comparison(&right, &left, merge_base));
            }
            C::FileHistory(path) => {
                self.initial_pick=false;
                // from a comparison with the picker hidden: the sidebar and the log load as when it is shown
                self.show_picker(true);
                if self.sources.is_some() {
                    // the inventory resolves the root (a link or a subdirectory becomes git's top level);
                    // a request against the unresolved one would be dropped as stale when it arrives
                    self.initial_follow=Some(path);
                } else {
                    let request=self.picker.begin_file_history(path);
                    self.followed=Some(submit(&self.filter_token,move || {
                        let result=request.read_followed();(request,result)
                    }));
                }
            }
            C::WriteGraph => {
                self.picker.graph_notice = false;
                self.notice = Some("Writing the commit-graph file…".into());
                let root = self.picker.root.clone();
                self.graph = Some(submit(&self.token, move || {
                    history::write_commit_graph(&root)
                }));
            }
            C::Index => self.load(index_spec()),
            C::OpenCommit => {
                let root = self.picker.root.clone();
                if let Some(id) = self.picker.log_row(self.picker.anchor).map(|row| self.picker.log.id(row)) {
                    match crate::blame::parent_or_empty_tree(&root, &id) {
                        Ok(base) => self.effects.push(RepositoryEffect::OpenRevisions { root, revs: vec![base, id] }),
                        Err(error) => self.notice = Some(error),
                    }
                }
            }
            C::CompareRevs(left, right) => self.load(Spec::Git { cached: false, revs: vec![left, right], paths: Vec::new(), untracked: false }),
            C::ShowInGraph => {
                let id = self
                    .picker
                    .log_row(self.picker.anchor)
                    .map(|row| self.picker.log.id(row));
                self.execute(C::ClearFilter);
                if let Some(row) = id.as_deref().and_then(|id| self.picker.log.row_of(id)) {
                    self.picker.anchor = row + 1;
                    self.picker.second = None;
                    self.picker.scroll = (row as f32 - 4.0).max(0.0);
                }
            }
            C::CopyHash | C::CopySubject | C::CopyDetails => {
                if let Some(text)=self.picker.clipboard_text(&command,self.picker.anchor){self.effects.push(RepositoryEffect::Copy(text));}
            }

            C::CopyUrl | C::OpenUrl => {
                let url = self.pr.as_ref().map(|pr| pr.pr.url.clone()).or_else(|| {
                    Some(format!(
                        "{}/commit/{}",
                        github_url(&self.picker.root)?,
                        self.picker.log.id(self.picker.log_row(self.picker.anchor)?)
                    ))
                });
                if let Some(url) = url {
                    self.effects.push(if matches!(command, C::CopyUrl) {
                        RepositoryEffect::Copy(url)
                    } else {
                        RepositoryEffect::OpenUrl(url)
                    });
                } else {
                    self.notice = Some("No GitHub URL for the selected commit".into());
                }
            }
            C::OpenPr(target) => {
                self.cleanup_pr = true;
                self.new_pick();
                self.pending = None;
                self.overlay=None;
                if let Some(pr)=self.pr.as_mut(){pr.execute(crate::pr_session::PrCommand::Close);}
                let root = self.picker.root.clone();
                self.opening_label=match target.as_deref(){Some(t) if t.chars().all(|c|c.is_ascii_digit())=>format!("Loading pull request #{t}…"),Some(t)=>format!("Loading pull request {t}…"),None=>"Loading pull request…".into()};
                self.opened = Some(submit_pr(&self.pick_token, move || {
                    crate::pr_session::open(&root, target.as_deref())
                }));
                self.notice = Some("Fetching pull request…".into());
            }
            C::RefreshRepository => {
                self.branch_token=std::sync::Arc::new(());self.branches=None;
                self.remote_token=std::sync::Arc::new(());self.remotes=None;self.picker.sources.remotes_loading=false;
                self.picker.sources.branches_loading=false;
                let plan=self.picker.begin_repository_refresh();
                if plan.history{self.picker.start_stream();}
                self.filter_token=std::sync::Arc::new(());self.filter=None;self.followed=None;
                let root=self.picker.root.clone();
                let limit=self.branch_limit;
                let remote_limit=self.remote_limit;
                self.refreshing=Some(submit_inventory(&self.token,move||{
                    let inventory=read_inventory_limited(root,limit,remote_limit);let filters=plan.read_filters();(plan,inventory,filters)
                }));
                if let Some(spec)=self.current_spec.clone(){
                    self.initial_pick=false;self.new_pick();self.opened=None;
                    let root=self.set.as_ref().and_then(|set|set.git_root()).unwrap_or(&self.picker.root).to_path_buf();
                    let selected=self.picker.log_row(self.picker.anchor).map(|row|self.picker.log.id(row));
                    let stash=self.set.as_ref().is_some_and(|set|set.groups.iter().any(|(group,_)|group=="Changes"));
                    self.pending=Some(submit(&self.pick_token,move||read_refresh_comparison(&root,spec,selected.as_deref(),stash)));
                    self.pending_refresh=true;
                }
            }
            C::RefreshPr => {
                if let Some(pr) = self.pr.as_ref() {
                    self.execute(C::OpenPr(Some(pr.pr.number.to_string())));
                }
            }
            C::CloseOverlay => {self.overlay=None;if let Some(pr)=self.pr.as_mut(){pr.execute(crate::pr_session::PrCommand::Close);}},
            C::ToggleBody=>{if let Some(pr)=self.pr.as_mut(){pr.execute(if pr.details().is_some(){crate::pr_session::PrCommand::Close}else{crate::pr_session::PrCommand::Show(crate::pr_session::DetailsTab::Conversation)});}},
            C::PrDetails(tab)=>{self.overlay=None;if let Some(pr)=self.pr.as_mut(){pr.execute(crate::pr_session::PrCommand::Show(tab));}},
            C::Threads{path,outdated}=>{self.overlay=None;if let Some(pr)=self.pr.as_mut(){let threads=pr.file_threads(&path,outdated);pr.execute(crate::pr_session::PrCommand::ShowThreads(threads));}},
            C::Viewed { index, jump } => {
                if let Some(pr) = self.pr.as_mut() {
                    match pr.toggle_viewed(
                        self.set.as_ref().map_or(&[], |set| &set.comparisons),
                        index,
                        jump,
                    ) {
                        Ok(Some(index)) => self.effects.push(RepositoryEffect::OpenFile(index)),
                        Ok(None) => self.notice = Some("Viewed Mark updated locally".into()),
                        Err(error) => self.notice = Some(error),
                    }
                }
            }
            C::ToggleMessage(None) => self.message_collapsed = !self.message_collapsed,
            C::ToggleMessage(Some(i)) => if let Some(open) = self.message.as_mut().and_then(|m| m.open.get_mut(i)) { *open = !*open; self.message_cursor = i; },
            C::MessageCursor(i) => self.message_cursor = i,
            C::PrCommit(sha) => if let Some(pr) = self.pr.as_mut() {
                // Files changed, for that commit alone or the whole PR again
                let spec = match &sha { Some(sha) => branch_comparison(&format!("{sha}^1"), sha, false), None => pr.fetched.spec() };
                pr.shown_commit = sha;
                pr.execute(crate::pr_session::PrCommand::Close);
                self.load(spec);
            },
            C::CheckPrHead => {
                if let Some(pr) = self.pr.as_ref() {
                    let (root, url, head) = (
                        self.picker.root.clone(),
                        pr.pr.url.clone(),
                        pr.fetched.head.clone(),
                    );
                    self.moved = Some(submit_github(&self.token, move || {
                        crate::pr::Gh::new(&root).head_moved(&url, &head)
                    }));
                }
            }
        }
    }
}

pub fn github_url(root: &Path) -> Option<String> {
    let bytes = crate::git::run(root, &["remote", "get-url", "origin"]).ok()?;
    let url = String::from_utf8_lossy(&bytes)
        .trim()
        .trim_end_matches(".git")
        .to_string();
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.split_once("github.com/").map(|(_, rest)| rest))?;
    Some(format!("https://github.com/{rest}"))
}

impl Drop for RepositorySession {
    fn drop(&mut self) {
        if self.cleanup_pr {
            let root = self.picker.root.clone();
            // Queue cleanup after any fetch already running, without waiting on network at exit.
            let token = std::sync::Arc::new(());
            let keep = token.clone();
            let _ = submit_pr(&token, move || {
                let _keep = keep;
                let _ = crate::pr::remove_refs(&root);
            });
        }
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::git::testing::{commit_all, git, repo, write};
    fn settle(state: &mut RepositorySession) {
        let started = std::time::Instant::now();
        while state.busy() {
            state.poll();
            assert!(
                started.elapsed().as_secs() < 10,
                "repository worker did not complete"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn open(root: &Path) -> RepositorySession {
        let request = crate::cli::parse(&["log".into()], root).unwrap();
        let mut state = RepositorySession::open(&request, root.into());
        settle(&mut state);
        state
    }
    #[test]
    fn repository_branches_load_in_batches_and_filter_finds_unloaded_branches() {
        let root=repo();write(root.path(),"file","one\n");commit_all(root.path(),"One");
        for n in 0..65 { git(root.path(),&["branch",&format!("topic-{n:03}")]); }
        let mut state=open(root.path());
        assert_eq!(state.picker.sources.branches.len(),20,"startup reads only the first branch batch");
        assert!(state.picker.sources.branches[0].current);
        for count in [40,60,66] {
            let rows=state.picker.sources.rows_with_palette(&state.picker.branches,SourcePalette::default());
            let more=rows.iter().position(|r| r.text.contains("Load more branches")).expect("more branches row");
            state.execute(RepositoryCommand::SourceSelect(more));settle(&mut state);
            assert_eq!(state.picker.sources.branches.len(),count);
            assert!(!state.can_go_back(),"loading a batch is not navigation");
        }
        state.execute(RepositoryCommand::RefreshRepository);settle(&mut state);
        assert_eq!(state.picker.sources.branches.len(),66,"refresh keeps the requested coverage");
        let mut filtered=open(root.path());
        filtered.execute(RepositoryCommand::SourceFilter("topic-064".into()));settle(&mut filtered);
        assert!(filtered.picker.sources.rows_with_palette(&filtered.picker.branches,SourcePalette::default()).iter().any(|r| r.text.contains("topic-064")));
    }

    #[test]
    fn repository_remote_branches_load_in_batches() {
        let root=repo();write(root.path(),"file","one\n");commit_all(root.path(),"One");
        for n in 0..45 {git(root.path(),&["update-ref",&format!("refs/remotes/origin/topic-{n:03}"),"HEAD"]);}
        let count=|state:&RepositorySession|state.picker.sources.remotes.iter().map(|r|r.branches.len()).sum::<usize>();
        let mut state=open(root.path());assert_eq!(count(&state),20);
        state.execute(RepositoryCommand::Source(SourceAction::Toggle("Remotes")));
        for n in [40,45] {
            let rows=state.picker.sources.rows_with_palette(&state.picker.branches,SourcePalette::default());
            let more=rows.iter().position(|r|r.text.contains("Load more remote branches")).expect("more remotes row");
            state.execute(RepositoryCommand::SourceSelect(more));settle(&mut state);assert_eq!(count(&state),n);
        }
        state.execute(RepositoryCommand::RefreshRepository);settle(&mut state);assert_eq!(count(&state),45);
        let mut filtered=open(root.path());filtered.execute(RepositoryCommand::Source(SourceAction::Toggle("Remotes")));
        filtered.execute(RepositoryCommand::SourceFilter("topic-044".into()));settle(&mut filtered);
        assert!(filtered.picker.sources.rows_with_palette(&filtered.picker.branches,SourcePalette::default()).iter().any(|r|r.text.contains("topic-044")));
    }

    #[test]
    fn repository_prs_load_in_batches_without_changing_the_comparison() {
        let root=repo();write(root.path(),"file","one\n");commit_all(root.path(),"One");
        write(root.path(),"file","two\n");commit_all(root.path(),"Two");
        let _provider=crate::pr::test_support::markdown_provider(root.path());
        let dir=root.path().join(".git/markdown-provider");
        let script=std::fs::read_to_string(dir.join("gh.sh")).unwrap();
        let script=script.lines().map(|line| if line.starts_with("  'pr list'*") {
            "  'pr list'*) case \"$*\" in *'--limit 21 '*) cat \"$dir/list-21.json\";; *'--limit 41 '*) cat \"$dir/list-41.json\";; *) cat \"$dir/list-45.json\";; esac;;"
        }else{line}).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.join("gh.sh"),script).unwrap();
        for count in [21,41,45] {
            let prs:Vec<_>=(1..=count).map(|number|serde_json::json!({"number":number,"title":format!("PR {number}"),"author":{"login":"tester"},"headRefName":"topic","baseRefName":"main","isDraft":false,"updatedAt":""})).collect();
            std::fs::write(dir.join(format!("list-{count}.json")),serde_json::to_vec(&prs).unwrap()).unwrap();
        }
        let mut state=open(root.path());let before=state.current_spec.clone();
        assert_eq!(state.picker.sources.prs.len(),20);
        for count in [40,45] {
            let rows=state.picker.sources.rows_with_palette(&state.picker.branches,SourcePalette::default());
            let more=rows.iter().position(|r|r.text.contains("Load more pull requests")).expect("more PRs row");
            state.execute(RepositoryCommand::SourceSelect(more));settle(&mut state);
            assert_eq!(state.picker.sources.prs.len(),count);
            assert_eq!(state.current_spec,before);
        }
        let rows=state.picker.sources.rows_with_palette(&state.picker.branches,SourcePalette::default());
        assert!(!rows.iter().any(|r|r.text.contains("Load more pull requests")));
        assert!(!state.can_go_back());
    }

    #[test]
    fn a_slow_pr_list_does_not_block_local_navigation_or_opening_a_pr() {
        let root=repo();write(root.path(),"file","one\n");commit_all(root.path(),"One");
        write(root.path(),"file","two\n");commit_all(root.path(),"Two");
        let _provider=crate::pr::test_support::markdown_provider(root.path());
        let dir=root.path().join(".git/markdown-provider");
        struct Release(PathBuf);
        impl Drop for Release {fn drop(&mut self){let _=std::fs::write(&self.0,"");}}
        let release=Release(dir.join("list-release"));
        let script=std::fs::read_to_string(dir.join("gh.sh")).unwrap();
        let script=script.lines().map(|line|if line.starts_with("  'pr list'*"){
            "  'pr list'*) touch \"$dir/list-started\"; while [ ! -e \"$dir/list-release\" ]; do sleep 0.01; done; echo '[]';;"
        }else{line}).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.join("gh.sh"),script).unwrap();
        let request=crate::cli::parse(&["log".into()],root.path()).unwrap();
        let mut state=RepositorySession::open(&request,root.path().into());
        let start=std::time::Instant::now();
        while !dir.join("list-started").exists(){state.poll();assert!(start.elapsed().as_secs()<10);std::thread::sleep(std::time::Duration::from_millis(2));}
        state.take_change_set();
        state.execute(RepositoryCommand::Source(SourceAction::Local));
        let start=std::time::Instant::now();
        while state.take_change_set().is_none(){state.poll();assert!(start.elapsed().as_secs()<10,"local comparison waited for GitHub");std::thread::sleep(std::time::Duration::from_millis(2));}
        state.execute(RepositoryCommand::OpenPr(Some("9".into())));
        assert!(state.pr_loading().unwrap().contains("#9"));
        let start=std::time::Instant::now();
        while state.pr.is_none(){state.poll();assert!(start.elapsed().as_secs()<10,"PR open waited for background listing");std::thread::sleep(std::time::Duration::from_millis(2));}
        assert!(state.pr_loading().is_none());
        assert!(!dir.join("list-release").exists());
        drop(release);settle(&mut state);
        assert_eq!(std::fs::read_to_string(root.path().join("file")).unwrap(),"two\n");
    }

    #[test]
    fn early_refresh_keeps_startup_inventory_and_pr_bootstrap_even_when_cancelled() {
        for cancel in [false,true] {
            let root=repo();write(root.path(),"file","one\n");commit_all(root.path(),"One");
            write(root.path(),"file","two\n");commit_all(root.path(),"Two");
            let _provider=crate::pr::test_support::markdown_provider(root.path());
            let initial=read_startup_inventory(root.path().into());
            let request=crate::cli::parse(&["log".into()],root.path()).unwrap();
            let mut state=RepositorySession::open(&request,root.path().into());
            let (tx,rx)=std::sync::mpsc::sync_channel(1);state.sources=Some(rx);
            git(root.path(),&["tag","after-startup"]);
            state.execute(RepositoryCommand::RefreshRepository);
            if cancel {state.execute(RepositoryCommand::Source(SourceAction::Local));}
            else {
                let start=std::time::Instant::now();
                while state.refreshing.is_some(){state.poll();assert!(start.elapsed().as_secs()<10);std::thread::sleep(std::time::Duration::from_millis(2));}
            }
            let _=tx.send(Some(initial));settle(&mut state);
            assert_eq!(state.picker.sources.prs.len(),1,"initial listing must survive early refresh, cancelled={cancel}");
            assert!(state.picker.sources.loaded&&!state.picker.sources.details_loading);
            assert!(state.picker.sources.tags.iter().any(|t|t.name=="after-startup"),"late first batch must not overwrite refreshed details");
        }
    }

    #[test]
    #[ignore]
    fn repository_startup_many_branches_perf() {
        let root=repo();write(root.path(),"file","one\n");commit_all(root.path(),"One");
        let head=git(root.path(),&["rev-parse","HEAD"]).trim().to_string();
        let refs=(0..5000).map(|n|format!("create refs/heads/topic-{n:05} {head}\n")).collect::<String>();
        crate::git::run_with_input(root.path(),&["update-ref","--stdin"],refs.into_bytes()).unwrap();
        let trees=tempfile::tempdir().unwrap();
        for n in 0..24 { git(root.path(),&["worktree","add","-q","--detach",trees.path().join(format!("tree-{n}")).to_str().unwrap()]); }
        let request=crate::cli::parse(&["log".into()],root.path()).unwrap();
        let mut samples=Vec::new();
        for _ in 0..6 {
            let start=std::time::Instant::now();
            let mut state=RepositorySession::open(&request,root.path().into());
            let (mut branches,mut comparison)=(None,None);
            while state.busy() {
                state.poll();
                if branches.is_none() && state.picker.sources.loaded { branches=Some(start.elapsed().as_secs_f64()*1000.); }
                if comparison.is_none() && state.take_change_set().is_some() { comparison=Some(start.elapsed().as_secs_f64()*1000.); }
                assert!(start.elapsed().as_secs()<30);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert_eq!(state.picker.log.len(),1);
            assert_eq!(state.picker.sources.worktrees.len(),25);
            samples.push(serde_json::json!({"branches_ms":branches.unwrap(),"comparison_ms":comparison.unwrap(),"settled_ms":start.elapsed().as_secs_f64()*1000.,"retained_branches":state.picker.sources.branches.len(),"rss_mib":crate::perf::rss_mib()}));
        }
        println!("LENZ_REPOSITORY {}",serde_json::json!({"branches":5001,"worktrees":25,"samples":samples}));
    }
    #[test]
    fn selection_title_names_the_selected_commit() {
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");
        write(root.path(),"file","second\n");commit_all(root.path(),"Second");
        let mut state=open(root.path());
        while state.take_change_set().is_some(){}
        let started=std::time::Instant::now();
        while state.picker.refs.is_none(){state.poll();assert!(started.elapsed().as_secs()<10);std::thread::sleep(std::time::Duration::from_millis(2));}
        let head=git(root.path(),&["rev-parse","HEAD"]);
        let t=state.selection_title().expect("a commit is selected");
        assert_eq!(t.text,format!("main ({})",&head.trim()[..7]));
    }

    #[test]
    fn selecting_the_shown_commit_again_loads_nothing() {
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");
        write(root.path(),"file","second\n");commit_all(root.path(),"Second");
        let mut state=open(root.path());
        while state.take_change_set().is_some(){}
        let shown=state.picker.anchor;
        state.execute(RepositoryCommand::SelectRow{row:shown,extend:false,two:false});
        assert!(state.pending.is_none(),"the shown comparison is not loaded again");
        settle(&mut state);
        assert!(state.take_change_set().is_none());
        // another commit starts loading; going back to the shown one drops that load and keeps what is on screen
        let other=(0..state.picker.rows()).find(|&r|r!=shown&&state.picker.log_row(r).is_some()).unwrap();
        state.execute(RepositoryCommand::SelectRow{row:other,extend:false,two:false});
        assert!(state.pending.is_some());
        state.execute(RepositoryCommand::SelectRow{row:shown,extend:false,two:false});
        assert!(state.pending.is_none());
        settle(&mut state);
        assert!(state.take_change_set().is_none());
    }

    #[test]
    fn switching_branches_from_file_history_never_indexes_the_old_log() {
        // the crash report's trail: Follow file history, then a branch in the sidebar, then a repaint
        let root=repo();write(root.path(),"file","1\n");commit_all(root.path(),"one");
        for n in 2..10 {write(root.path(),"file",format!("{n}\n"));commit_all(root.path(),&format!("c{n}"));}
        git(root.path(),&["checkout","-qb","side","HEAD~8"]);write(root.path(),"other","x\n");commit_all(root.path(),"side");
        git(root.path(),&["checkout","-q","main"]);
        let mut state=open(root.path());
        state.execute(RepositoryCommand::FileHistory("file".into()));
        settle(&mut state);
        assert!(state.picker.matches.as_ref().is_some_and(|m|m.len()>5));
        let rows=state.picker.sources.rows_with_palette(&state.picker.branches,SourcePalette::default());
        let side=rows.iter().position(|r|matches!(&r.action,Some(SourceAction::Branch(name,_)) if name.ends_with("side"))).unwrap();
        state.execute(RepositoryCommand::SourceSelect(side));
        // a paint between the click and the first new rows
        for r in 0..state.picker.rows() {
            if let Some(i)=state.picker.log_row(r) { let _=state.picker.log.subject(i); }
        }
        settle(&mut state);
        for r in 0..state.picker.rows() {
            if let Some(i)=state.picker.log_row(r) { let _=state.picker.log.subject(i); }
        }
    }

    #[test]
    fn catalog_repository_actions_become_commands_here() {
        use crate::catalog::RepoAction as A;
        let root=repo();
        for n in 1..3 {write(root.path(),"file",format!("{n}\n"));commit_all(root.path(),&format!("c{n}"));}
        let mut state=open(root.path());
        while state.take_change_set().is_some(){}
        assert_eq!(state.action_command(A::AllBranches,"file"),Ok(Some(RepositoryCommand::Walk(Branches::All))));
        assert_eq!(state.action_command(A::FileHistory,"file"),Ok(Some(RepositoryCommand::FileHistory("file".into()))));
        assert!(state.action_command(A::FileHistory,"").is_err(),"no file shown");
        assert!(state.action_command(A::CompareBranch{merge_base:true},"file").is_err(),"no branch selected in the sidebar");
        assert!(state.action_command(A::OpenWorktree,"file").is_err(),"no worktree selected");
        assert_eq!(state.action_command(A::Back,"file"),Ok(None),"adapters step back themselves");
        // select the main branch in the sidebar: comparing it with the current branch works
        let rows=state.picker.sources.rows_with_palette(&state.picker.branches,SourcePalette::default());
        let main=rows.iter().position(|r|matches!(&r.action,Some(SourceAction::Branch(_,_)))).expect("a branch row");
        state.execute(RepositoryCommand::SourceSelect(main));
        assert!(matches!(state.action_command(A::CompareBranch{merge_base:false},"file"),Ok(Some(RepositoryCommand::Compare{merge_base:false,..}))));
    }

    #[test]
    fn back_returns_to_the_head_commit_the_log_opened_on() {
        let root=repo();
        for n in 1..4 {write(root.path(),"file",format!("{n}\n"));commit_all(root.path(),&format!("c{n}"));}
        let mut state=open(root.path());
        while state.take_change_set().is_some(){}
        let head=state.picker.anchor;
        state.execute(RepositoryCommand::Move{delta:1,extend:false,two:false});
        settle(&mut state);
        assert_ne!(state.picker.anchor,head);
        state.go_back();
        settle(&mut state);
        assert_eq!(state.picker.anchor,head,"Back selects HEAD again, though the log opened on it");
    }

    #[test]
    fn back_returns_from_file_history_to_the_commit_file_and_line() {
        let root=repo();
        for n in 1..6 {write(root.path(),"file",format!("{n}\n"));write(root.path(),"other",format!("o{n}\n"));commit_all(root.path(),&format!("c{n}"));}
        let mut state=open(root.path());
        while state.take_change_set().is_some(){}
        assert!(!state.can_go_back());
        // the user picks an older commit and reads `other` at line 1
        let row=(0..state.picker.rows()).find(|&r|state.picker.log_row(r).is_some_and(|i|state.picker.log.subject(i)=="c3")).unwrap();
        state.execute(RepositoryCommand::SelectRow{row,extend:false,two:false});
        settle(&mut state);
        let c3=state.picker.log.id(state.picker.log_row(state.picker.anchor).unwrap());
        state.set_position(Some("other".into()),Some(1));
        // then shows `file`'s history
        state.execute(RepositoryCommand::FileHistory("file".into()));
        settle(&mut state);
        assert!(state.picker.file_paths.is_some());
        assert!(state.can_go_back());
        state.go_back();
        settle(&mut state);
        assert!(state.picker.file_paths.is_none(),"Back leaves file history");
        assert_eq!(state.place_source(),Some(crate::place::Source::Log{commit:c3.clone()}),"Back selects the commit again");
        assert_eq!(state.take_nav_restore(),Some(crate::place::Restore{commit:Some(c3.clone()),file:Some("other".into()),line:Some(1)}));
        assert!(state.can_go_forward()&&!state.can_go_back()||state.can_go_back());
        state.go_forward();
        settle(&mut state);
        assert!(state.picker.file_paths.is_some(),"Forward returns to file history");
    }

    #[test]
    fn re_selecting_the_shown_commit_is_not_a_step() {
        let root=repo();write(root.path(),"file","1\n");commit_all(root.path(),"c1");write(root.path(),"file","2\n");commit_all(root.path(),"c2");
        let mut state=open(root.path());
        let shown=state.picker.anchor;
        state.execute(RepositoryCommand::SelectRow{row:shown,extend:false,two:false});
        assert!(!state.can_go_back());
    }

    #[test]
    fn log_streams_before_the_sidebar_inventory_and_only_once(){
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");
        write(root.path(),"file","second\n");commit_all(root.path(),"Second");
        let request=crate::cli::parse(&["log".into()],root.path()).unwrap();
        let mut state=RepositorySession::open(&request,root.path().into());
        assert!(state.picker.loading,"the commit list must not wait for branches, tags and stashes");
        settle(&mut state);
        assert_eq!(state.picker.log.len(),2,"the inventory must not restart the stream");
    }
    #[test]
    fn the_log_reads_a_window_beyond_the_view_and_more_as_it_scrolls(){
        let root=repo();for i in 0..30{write(root.path(),"file",format!("{i}\n"));commit_all(root.path(),&format!("c{i}"));}
        let mut picker=Picker::new(root.path().into(),true);picker.window=5;picker.start_stream();
        let poll=|picker:&mut Picker|{let started=std::time::Instant::now();while picker.loading{picker.poll_stream(4.0);assert!(started.elapsed().as_secs()<10);std::thread::sleep(std::time::Duration::from_millis(2));}};
        poll(&mut picker);
        assert!(picker.paused&&!picker.loading,"a long history pauses instead of loading everything");
        assert!(picker.log.len()>=5&&picker.log.len()<30,"read {} commits",picker.log.len());
        assert_eq!(picker.more_label().unwrap(),format!("{}+ commits, more as you scroll",picker.log.len()));
        let before=picker.log.len();picker.scroll=before as f32;
        assert!(picker.resume_stream(4.0),"scrolling near the end resumes reading");poll(&mut picker);
        assert!(picker.log.len()>before,"more rows arrived after scrolling");
        picker.set_filter("c0".into());assert!(!picker.resume_stream(4.0)||picker.loading);poll(&mut picker);
        assert_eq!(picker.log.len(),30,"a filter needs the whole history");assert!(!picker.paused&&picker.more_label().is_none());
    }
    #[test]
    fn history_requests_reject_obsolete_follow_branch_filter_and_aba_results(){
        let root=repo();write(root.path(),"first","first\n");write(root.path(),"second","second\n");commit_all(root.path(),"First");
        let mut state=open(root.path());
        let old=state.picker.begin_file_history("first".into());let result=old.read_followed();
        let latest=state.picker.begin_file_history("second".into());let latest_result=latest.read_followed();
        assert!(matches!(state.picker.accept_file_history(&old,result),HistoryAcceptance::Stale));
        assert!(matches!(state.picker.accept_file_history(&latest,latest_result),HistoryAcceptance::Followed{pick:true}));
        assert_eq!(state.picker.file_paths.as_ref().unwrap().0,"second");
        let filtered=state.picker.begin_deep_filter();let filtered_result=filtered.read_filter();
        let followed=state.picker.begin_file_history("first".into());
        assert!(matches!(state.picker.accept_deep_filter(&filtered,filtered_result),HistoryAcceptance::Stale));
        let old_result=followed.read_followed();state.picker.restart(Branches::One("HEAD".into()),None);
        assert!(matches!(state.picker.accept_file_history(&followed,old_result),HistoryAcceptance::Stale));
        state.picker.set_filter("First".into());let first=state.picker.begin_deep_filter();let result=first.read_filter();
        state.picker.set_filter("Second".into());state.picker.set_filter("First".into());
        assert!(matches!(state.picker.accept_deep_filter(&first,result),HistoryAcceptance::Stale));
        let followed=state.picker.begin_file_history("first".into());let result=followed.read_followed();
        state.picker.set_filter("different".into());
        assert!(matches!(state.picker.accept_file_history(&followed,result),HistoryAcceptance::Stale));
    }
    #[test]
    fn clear_history_rejects_held_results_and_cancels_waiting_follow_selection(){
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");
        let mut state=open(root.path());let request=state.picker.begin_file_history("file".into());let result=request.read_followed();
        state.picker.log=Log::default();
        assert!(matches!(state.picker.accept_file_history(&request,result),HistoryAcceptance::Followed{pick:false}));
        assert!(state.picker.want.is_some()&&state.picker.pick_on_want);
        let obsolete=state.picker.begin_file_history("file".into());let held=obsolete.read_followed();
        state.execute(RepositoryCommand::ClearFilter);
        assert!(state.picker.want.is_none()&&!state.picker.pick_on_want);assert!(state.picker.file_paths.is_none()&&state.picker.matches.is_none());
        assert!(matches!(state.picker.accept_file_history(&obsolete,held),HistoryAcceptance::Stale));
    }
    #[test]
    fn explicit_selection_survives_a_delayed_wanted_commit_stream_batch(){
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");
        write(root.path(),"file","second\n");commit_all(root.path(),"Second");
        for mouse in [true,false]{
            let mut picker=Picker::new(root.path().into(),true);picker.start_stream();
            let (sender,held)=std::sync::mpsc::channel();
            let original=std::mem::replace(&mut picker.stream.as_mut().unwrap().batches,held);
            let mut commits:Vec<_>=original.into_iter().flat_map(Result::unwrap).collect();assert_eq!(commits.len(),2);
            let old=commits[1].id.clone();let visible=commits[0].id.clone();picker.log.extend(commits.drain(..1));
            picker.want=Some(old);picker.pick_on_want=true;
            if mouse{picker.select_row(1,false,false);}else{picker.move_selection(1,false,false);}
            sender.send(Ok(commits)).unwrap();drop(sender);
            let (changed,pick)=picker.poll_stream(8.0);assert!(changed&&!pick);assert!(!picker.loading);
            assert_eq!(picker.log.id(picker.log_row(picker.anchor).unwrap()),visible);assert!(picker.want.is_none()&&!picker.pick_on_want);
        }
    }
    #[test]
    fn initial_log_pins_root_commit_and_requested_branch_then_survives_head_movement(){
        let root=repo();write(root.path(),"root","first\n");commit_all(root.path(),"Root");
        let first=String::from_utf8_lossy(&crate::git::run(root.path(),&["rev-parse","HEAD"]).unwrap()).trim().to_string();
        git(root.path(),&["branch","earlier"]);write(root.path(),"later","second\n");commit_all(root.path(),"Later");
        let request=crate::cli::parse(&["log".into(),"earlier".into()],root.path()).unwrap();
        let (set,spec,selected)=initial_log_comparison(root.path(),&request.spec).unwrap();
        assert_eq!(selected.as_deref(),Some(first.as_str()));assert_eq!(set.comparisons.len(),1);assert_eq!(set.comparisons[0].path,PathBuf::from("root"));
        let base=crate::blame::parent_or_empty_tree(root.path(),&first).unwrap();assert_eq!(spec,branch_comparison(&base,&first,false));
        let mut state=RepositorySession::open(&request,root.path().into());settle(&mut state);
        assert_eq!(state.picker.branches,initial_log_branches(&request.spec));
        assert_eq!(state.picker.log.id(state.picker.log_row(state.picker.anchor).unwrap()),first);
        assert_eq!(state.take_change_set().unwrap().1,spec);
        write(root.path(),"third","third\n");commit_all(root.path(),"Third");
        assert_eq!(ChangeSet::load(&spec,root.path()).unwrap().comparisons.len(),1);
        state.execute(RepositoryCommand::RefreshRepository);settle(&mut state);assert_eq!(state.take_change_set().unwrap().1,spec);
        write(root.path(),"root","dirty\n");
        let (_,local,selected)=initial_repository_comparison(root.path()).unwrap();assert_eq!(local,local_spec());assert!(selected.is_none());
    }
    #[test]
    fn a_wanted_commit_is_selected_and_reported_as_the_place_source(){
        let root=repo();write(root.path(),"a","1\n");commit_all(root.path(),"One");
        let first=String::from_utf8_lossy(&crate::git::run(root.path(),&["rev-parse","HEAD"]).unwrap()).trim().to_string();
        write(root.path(),"a","2\n");commit_all(root.path(),"Two");
        let request=crate::cli::parse_with(&["log".into()],root.path(),&Default::default()).unwrap();
        let mut state=RepositorySession::open(&request,root.path().into());
        state.want_commit(first.clone());
        settle(&mut state);
        assert!(!state.settled(),"the wanted commit's Change Set waits to be taken");
        let (_,spec)=state.take_change_set().unwrap();
        assert!(state.settled());
        assert_eq!(state.place_source(),Some(crate::place::Source::Log{commit:first.clone()}));
        let Spec::Git{revs,..}=spec else{panic!()};assert_eq!(revs.last(),Some(&first));
    }
    #[test]
    fn a_wanted_commit_the_log_never_reaches_is_dropped_and_the_session_settles(){
        let root=repo();write(root.path(),"a","1\n");commit_all(root.path(),"One");
        let amended=String::from_utf8_lossy(&crate::git::run(root.path(),&["rev-parse","HEAD"]).unwrap()).trim().to_string();
        git(root.path(),&["commit","-q","--amend","-m","One, amended"]);
        let request=crate::cli::parse_with(&["log".into()],root.path(),&Default::default()).unwrap();
        let mut state=RepositorySession::open(&request,root.path().into());
        state.want_commit(amended.clone());
        settle(&mut state);let _=state.take_change_set();
        assert!(state.settled(),"a commit outside the log must not hold the session open");
        assert!(!state.restored(&amended),"the log could not select it");
    }
    #[test]
    fn wanting_the_commit_the_log_starts_on_loads_it_once(){
        let root=repo();write(root.path(),"a","1\n");commit_all(root.path(),"One");write(root.path(),"a","2\n");commit_all(root.path(),"Two");
        let head=String::from_utf8_lossy(&crate::git::run(root.path(),&["rev-parse","HEAD"]).unwrap()).trim().to_string();
        let request=crate::cli::parse_with(&["log".into()],root.path(),&Default::default()).unwrap();
        let mut state=RepositorySession::open(&request,root.path().into());
        state.want_commit(head.clone());
        let mut sets=0;
        for _ in 0..3{settle(&mut state);sets+=usize::from(state.take_change_set().is_some());}
        assert_eq!(sets,1,"HEAD's Change Set loads once");
        assert!(state.settled()&&state.restored(&head));
    }
    #[test]
    fn initial_follow_selects_last_file_commit_regardless_of_startup_job_order(){
        let root=repo();write(root.path(),"followed","first\n");commit_all(root.path(),"File commit");
        let first=String::from_utf8_lossy(&crate::git::run(root.path(),&["rev-parse","HEAD"]).unwrap()).trim().to_string();
        write(root.path(),"other","second\n");commit_all(root.path(),"Other commit");
        let request=crate::cli::parse(&["log".into(),"--follow".into(),"followed".into()],root.path()).unwrap();
        for _ in 0..8{
            let mut state=RepositorySession::open(&request,root.path().into());settle(&mut state);
            let (set,spec)=state.take_change_set().unwrap();
            assert_eq!(state.picker.log.id(state.picker.log_row(state.picker.anchor).unwrap()),first);
            let Spec::Git{revs,paths,..}=spec else{panic!()};assert_eq!(revs.last(),Some(&first));assert_eq!(paths,["followed"]);
            assert_eq!(set.comparisons.len(),1);assert_eq!(set.comparisons[0].path,PathBuf::from("followed"));
        }
    }
    #[test]
    fn a_commit_opens_on_its_own_and_compares_with_each_branch(){
        let root=repo();write(root.path(),"a","1\n");commit_all(root.path(),"A");write(root.path(),"a","2\n");commit_all(root.path(),"B");
        crate::git::run(root.path(),&["branch","topic"]).unwrap();
        let request=crate::cli::parse(&["log".into()],root.path()).unwrap();let mut state=RepositorySession::open(&request,root.path().into());settle(&mut state);state.take_change_set();
        let head=state.picker.log.id(0);let row=state.picker.display_row(0).unwrap();
        state.execute_on_row(row,RepositoryCommand::OpenCommit);
        assert_eq!(state.take_effects(),[RepositoryEffect::OpenRevisions{root:state.picker.root.clone(),revs:vec![format!("{head}^1"),head.clone()]}]);
        let targets=state.branch_targets(row);
        assert_eq!(targets.iter().map(|(label,_)|label.as_str()).collect::<Vec<_>>(),["⎇ main","⎇ topic"]);
        state.execute(targets[1].1.clone());settle(&mut state);
        let (_,spec)=state.take_change_set().unwrap();let Spec::Git{revs,..}=spec else{panic!()};assert_eq!(revs,[head,"refs/heads/topic".to_string()]);
        assert_eq!(state.row_disabled(0,&RepositoryCommand::OpenCommit),Some("No commit selected"));
    }
    #[test]
    fn file_history_from_a_hidden_picker_loads_the_sidebar_and_the_log(){
        let root=repo();write(root.path(),"a","1\n");commit_all(root.path(),"A");write(root.path(),"b","other\n");commit_all(root.path(),"B");
        let request=crate::cli::parse(&[],root.path()).unwrap();
        let mut state=RepositorySession::open(&request,root.path().into());
        assert!(!state.picker.shown&&state.picker.log.is_empty());
        state.execute(RepositoryCommand::FileHistory("a".into()));settle(&mut state);
        assert!(state.picker.shown&&state.picker.sources.loaded);
        assert_eq!(state.picker.log.len(),2);
        let (_,spec)=state.take_change_set().unwrap();
        let Spec::Git{paths,..}=spec else{panic!()};assert_eq!(paths,["a"]);
    }
    #[cfg(unix)]
    #[test]
    fn file_history_through_a_symlinked_path_is_not_dropped_as_stale(){
        // git reports the resolved root; a session opened through a link must not treat its own requests as stale
        let root=repo();write(root.path(),"a","1\n");commit_all(root.path(),"A");write(root.path(),"b","other\n");commit_all(root.path(),"B");
        let outer=tempfile::tempdir().unwrap();let link=outer.path().join("link");std::os::unix::fs::symlink(root.path(),&link).unwrap();
        let request=crate::cli::parse(&[],&link).unwrap();
        let mut state=RepositorySession::open(&request,link.clone());
        state.execute(RepositoryCommand::FileHistory("a".into()));settle(&mut state);
        assert_eq!(state.picker.log.len(),2);
        let (_,spec)=state.take_change_set().expect("file history reaches the comparison");
        let Spec::Git{paths,..}=spec else{panic!()};assert_eq!(paths,["a"]);
    }
    #[test]
    fn file_history_from_a_subdirectory_is_not_dropped_as_stale(){
        let root=repo();write(root.path(),"sub/a","1\n");commit_all(root.path(),"A");write(root.path(),"b","other\n");commit_all(root.path(),"B");
        let sub=root.path().join("sub");
        let request=crate::cli::parse(&[],&sub).unwrap();
        let mut state=RepositorySession::open(&request,sub);
        state.execute(RepositoryCommand::FileHistory("sub/a".into()));settle(&mut state);
        let (_,spec)=state.take_change_set().expect("file history reaches the comparison");
        let Spec::Git{paths,..}=spec else{panic!()};assert_eq!(paths,["sub/a"]);
    }
    #[test]
    fn initial_unborn_repository_stays_local_and_invalid_log_revision_errors(){
        let root=repo();let (set,spec,selected)=initial_repository_comparison(root.path()).unwrap();
        assert!(set.comparisons.is_empty());assert_eq!(spec,local_spec());assert!(selected.is_none());
        let request=crate::cli::parse(&["log".into(),"missing".into()],root.path()).unwrap();
        assert!(initial_log_comparison(root.path(),&request.spec).is_err());
    }
    #[test]
    fn repository_refresh_keeps_cached_pr_and_never_schedules_github_work(){
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");write(root.path(),"file","second\n");commit_all(root.path(),"Second");
        let request=crate::cli::parse(&["log".into()],root.path()).unwrap();let mut state=RepositorySession::open(&request,root.path().into());settle(&mut state);
        assert_eq!(state.disabled(&RepositoryCommand::RefreshPr),Some("No pull request is open"));
        let found:crate::pr::PullRequest=serde_json::from_str(include_str!("../fixtures/pr/view.json")).unwrap();
        let cached=crate::pr::ListedPr{number:found.number,title:found.title.clone(),author:found.author.clone(),head_ref_name:found.head_ref_name.clone(),base_ref_name:found.base_ref_name.clone(),is_draft:found.is_draft,updated_at:"cached".into()};
        state.picker.sources.prs.push(cached.clone());let fetched=crate::pr::Fetched{number:found.number,merge_base:state.picker.log.id(1),head:state.picker.log.id(0)};
        let metadata=found.head_ref_oid.clone();state.current_spec=Some(fetched.spec());state.pr=Some(crate::pr_session::PrState::new(found,fetched));
        state.execute(RepositoryCommand::RefreshRepository);let started=std::time::Instant::now();
        while state.busy(){assert!(state.prs.is_none()&&state.opened.is_none()&&state.pr.as_ref().is_none_or(|p|!p.review_loading())&&state.moved.is_none(),"local refresh must not create GitHub jobs");state.poll();assert!(started.elapsed().as_secs()<10);std::thread::sleep(std::time::Duration::from_millis(2));}
        assert_eq!(state.picker.sources.prs,vec![cached]);assert_eq!(state.pr.as_ref().unwrap().pr.head_ref_oid,metadata);
    }
    #[test]
    fn shared_quiet_refresh_keeps_range_ids_and_ignores_obsolete_filter_results(){
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");
        write(root.path(),"file","second\n");commit_all(root.path(),"Second");
        let request=crate::cli::parse(&["log".into()],root.path()).unwrap();let mut state=RepositorySession::open(&request,root.path().into());settle(&mut state);
        state.execute(RepositoryCommand::Move{delta:1,extend:true,two:false});settle(&mut state);
        let ids=|p:&Picker|[Some(p.anchor),p.second].map(|row|row.and_then(|row|p.log_row(row)).map(|row|p.log.id(row)));
        state.picker.set_filter("author:t".into());let before=ids(&state.picker);
        let old=state.picker.begin_repository_refresh();let old_filters=old.read_filters();let fresh=state.picker.begin_repository_refresh();
        assert!(state.picker.apply_refresh_filters(&old,old_filters).is_empty());assert!(state.picker.refreshing());
        state.picker.start_stream();state.picker.apply_refresh_filters(&fresh,fresh.read_filters());settle(&mut state);
        assert_eq!(ids(&state.picker),before);assert!(state.picker.range);assert_eq!(state.picker.filter,"author:t");
    }
    #[test]
    fn refreshing_new_head_keeps_selected_commit_and_comparison_coherent() {
        let root=repo();write(root.path(),"file","first\n");commit_all(root.path(),"First");
        write(root.path(),"file","second\n");commit_all(root.path(),"Second");
        let request=crate::cli::parse(&["log".into()],root.path()).unwrap();
        let mut state=RepositorySession::open(&request,root.path().into());settle(&mut state);
        let (_,before)=state.take_change_set().unwrap();assert!(!state.take_refresh_placement());
        let selected=state.picker.log.id(state.picker.log_row(state.picker.anchor).unwrap());
        assert!(matches!(&before,Spec::Git{revs,..} if revs.last()==Some(&selected)));
        write(root.path(),"new","new\n");commit_all(root.path(),"Third");
        state.execute(RepositoryCommand::RefreshRepository);settle(&mut state);
        let (set,after)=state.take_change_set().unwrap();assert!(state.take_refresh_placement());assert!(!state.take_refresh_placement());assert_eq!(before,after);
        assert_eq!(state.picker.log.id(state.picker.log_row(state.picker.anchor).unwrap()),selected);
        assert_eq!(set.comparisons.len(),1,"{before:?} {after:?} selected={selected}");assert_eq!(set.comparisons[0].path,"file");
        assert_eq!(state.picker.log.subject(0),"Third");
    }
    #[test]
    fn refreshing_after_resolve_and_stage_updates_inventory_without_resetting_controls() {
        let dir=repo();write(dir.path(),"file","base\n");commit_all(dir.path(),"base");
        git(dir.path(), &["checkout","-qb","side"]);write(dir.path(),"file","side\n");commit_all(dir.path(),"side");
        git(dir.path(), &["checkout","-q","main"]);write(dir.path(),"file","main\n");commit_all(dir.path(),"main");
        assert!(!crate::git::testing::try_git(dir.path(), &["merge","side"]));
        let mut state=open(dir.path());
        assert!(state.picker.sources.conflicts.as_ref().unwrap().lines.iter().any(|line|line.contains("1 of 1 left")));
        state.execute(RepositoryCommand::SourceFilter("side".into()));
        state.execute(RepositoryCommand::Source(SourceAction::Toggle("Branches")));
        state.execute(RepositoryCommand::Filter("path:file".into()));
        state.execute(RepositoryCommand::Focus(Pane::Sidebar));
        let old_collapse=state.picker.sources.collapsed.clone();
        write(dir.path(),"file","resolved\n");git(dir.path(), &["add","file"]);
        write(dir.path(),"new","untracked\n");git(dir.path(), &["tag","after-resolve"]);
        state.execute(RepositoryCommand::Source(SourceAction::Local));settle(&mut state);
        state.execute(RepositoryCommand::RefreshRepository);settle(&mut state);
        let entry=state.picker.sources.conflicts.as_ref().unwrap();
        assert!(entry.lines.iter().any(|line|line.contains("All resolved")));assert_eq!(entry.copy.as_deref(),Some("git merge --continue"));
        assert_eq!(state.picker.sources.local,2);
        assert!(state.picker.sources.tags.iter().any(|tag|tag.name=="after-resolve"));
        assert_eq!(state.picker.sources.filter,"side");assert_eq!(state.picker.sources.collapsed,old_collapse);
        assert_eq!(state.picker.filter,"path:file");assert_eq!(state.picker.focus,Pane::Sidebar);
        assert_eq!(state.comparisons().len(),2);assert!(state.prs.is_none(),"local refresh must not start a GitHub request");
        git(dir.path(), &["commit","-qm","resolved merge"]);
        state.execute(RepositoryCommand::RefreshRepository);settle(&mut state);
        assert!(state.picker.sources.conflicts.is_none());assert_eq!(state.picker.sources.local,1);
        assert_eq!(state.comparisons().len(),1);
    }
    #[test]
    fn repository_selection_range_two_and_parent_use_gui_semantics() {
        let dir = repo();
        write(dir.path(), "file", "one\n");
        commit_all(dir.path(), "first");
        write(dir.path(), "file", "two\n");
        commit_all(dir.path(), "second");
        write(dir.path(), "file", "three\n");
        commit_all(dir.path(), "third");
        let mut state = open(dir.path());
        state.execute(RepositoryCommand::Move {
            delta: 1,
            extend: false,
            two: false,
        });
        settle(&mut state);
        let (_, spec) = state.take_change_set().unwrap();
        let Spec::Git { revs, .. } = spec else {
            panic!()
        };
        assert_eq!(
            revs,
            vec![
                format!("{}^1", state.picker.log.id(1)),
                state.picker.log.id(1)
            ]
        );
        state.execute(RepositoryCommand::Move {
            delta: 1,
            extend: true,
            two: false,
        });
        settle(&mut state);
        let Spec::Git { revs, .. } = state.take_change_set().unwrap().1 else {
            panic!()
        };
        assert_eq!(revs[0], history::empty_tree(dir.path()).unwrap());
        assert_eq!(revs[1], state.picker.log.id(1));
        state.execute(RepositoryCommand::Move {
            delta: -1,
            extend: false,
            two: true,
        });
        settle(&mut state);
        let Spec::Git { revs, .. } = state.take_change_set().unwrap().1 else {
            panic!()
        };
        assert_eq!(revs, vec![state.picker.log.id(1), state.picker.log.id(1)]);
    }
    #[test]
    fn sources_local_index_branch_stash_and_filters_share_models() {
        let dir = repo();
        write(dir.path(), "old", "one\n");
        commit_all(dir.path(), "first");
        git(dir.path(), &["branch", "other"]);
        git(dir.path(), &["mv", "old", "new"]);
        commit_all(dir.path(), "rename");
        write(dir.path(), "new", "two\n");
        git(dir.path(), &["stash", "push", "-m", "saved"]);
        write(dir.path(), "new", "three\n");
        git(dir.path(), &["add", "new"]);
        let mut state = open(dir.path());
        assert_eq!(state.picker.sources.branches.len(), 2);
        assert_eq!(state.picker.sources.stashes.len(), 1);
        assert_eq!(state.take_change_set().unwrap().0.groups[0].0, "Staged");
        state.execute(RepositoryCommand::Index);
        settle(&mut state);
        assert!(
            state
                .take_change_set()
                .unwrap()
                .0
                .comparisons
                .iter()
                .any(|comparison| comparison.path == "new")
        );
        state.execute(RepositoryCommand::FileHistory("new".into()));
        settle(&mut state);
        assert_eq!(state.picker.matches.as_ref().unwrap().len(), 2);
        assert!(
            state
                .picker
                .file_paths
                .as_ref()
                .unwrap()
                .1
                .values()
                .any(|path| path == "old")
        );
        state.execute(RepositoryCommand::ClearFilter);
        state.execute(RepositoryCommand::Filter("path:new rename".into()));
        state.execute(RepositoryCommand::ApplyFilter);
        settle(&mut state);
        assert_eq!(state.picker.matches.as_ref().unwrap().len(), 1);
        state.execute(RepositoryCommand::Source(SourceAction::Stash(
            state.picker.sources.stashes[0].sha.clone(),
        )));
        settle(&mut state);
        assert_eq!(state.take_change_set().unwrap().0.groups[0].0, "Changes");
    }
    #[test]
    fn merge_parent_branch_comparisons_and_graph_write_work_in_disposable_repo() {
        let dir = repo();
        write(dir.path(), "file", "one\n");
        commit_all(dir.path(), "root");
        let current = String::from_utf8_lossy(
            &crate::git::run(dir.path(), &["branch", "--show-current"]).unwrap(),
        )
        .trim()
        .to_string();
        git(dir.path(), &["checkout", "-qb", "topic"]);
        write(dir.path(), "topic", "topic\n");
        commit_all(dir.path(), "topic commit");
        git(dir.path(), &["checkout", &current]);
        write(dir.path(), "main", "main\n");
        commit_all(dir.path(), "main commit");
        git(
            dir.path(),
            &["merge", "--no-ff", "topic", "-m", "merge commit"],
        );
        let mut state = open(dir.path());
        assert_eq!(state.picker.merge_parents(), 2);
        state.execute(RepositoryCommand::Parent(1));
        settle(&mut state);
        let Spec::Git { revs, .. } = state.take_change_set().unwrap().1 else {
            panic!()
        };
        assert_eq!(
            revs,
            vec![
                format!("{}^2", state.picker.log.id(0)),
                state.picker.log.id(0)
            ]
        );
        state.execute(RepositoryCommand::Compare {
            left: "HEAD".into(),
            right: "refs/heads/topic".into(),
            merge_base: true,
        });
        settle(&mut state);
        let (_, spec) = state.take_change_set().unwrap();
        assert_eq!(spec, branch_comparison("refs/heads/topic", "HEAD", true), "what the current branch adds since splitting off from topic");
        state.execute(RepositoryCommand::Compare {
            left: "HEAD".into(),
            right: "refs/heads/topic".into(),
            merge_base: false,
        });
        settle(&mut state);
        assert!(!state.take_change_set().unwrap().0.comparisons.is_empty());
        state.execute(RepositoryCommand::WriteGraph);
        settle(&mut state);
        assert!(history::has_commit_graph(dir.path()));
    }
    #[test]
    fn choosing_a_branch_leaves_a_comparison() {
        let dir = repo(); write(dir.path(), "file", "one\n"); commit_all(dir.path(), "first");
        git(dir.path(), &["branch", "side"]);
        let request = crate::cli::parse(&["log".into()], dir.path()).unwrap();
        let mut state = RepositorySession::open(&request, dir.path().into());
        settle(&mut state);
        state.execute(RepositoryCommand::Compare { left: "HEAD".into(), right: "refs/heads/side".into(), merge_base: true }); settle(&mut state);
        assert!(state.picker.compare.is_some());
        let sha = git(dir.path(), &["rev-parse", "side"]).trim().to_string();
        state.execute(RepositoryCommand::Source(SourceAction::Branch("refs/heads/side".into(), sha))); settle(&mut state);
        assert!(state.picker.compare.is_none(), "the branch's own log, not the comparison");
        assert_eq!(state.picker.branches, Branches::One("refs/heads/side".into()));
    }
    #[test]
    fn sidebar_toggle_loads_hidden_picker_and_escape_restores_all_branches() {
        let dir = repo(); write(dir.path(), "file", "one\n"); commit_all(dir.path(), "first");
        let request = crate::cli::parse(&[], dir.path()).unwrap();
        let mut state = RepositorySession::open(&request, dir.path().into());
        assert!(!state.picker.shown);
        state.execute(RepositoryCommand::ToggleSidebar); settle(&mut state);
        assert!(state.picker.shown && state.picker.sidebar && state.picker.sources.loaded);
        state.execute(RepositoryCommand::Compare { left: "HEAD".into(), right: "HEAD".into(), merge_base: true }); settle(&mut state);
        assert!(state.picker.compare.is_some());
        state.execute(RepositoryCommand::ClearFilter); settle(&mut state);
        assert_eq!(state.picker.branches, Branches::All); assert!(state.picker.compare.is_none());
    }
    #[test]
    fn obsolete_pick_and_filter_cannot_replace_latest_command() {
        let dir = repo();
        write(dir.path(), "file", "one\n");
        commit_all(dir.path(), "first");
        write(dir.path(), "file", "two\n");
        commit_all(dir.path(), "second");
        let mut state = open(dir.path());
        state.execute(RepositoryCommand::Filter("first".into()));
        state.execute(RepositoryCommand::ApplyFilter);
        state.execute(RepositoryCommand::Filter("second".into()));
        state.execute(RepositoryCommand::ApplyFilter);
        state.execute(RepositoryCommand::Move {
            delta: 1,
            extend: false,
            two: false,
        });
        state.execute(RepositoryCommand::Source(SourceAction::Local));
        settle(&mut state);
        assert_eq!(state.picker.matches.as_ref().unwrap().len(), 1);
        assert_eq!(
            state
                .picker
                .log
                .subject(state.picker.matches.as_ref().unwrap()[0]),
            "second"
        );
        assert_eq!(state.take_change_set().unwrap().1, local_spec());
    }
}
