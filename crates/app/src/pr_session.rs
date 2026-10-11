//! Read-only PR review behavior shared by both frontends.
use crate::{
    changeset::{ChangeSet, Comparison, Content},
    pr::{self, Fetched, PullRequest, Review, Thread, ViewStatus, ViewedMarks},
};
use crate::{pr_conversation::{self, Loaded, Page}, rich::Target};
use std::{collections::HashSet,path::Path,sync::{Arc,Weak,mpsc}};

/// The PR's reading tabs; the diff is the third, Files changed.
#[derive(Debug,Clone,Copy,PartialEq,Eq,Hash)]
pub enum DetailsTab {Conversation,Commits}
impl DetailsTab {
    pub const ALL:[Self;2]=[Self::Conversation,Self::Commits];
    pub fn label(self)->&'static str{match self{Self::Conversation=>"Conversation",Self::Commits=>"Commits"}}
    pub fn command_label(self)->&'static str{match self{Self::Conversation=>"Show PR conversation",Self::Commits=>"Show PR commits"}}
}
/// The diff's tab label.
pub const FILES_CHANGED:&str="Files changed";
#[derive(Clone)]
pub enum PrCommand {
    Show(DetailsTab),Close,
    /// Conversation, scrolled to the first of these threads (a thread marker in the diff).
    ShowThreads(Vec<Thread>),
    /// Fold or unfold a thread, or open or close a commit, by its page key.
    Toggle(String),
}
#[derive(Clone)]
pub struct Details {pub tab:DetailsTab}
/// The pull request on screen.
pub struct PrState {
    pub pr: PullRequest,
    pub fetched: Fetched,
    pub review: Option<Review>,
    pub marks: Option<ViewedMarks>,
    details:Option<Details>,
    /// Threads unfolded (or folded) and commits opened, by page key.
    toggled:HashSet<String>,
    /// Where the next page should scroll to once.
    reveal:Option<Target>,
    /// The commit Files changed shows, when one of the PR's commits is open.
    pub shown_commit:Option<String>,
    /// Bumped whenever the pages change, so adapters can keep a laid-out page.
    generation:u64,
    review_error:Option<String>,
    review_token:Arc<()>,
    review_job:Option<mpsc::Receiver<Option<Result<Review,pr::PrError>>>>,
}

/// What opening a PR needs, gathered off the UI thread.
pub struct Opened {
    pub pr: PullRequest,
    pub fetched: Fetched,
    pub set: ChangeSet,
}

/// Resolve, fetch and list a PR (`target`: a number, a URL, or `None` for the current branch's).
pub fn open(root: &Path, target: Option<&str>) -> Result<Opened, pr::PrError> {
    let gh = pr::Gh::new(root);
    gh.check()?;
    let found = gh.view(target)?;
    let repo = found.repo()?;
    pr::ensure_clone(root, &repo)?;
    let url = gh.fetch_url(&repo)?;
    let fetched = pr::fetch(root, &url, found.number, &found.base_ref_name)?;
    let set = ChangeSet::load(&fetched.spec(), root).map_err(pr::PrError::Git)?;
    Ok(Opened {
        pr: found,
        fetched,
        set,
    })
}

/// The id of the version a Viewed Mark is keyed on: the right Side's blob (the left one for a deleted file).
fn blob(c: &crate::changeset::Comparison) -> Option<String> {
    match (&c.right, &c.left) {
        (Content::Blob(id), _) | (Content::Empty, Content::Blob(id)) => Some(id.clone()),
        _ => None,
    }
}

impl PrState {
    pub fn new(pr: PullRequest, fetched: Fetched) -> Self {
        Self {
            pr,
            fetched,
            review: None,
            marks: pr::state_dir().and_then(|directory| ViewedMarks::open(&directory).ok()),
            details:None,toggled:HashSet::new(),reveal:None,shown_commit:None,generation:0,review_error:None,
            review_token:Arc::new(()),review_job:None,
        }
    }
    pub fn details(&self)->Option<&Details>{self.details.as_ref()}
    pub fn execute(&mut self,command:PrCommand){
        self.generation+=1;
        match command {
            PrCommand::Close=>self.details=None,
            PrCommand::Show(tab)=>self.details=Some(Details{tab}),
            PrCommand::ShowThreads(threads)=>{
                self.details=Some(Details{tab:DetailsTab::Conversation});
                if let Some(t)=threads.first(){
                    // a folded thread opens to be read
                    let key=pr_conversation::thread_key(t);
                    if (t.is_resolved||t.is_outdated)&&!self.toggled.contains(&key){self.toggled.insert(key.clone());}
                    self.reveal=Some(match (t.line,t.is_outdated){(Some(line),false)=>Target::Line{path:t.path.clone(),line,left:t.diff_side==crate::view::Side::Left},_=>Target::Toggle(key)});
                }
            }
            PrCommand::Toggle(key)=>{if !self.toggled.remove(&key){self.toggled.insert(key);}}
        }
    }
    /// Changes whenever [`page`](Self::page) would.
    pub fn generation(&self)->u64{self.generation}
    /// The open tab laid out at `width` columns; `now` is Unix seconds.
    pub fn page(&self,width:usize,now:i64)->Option<Page>{
        let loaded=match (&self.review,&self.review_error){(Some(r),_)=>Loaded::Ready(r),(None,Some(e))=>Loaded::Failed(e),_=>Loaded::Loading};
        Some(match self.details.as_ref()?.tab {
            DetailsTab::Conversation=>pr_conversation::conversation(&self.pr,loaded,&self.toggled,width,now),
            DetailsTab::Commits=>pr_conversation::commits(&self.pr,loaded,&self.toggled,self.shown_commit.as_deref(),width,now),
        })
    }
    /// The place the page should scroll to, once.
    pub fn take_reveal(&mut self)->Option<Target>{self.reveal.take()}
    /// One request identity owns the inbox; replacing or dropping the PR cancels obsolete work.
    pub fn start_review(&mut self,root:&Path){
        #[cfg(any(test,feature="gui-test"))]
        if let Some(dir)=pr::test_support::state_dir(root){self.marks=ViewedMarks::open(&dir).ok();}
        self.review_token=Arc::new(());self.review_error=None;self.review_job=None;self.review=None;
        match self.pr.repo(){
            Ok(repo)=>{let root=root.to_path_buf();let number=self.pr.number;let current=Arc::downgrade(&self.review_token);self.review_job=Some(crate::repository_session::submit(&self.review_token,move||pr::Gh::new(&root).review_while(&repo,number,||current.strong_count()>0)));},
            Err(e)=>self.review_error=Some(e.to_string()),
        }
        self.generation+=1;
    }
    pub fn review_request(&self)->Weak<()>{Arc::downgrade(&self.review_token)}
    pub fn matches_review_request(&self,request:&Weak<()>)->bool{request.ptr_eq(&Arc::downgrade(&self.review_token))}
    pub fn review_loading(&self)->bool{self.review_job.is_some()}
    pub fn poll_review(&mut self,comparisons:&[Comparison])->bool{
        let Some(result)=crate::repository_session::receive(&mut self.review_job)else{return false;};
        match result {
            Ok(review)=>{if let Err(error)=self.seed_review(review,comparisons){self.review_error=Some(error);}},
            Err(error)=>{self.review_error=Some(error.to_string());self.generation+=1;},
        }
        true
    }
    /// PR metadata shown in either frontend's review header and description.
    pub fn summary(&self) -> String {
        let state = match (self.pr.state, self.pr.is_draft) {
            (_, true) => "draft",
            (pr::State::Open, _) => "open",
            (pr::State::Merged, _) => "merged",
            (pr::State::Closed, _) => "closed",
        };
        format!(
            "#{} {} · @{} · {} ← {} · {state} · +{} −{}",
            self.pr.number,
            self.pr.title,
            self.pr.author.login,
            self.pr.base_ref_name,
            self.pr.head_ref_name,
            self.pr.additions,
            self.pr.deletions
        )
    }
    pub fn seed_review(
        &mut self,
        review: Review,
        comparisons: &[Comparison],
    ) -> Result<(), String> {
        let result = if let Some(marks) = self.marks.as_mut() {
            let versions: std::collections::HashMap<_, _> = comparisons
                .iter()
                .filter_map(|comparison| Some((comparison.path.as_str(), blob(comparison)?)))
                .collect();
            marks.seed(&self.pr.url, &review.files, |path| {
                versions.get(path).cloned()
            })
        } else {
            Ok(())
        };
        self.review = Some(review);self.review_error=None;self.generation+=1;
        result
    }

    pub fn view_status(&self, comparison: &Comparison) -> Option<ViewStatus> {
        Some(
            self.marks
                .as_ref()?
                .status(&self.pr.url, &comparison.path, &blob(comparison)?),
        )
    }
    /// Toggle a personal mark. Returns the next unviewed index when requested.
    pub fn toggle_viewed(
        &mut self,
        comparisons: &[Comparison],
        index: usize,
        jump: bool,
    ) -> Result<Option<usize>, String> {
        let Some(comparison) = comparisons.get(index) else {
            return Ok(None);
        };
        let Some(version) = blob(comparison) else {
            return Ok(None);
        };
        let viewed = self.view_status(comparison) == Some(ViewStatus::Viewed);
        let Some(marks) = self.marks.as_mut() else {
            return Err("Viewed Marks storage is unavailable".into());
        };
        if viewed {
            marks.unmark(&self.pr.url, &comparison.path)?;
        } else {
            marks.mark(&self.pr.url, &comparison.path, &version)?;
        }
        Ok((jump && !viewed)
            .then(|| {
                (1..comparisons.len())
                    .map(|step| (index + step) % comparisons.len())
                    .find(|&next| self.view_status(&comparisons[next]) != Some(ViewStatus::Viewed))
            })
            .flatten())
    }
    pub fn viewed_count(&self, comparisons: &[Comparison]) -> (usize, usize) {
        (
            comparisons
                .iter()
                .filter(|comparison| self.view_status(comparison) == Some(ViewStatus::Viewed))
                .count(),
            comparisons.len(),
        )
    }
    pub fn file_threads(&self, path: &str, outdated: bool) -> Vec<Thread> {
        let Some(review) = self.review.as_ref() else {
            return Vec::new();
        };
        if outdated {
            return review.listed.get(path).cloned().unwrap_or_default();
        }
        let mut threads: Vec<_> = review
            .placed
            .iter()
            .filter(|((file, _, _), _)| file == path)
            .flat_map(|(_, threads)| threads.clone())
            .collect();
        threads.sort_by_key(|thread| thread.line);
        threads
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::Status;
    use crate::pr::{Account, ChangedFile, GithubViewed, State};
    fn pr() -> PullRequest {
        PullRequest {
            number: 7,
            title: "review".into(),
            author: Account {
                login: "ann".into(),
            },
            state: State::Open,
            is_draft: false,
            body: "Description".into(),
            base_ref_name: "main".into(),
            head_ref_name: "feature".into(),
            head_ref_oid: "head".into(),
            url: "https://github.com/example/repo/pull/7".into(),
            additions: 1,
            deletions: 0,
            changed_files: 2,
            is_cross_repository: false,
            head_repository: None,
            head_repository_owner: None,
        }
    }
    fn comparison(path: &str, version: &str) -> Comparison {
        Comparison {
            old_path: path.into(),
            path: path.into(),
            status: Status::Modified,
            left: Content::Blob("base".into()),
            right: Content::Blob(version.into()),
        }
    }
    fn state(dir: &Path) -> PrState {
        let mut state=PrState::new(pr(),Fetched{number:7,merge_base:"base".into(),head:"head".into()});
        state.marks=Some(ViewedMarks::open(dir).unwrap());state
    }

    #[test]
    fn personal_viewed_marks_seed_once_jump_and_detect_changed_versions() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state(dir.path());
        let comparisons = vec![comparison("a", "v1"), comparison("b", "v2")];
        let review = Review {
                discussion: Vec::new(),
            files: vec![ChangedFile {
                path: "a".into(),
                viewer_viewed_state: GithubViewed::Viewed,
            }],
            placed: Default::default(),
            listed: Default::default(),
            ..Default::default()
        };
        state.seed_review(review.clone(), &comparisons).unwrap();
        assert_eq!(state.view_status(&comparisons[0]), Some(ViewStatus::Viewed));
        assert_eq!(state.viewed_count(&comparisons), (1, 2));
        state.toggle_viewed(&comparisons, 0, false).unwrap();
        state.seed_review(review, &comparisons).unwrap();
        assert_eq!(
            state.view_status(&comparisons[0]),
            Some(ViewStatus::Unviewed)
        );
        assert_eq!(state.toggle_viewed(&comparisons, 0, true).unwrap(), Some(1));
        assert_eq!(
            state.view_status(&comparison("a", "new-push")),
            Some(ViewStatus::ChangedSinceViewed)
        );
        let reopened = state.marks.take().unwrap();
        assert_eq!(
            reopened.status(&state.pr.url, "a", "v1"),
            ViewStatus::Viewed
        );
        let deleted = Comparison {
            right: Content::Empty,
            ..comparison("a", "unused")
        };
        assert_eq!(blob(&deleted).as_deref(), Some("base"));
    }
    #[test]
    fn read_only_review_keeps_current_and_outdated_threads_separate() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state(dir.path());
        let thread: Thread = serde_json::from_str(r#"{"path":"a","diffSide":"RIGHT","line":4,"startLine":null,"originalLine":4,"isOutdated":false,"isResolved":true,"comments":{"nodes":[{"author":{"login":"ann"},"body":"Existing comment","createdAt":"2026-10-03"}]}}"#).unwrap();
        let mut outdated = thread.clone();
        outdated.is_outdated = true;
        outdated.line = None;
        state.review = Some(Review {
                discussion: Vec::new(),
            files: Vec::new(),
            placed: [(
                ("a".into(), crate::view::Side::Right, 4),
                vec![thread.clone()],
            )]
            .into(),
            listed: [("a".into(), vec![outdated.clone()])].into(),
            ..Default::default()
        });
        assert_eq!(state.file_threads("a", false), vec![thread]);
        assert_eq!(state.file_threads("a", true), vec![outdated]);
        assert!(state.file_threads("other", false).is_empty());
    }
    #[test]
    fn tabs_lay_out_pages_and_thread_markers_open_the_conversation_at_their_thread(){
        let dir=tempfile::tempdir().unwrap();let mut state=state(dir.path());
        state.pr.body="DESCRIPTION_END".into();
        state.execute(PrCommand::Show(DetailsTab::Conversation));
        let text=|state:&PrState|state.page(80,0).unwrap().rows.iter().map(|r|r.text()).collect::<Vec<_>>().join("\n");
        assert!(text(&state).contains("DESCRIPTION_END")&&text(&state).contains("Loading comments"));
        let before=state.generation();
        let mut thread=Thread{path:"a.rs".into(),diff_side:crate::view::Side::Right,line:Some(2),start_line:None,original_line:Some(2),is_outdated:false,is_resolved:true,
            comments:vec![pr::Comment{body:"THREAD_END".into(),created_at:"2026-10-05T00:00:00Z".into(),..Default::default()}],subject_type:String::new()};
        state.seed_review(Review{placed:[(("a.rs".into(),crate::view::Side::Right,2),vec![thread.clone()])].into(),..Default::default()},&[]).unwrap();
        assert!(state.generation()>before,"the review arriving changes the page");
        assert!(!text(&state).contains("THREAD_END"),"resolved: folded");
        state.execute(PrCommand::Close);assert!(state.details().is_none());
        state.execute(PrCommand::ShowThreads(vec![thread.clone()]));
        assert_eq!(state.details().map(|d|d.tab),Some(DetailsTab::Conversation));
        assert!(text(&state).contains("THREAD_END"),"its marker unfolds it");
        assert_eq!(state.take_reveal(),Some(Target::Line{path:"a.rs".into(),line:2,left:false}));
        assert_eq!(state.take_reveal(),None,"once");
        thread.is_resolved=false;
        state.execute(PrCommand::Toggle(pr_conversation::thread_key(&thread)));
        assert!(!text(&state).contains("THREAD_END"),"folded again");
        state.execute(PrCommand::Show(DetailsTab::Commits));
        assert!(text(&state).contains("Loading commits")||text(&state).contains("wants to merge"));
    }

}
