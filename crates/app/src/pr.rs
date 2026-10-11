//! Pull requests, read-only: `gh` for metadata, the picker, review threads and GitHub's viewed state; plain
//! `git fetch` into `refs/lenz/pr/N/*` for the commits (the one sanctioned write); Viewed Marks in a local file.
//!
//! Nothing here writes to GitHub: the only GraphQL sent is [`REVIEW_QUERY`], a query (see the guard test).


use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{changeset::Spec, git, view::Side};

/// Why a PR can't be opened; `Display` is the message for the user, with what to do about it.
#[derive(Debug, Clone, PartialEq)]
pub enum PrError {
    /// No `gh` on the PATH.
    GhMissing,
    /// `gh` exited 4, or `gh auth status` failed.
    GhLoggedOut,
    /// `gh pr view` without a PR argument, on a branch without a PR: show the picker instead.
    NoPullRequest,
    /// The cwd has no remote pointing at the PR's base repository (`owner/name`).
    NotInClone(String),
    /// Any other `gh` failure: its stderr.
    Gh(String),
    /// A `git` failure: its stderr.
    Git(String),
}

impl PrError {
    /// The user's to fix (where to run it, what to install or sign in to), not a failure worth a diagnostic report.
    pub fn is_user_error(&self) -> bool {
        matches!(self, PrError::GhMissing | PrError::GhLoggedOut | PrError::NoPullRequest | PrError::NotInClone(_))
    }

    /// What to do about it, for the diagnostic report and the issue draft.
    pub fn next_step(&self) -> &'static str {
        match self {
            PrError::GhMissing => "Install the GitHub CLI (gh), then retry.",
            PrError::GhLoggedOut => "Sign in with gh auth login, then retry.",
            PrError::NoPullRequest => "Name a pull request: lenz pr <number> or lenz pr <url>.",
            PrError::NotInClone(_) => "Run lenz pr from inside a clone of the pull request's repository.",
            PrError::Gh(_) => "Check gh auth status and network access, then retry.",
            PrError::Git(_) => "Check the repository's Git state and remotes, then retry.",
        }
    }
}

impl fmt::Display for PrError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PrError::GhMissing => write!(
                f,
                "lenz: pull requests need the GitHub CLI; install gh (https://cli.github.com), then run `gh auth login`"
            ),
            PrError::GhLoggedOut => write!(f, "lenz: gh is not logged in to GitHub; run `gh auth login`"),
            PrError::NoPullRequest => write!(f, "lenz: no pull request for the current branch"),
            PrError::NotInClone(repo) => write!(
                f,
                "lenz: this pull request belongs to {repo}; run `lenz pr` inside a clone of it (e.g. `gh repo clone {repo}`)"
            ),
            PrError::Gh(msg) => write!(f, "lenz: gh: {msg}"),
            PrError::Git(msg) => write!(f, "lenz: git: {msg}"),
        }
    }
}

/// A pull request as `gh pr view --json` describes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: Account,
    pub state: State,
    pub is_draft: bool,
    #[serde(default)]
    pub body: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub head_ref_oid: String,
    /// `https://<host>/<owner>/<name>/pull/<number>`; also the PR's key in [`ViewedMarks`].
    pub url: String,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub is_cross_repository: bool,
    /// Where the head branch lives (a fork for cross-repository PRs); `None` once that repository is deleted.
    pub head_repository: Option<RepoName>,
    pub head_repository_owner: Option<Account>,
}

const VIEW_FIELDS: &str = "number,title,author,state,isDraft,body,baseRefName,headRefName,headRefOid,url,\
                           additions,deletions,changedFiles,isCrossRepository,headRepository,headRepositoryOwner";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Account {
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RepoName {
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    Open,
    Closed,
    Merged,
}

/// A PR's base repository.
#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl PullRequest {
    /// The base repository, read from [`PullRequest::url`].
    pub fn repo(&self) -> Result<Repo, PrError> {
        let rest = self.url.split_once("://").map_or(self.url.as_str(), |(_, rest)| rest);
        match rest.split('/').collect::<Vec<_>>()[..] {
            [host, owner, name, "pull", ..] => {
                Ok(Repo { host: host.into(), owner: owner.into(), name: name.into() })
            }
            _ => Err(PrError::Gh(format!("unexpected pull request URL {:?}", self.url))),
        }
    }
}

/// One row of the open-PR picker.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedPr {
    pub number: u64,
    pub title: String,
    pub author: Account,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub is_draft: bool,
    /// ISO 8601, as GitHub sends it.
    pub updated_at: String,
}

/// The `gh` command: `gh` itself, run in the user's repository.
pub struct Gh {
    /// The program and any leading arguments (tests use `sh <fake gh script>`).
    cmd: Vec<OsString>,
    cwd: PathBuf,
}

impl Gh {
    pub fn new(cwd: &Path) -> Gh {
        #[cfg(any(test, feature="gui-test"))]
        if let Some(cmd) = test_support::command(cwd) { return Gh { cmd, cwd: cwd.to_path_buf() }; }
        Gh { cmd: vec!["gh".into()], cwd: cwd.to_path_buf() }
    }

    /// Runs gh and returns its stdout; exit 4 means not logged in.
    fn run(&self, args: &[&str]) -> Result<Vec<u8>, PrError> {
        let (program, prefix) = self.cmd.split_first().expect("a gh command");
        let out = Command::new(program)
            .args(prefix)
            .args(args)
            .current_dir(&self.cwd)
            .env("GH_PROMPT_DISABLED", "1")
            // gh's update check would hit the network and print a notice into the stderr Lenz shows as the error
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => PrError::GhMissing,
                _ => PrError::Gh(format!("cannot run gh: {e}")),
            })?;
        match out.status.code() {
            Some(0) => Ok(out.stdout),
            Some(4) => Err(PrError::GhLoggedOut),
            _ => Err(PrError::Gh(String::from_utf8_lossy(&out.stderr).trim_end().to_string())),
        }
    }

    fn json<T: DeserializeOwned>(&self, args: &[&str]) -> Result<T, PrError> {
        parse(&self.run(args)?)
    }

    /// Is gh installed and logged in? For deciding whether to offer PRs at all (the sidebar section).
    pub fn check(&self) -> Result<(), PrError> {
        match self.run(&["auth", "status"]) {
            Err(PrError::Gh(_)) => Err(PrError::GhLoggedOut), // `gh auth status` exits 1 when logged out
            other => other.map(drop),
        }
    }

    /// Resolves a PR from a number, URL or branch name; `None` is the current branch's PR.
    pub fn view(&self, pr: Option<&str>) -> Result<PullRequest, PrError> {
        let mut args = vec!["pr", "view"];
        args.extend(pr);
        args.extend(["--json", VIEW_FIELDS]);
        match self.json(&args) {
            Err(PrError::Gh(msg)) if pr.is_none() && msg.contains("no pull requests found") => {
                Err(PrError::NoPullRequest)
            }
            other => other,
        }
    }

    /// The first `limit` open PRs of the cwd's repository, most recently created first (gh's order).
    /// gh has no offset: "more on scroll" asks again with a larger `limit`.
    pub fn list_open(&self, limit: usize) -> Result<Vec<ListedPr>, PrError> {
        let limit = limit.to_string();
        self.json(&[
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            &limit,
            "--json",
            "number,title,author,headRefName,baseRefName,isDraft,updatedAt",
        ])
    }

    /// The URL to fetch `repo` from, in the user's protocol (`gh config get git_protocol`): ssh or https.
    pub fn fetch_url(&self, repo: &Repo) -> Result<String, PrError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Urls {
            url: String,
            ssh_url: String,
        }
        let protocol = self.run(&["config", "get", "git_protocol", "--host", &repo.host])?;
        let slug = format!("{}/{}/{}", repo.host, repo.owner, repo.name);
        let urls: Urls = self.json(&["repo", "view", &slug, "--json", "url,sshUrl"])?;
        Ok(if String::from_utf8_lossy(&protocol).trim() == "ssh" { urls.ssh_url } else { urls.url })
    }

    /// Has the PR moved past `fetched_head` (the head Lenz fetched)? One `gh pr view --json headRefOid`.
    pub fn head_moved(&self, url: &str, fetched_head: &str) -> Result<bool, PrError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Head {
            head_ref_oid: String,
        }
        let head: Head = self.json(&["pr", "view", url, "--json", "headRefOid"])?;
        Ok(head.head_ref_oid != fetched_head)
    }

    /// Read-only paginated PR discussion, reviews, viewed files and complete threads.
    pub fn review(&self, repo: &Repo, number: u64) -> Result<Review, PrError> {self.review_while(repo,number,||true)}
    pub(crate) fn review_while(&self,repo:&Repo,number:u64,current:impl Fn()->bool)->Result<Review,PrError>{
        let (mut files, mut threads, mut discussion) = (Vec::new(), Vec::new(), Vec::new());
        let mut overview = None;
        let mut cursors = [None::<String>, None, None, None];
        loop {
            if !current(){return Err(PrError::Gh("Obsolete PR request".into()));}
            let mut vars = vec![("owner",repo.owner.clone()),("name",repo.name.clone()),("number",number.to_string())];
            for (name,cursor) in ["filesAfter","threadsAfter","commentsAfter","reviewsAfter"].into_iter().zip(&cursors) {
                if let Some(cursor)=cursor { vars.push((name,cursor.clone())); }
            }
            let page = review_page(&self.graphql(repo, REVIEW_QUERY, &vars)?)?;
            if overview.is_none() { overview = Some(page.overview()); }
            let infos = [page.files.page_info, page.review_threads.page_info, page.comments.page_info, page.reviews.page_info];
            files.extend(page.files.nodes);
            for wire in page.review_threads.nodes {
                let mut thread=wire.thread;
                thread.comments=wire.comments.nodes;
                let mut info=wire.comments.page_info;
                while info.has_next_page {
                    if !current(){return Err(PrError::Gh("Obsolete PR request".into()));}
                    let cursor=info.end_cursor.as_ref().ok_or_else(||PrError::Gh("Missing thread reply cursor".into()))?;
                    if wire.id.is_empty(){return Err(PrError::Gh("Missing review thread identity".into()));}
                    let bytes=self.graphql(repo,THREAD_COMMENTS_QUERY,&[("id",wire.id.clone()),("after",cursor.clone())])?;
                    let value:serde_json::Value=parse(&bytes)?;
                    check_graphql(&value)?;
                    let replies:Page<Comment>=serde_json::from_value(value.pointer("/data/node/comments").cloned().ok_or_else(||PrError::Gh("Review thread unavailable".into()))?)
                        .map_err(|e|PrError::Gh(format!("unexpected thread reply: {e}")))?;
                    if replies.page_info.has_next_page && replies.page_info.end_cursor.as_ref()==Some(cursor){return Err(PrError::Gh("Thread reply pagination did not advance".into()));}
                    thread.comments.extend(replies.nodes);info=replies.page_info;
                }
                threads.push(thread);
            }
            discussion.extend(page.comments.nodes.into_iter().map(|comment|DiscussionComment{comment,..Default::default()}));
            discussion.extend(page.reviews.nodes.into_iter().filter(|r|r.state!="PENDING").map(|r|DiscussionComment{
                comment:Comment{author:r.author,body:r.body,created_at:r.submitted_at.unwrap_or_default(),url:r.url,..Default::default()},review_state:Some(r.state),review_id:Some(r.id)
            }));
            let mut more=false;
            for (cursor,info) in cursors.iter_mut().zip(infos) {
                if info.has_next_page {
                    more=true;
                    if info.end_cursor.is_none() || info.end_cursor==*cursor {return Err(PrError::Gh("PR pagination did not advance".into()));}
                }
                if info.end_cursor.is_some(){*cursor=info.end_cursor;}
            }
            if !more {break;}
        }
        discussion.sort_by(|a,b|a.comment.created_at.cmp(&b.comment.created_at));
        let mut review=Review::new(files, threads);review.discussion=discussion;review.overview=overview.unwrap_or_default();Ok(review)
    }
    fn graphql(&self, repo:&Repo,query:&str,vars:&[(&str,String)])->Result<Vec<u8>,PrError>{
        let values:Vec<_>=vars.iter().map(|(name,value)|format!("{name}={value}")).collect();
        let query=format!("query={query}");
        let mut args=vec!["api","graphql","--hostname",repo.host.as_str()];
        for ((name,_),value) in vars.iter().zip(&values){args.extend([if *name=="number"{"-F"}else{"-f"},value.as_str()]);}
        args.extend(["-f",query.as_str()]);
        self.run(&args)
    }

}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, PrError> {
    serde_json::from_slice(bytes).map_err(|e| PrError::Gh(format!("unexpected output: {e}")))
}

/// Files with the viewer's viewed state, and review threads. A query: Lenz reads GitHub's viewed state and
/// never sets it (that is a mutation, a write to GitHub, out of scope).
pub const REVIEW_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!, $filesAfter: String, $threadsAfter: String, $commentsAfter: String, $reviewsAfter: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      createdAt
      labels(first: 50) { nodes { name color } }
      reviewRequests(first: 50) { nodes { requestedReviewer { ... on User { login } ... on Team { name } ... on Mannequin { login } } } }
      latestReviews(first: 50) { nodes { author { login } state } }
      commits(first: 100) { totalCount nodes { commit { oid message authoredDate author { name user { login } } } } }
      checks: commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 50) { nodes { ... on CheckRun { name status conclusion } ... on StatusContext { context state } } } } } } }
      files(first: 100, after: $filesAfter) {
        pageInfo { hasNextPage endCursor }
        nodes { path viewerViewedState }
      }
      comments(first: 100, after: $commentsAfter) {
        pageInfo { hasNextPage endCursor }
        nodes { author { login } body createdAt url }
      }
      reviews(first: 100, after: $reviewsAfter) {
        pageInfo { hasNextPage endCursor }
        nodes { id author { login } body submittedAt state url }
      }
      reviewThreads(first: 100, after: $threadsAfter) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id isResolved isOutdated path diffSide line startLine originalLine subjectType
          comments(first: 100) { pageInfo { hasNextPage endCursor } nodes { author { login } body createdAt url diffHunk pullRequestReview { id } } }
        }
      }
    }
  }
}";
const THREAD_COMMENTS_QUERY:&str="query($id: ID!, $after: String!) { node(id: $id) { ... on PullRequestReviewThread { comments(first: 100, after: $after) { pageInfo { hasNextPage endCursor } nodes { author { login } body createdAt url diffHunk pullRequestReview { id } } } } } }";

#[derive(Deserialize)]
struct ThreadPage {
    #[serde(default)] id:String,
    comments:Page<Comment>,
    #[serde(flatten)] thread:Thread,
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
struct ReviewSummary {#[serde(default)] id:String,author:Option<Account>,body:String,submitted_at:Option<String>,state:String,#[serde(default)] url:String}
/// A PR comment, or a review (its state and id set).
#[derive(Debug,Clone,PartialEq,Default)]
pub struct DiscussionComment {pub comment:Comment,pub review_state:Option<String>,pub review_id:Option<String>}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewPage {
    #[serde(default)] created_at: String,
    #[serde(default, deserialize_with = "nodes")] labels: Vec<Label>,
    #[serde(default, deserialize_with = "nodes")] review_requests: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "nodes")] latest_reviews: Vec<ReviewSummaryState>,
    #[serde(default)] commits: Option<CommitPage>,
    #[serde(default)] checks: Option<serde_json::Value>,
    files: Page<ChangedFile>,
    review_threads: Page<ThreadPage>,
    comments:Page<Comment>,
    reviews:Page<ReviewSummary>,
}

#[derive(Deserialize)]
struct ReviewSummaryState {author:Option<Account>,state:String}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitPage {total_count:u64,nodes:Vec<serde_json::Value>}

impl ReviewPage {
    /// The side panel's and Commits tab's facts, from the loosely shaped parts of the reply.
    fn overview(&self) -> Overview {
        let text = |v: &serde_json::Value, p: &str| v.pointer(p).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let commits = self.commits.as_ref();
        let rollup = self.checks.as_ref().and_then(|c| c.pointer("/nodes/0/commit/statusCheckRollup")).filter(|r| !r.is_null());
        Overview {
            created_at: self.created_at.clone(),
            labels: self.labels.clone(),
            requested: self.review_requests.iter().map(|r| { let l = text(r, "/requestedReviewer/login"); if l.is_empty() { text(r, "/requestedReviewer/name") } else { l } })
                .filter(|l| !l.is_empty()).collect(),
            reviewers: self.latest_reviews.iter().map(|r| (r.author.as_ref().map_or("deleted".into(), |a| a.login.clone()), r.state.clone())).collect(),
            commits: commits.map_or_else(Vec::new, |c| c.nodes.iter().map(|n| PrCommit {
                sha: text(n, "/commit/oid"), message: text(n, "/commit/message"), date: text(n, "/commit/authoredDate"),
                author: { let login = text(n, "/commit/author/user/login"); if login.is_empty() { text(n, "/commit/author/name") } else { login } },
            }).collect()),
            commit_total: commits.map_or(0, |c| c.total_count),
            checks: rollup.and_then(|r| r.get("state")).and_then(|s| s.as_str()).map(str::to_string),
            check_runs: rollup.and_then(|r| r.pointer("/contexts/nodes")).and_then(|n| n.as_array()).map_or_else(Vec::new, |runs| runs.iter().map(|r| {
                let name = { let n = text(r, "/name"); if n.is_empty() { text(r, "/context") } else { n } };
                let outcome = [text(r, "/conclusion"), text(r, "/state"), text(r, "/status")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
                (name, outcome)
            }).collect()),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    page_info: PageInfo,
    nodes: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

fn review_page(bytes: &[u8]) -> Result<ReviewPage, PrError> {
    let mut value: serde_json::Value = parse(bytes)?;
    check_graphql(&value)?;
    match value.pointer_mut("/data/repository/pullRequest").map(serde_json::Value::take) {
        Some(pr) if !pr.is_null() => {
            serde_json::from_value(pr).map_err(|e| PrError::Gh(format!("unexpected GraphQL reply: {e}")))
        }
        _ => Err(PrError::Gh("pull request not found".into())),
    }
}

fn check_graphql(value:&serde_json::Value)->Result<(),PrError>{
    if let Some(errors)=value.get("errors").and_then(|v|v.as_array()).filter(|v|!v.is_empty()) {
        return Err(PrError::Gh(errors.iter().filter_map(|e|e.get("message").and_then(|v|v.as_str())).collect::<Vec<_>>().join("; ")));
    }Ok(())
}

/// `{ "nodes": [...] }` → the nodes.
fn nodes<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    #[derive(Deserialize)]
    struct Nodes<T> {
        nodes: Vec<T>,
    }
    Ok(Nodes::deserialize(d)?.nodes)
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub viewer_viewed_state: GithubViewed,
}

/// GitHub's own per-file viewed state for the logged-in user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GithubViewed {
    Viewed,
    Unviewed,
    /// Viewed, then changed by a later push.
    Dismissed,
}

/// GitHub's `diffSide`: LEFT is the merge-base file, RIGHT the head file — Lenz's Sides as they are.
fn diff_side<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Side, D::Error> {
    match String::deserialize(d)?.as_str() {
        "LEFT" => Ok(Side::Left),
        "RIGHT" => Ok(Side::Right),
        other => Err(serde::de::Error::custom(format!("unknown diffSide {other:?}"))),
    }
}

/// A review thread, read-only.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub path: String,
    #[serde(deserialize_with = "diff_side")]
    pub diff_side: Side,
    /// 1-based, in the `diff_side` file at the current head; the last line of a multi-line thread.
    /// `None` for outdated threads and threads on the whole file.
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    /// Where it was written, in a commit a force-push may have removed.
    pub original_line: Option<u32>,
    pub is_outdated: bool,
    pub is_resolved: bool,
    #[serde(default, deserialize_with = "nodes")]
    pub comments: Vec<Comment>,
    /// `LINE`, or `FILE` for a comment on the whole file.
    #[serde(default)]
    pub subject_type: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// `None` for a deleted account.
    pub author: Option<Account>,
    pub body: String,
    pub created_at: String,
    /// The comment on GitHub.
    #[serde(default)]
    pub url: String,
    /// A review comment's code context, as GitHub shows it above the comment.
    #[serde(default)]
    pub diff_hunk: String,
    /// The review a review comment belongs to.
    #[serde(default)]
    pub pull_request_review: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Id { pub id: String }

/// What the Conversation's side panel and Commits tab show: read with the review, first page only.
// ponytail: the first 100 commits and 50 labels, reviewers and checks; page them if a PR ever has more
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Overview {
    /// When the PR was opened (the description's time).
    pub created_at: String,
    pub labels: Vec<Label>,
    /// Requested reviewers who have not reviewed yet.
    pub requested: Vec<String>,
    /// Each reviewer's latest verdict: `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, `DISMISSED`.
    pub reviewers: Vec<(String, String)>,
    pub commits: Vec<PrCommit>,
    pub commit_total: u64,
    /// The head commit's check rollup: `SUCCESS`, `FAILURE`, `PENDING`, …; `None` without checks.
    pub checks: Option<String>,
    /// Each check's name and outcome (a conclusion, or the status while it runs).
    pub check_runs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Label { pub name: String, pub color: String }

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrCommit { pub sha: String, pub message: String, pub author: String, pub date: String }

/// What [`Gh::review`] read: viewed states, and the threads sorted for display.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Review {
    pub overview: Overview,
    pub files: Vec<ChangedFile>,
    pub discussion:Vec<DiscussionComment>,
    /// Current threads by (path, Side, 1-based line).
    pub placed: BTreeMap<(String, Side, u32), Vec<Thread>>,
    /// Per path: outdated threads and threads on the whole file, listed rather than placed.
    pub listed: BTreeMap<String, Vec<Thread>>,
}

impl Review {
    fn new(files: Vec<ChangedFile>, threads: Vec<Thread>) -> Review {
        let (mut placed, mut listed) = (BTreeMap::<_, Vec<_>>::new(), BTreeMap::<_, Vec<_>>::new());
        for thread in threads {
            match thread.line {
                Some(line) if !thread.is_outdated => {
                    placed.entry((thread.path.clone(), thread.diff_side, line)).or_default().push(thread)
                }
                _ => listed.entry(thread.path.clone()).or_default().push(thread),
            }
        }
        Review { files, placed, listed, ..Default::default() }
    }
}

/// Fails unless a remote of the repository at `cwd` points at `repo` (any protocol or host alias), so a
/// PR's objects never land in an unrelated repository.
pub fn ensure_clone(cwd: &Path, repo: &Repo) -> Result<(), PrError> {
    let want = format!("{}/{}", repo.owner, repo.name);
    let remotes = git::run(cwd, &["remote", "-v"]).unwrap_or_default(); // outside a repo: no remotes
    let found = String::from_utf8_lossy(&remotes)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .any(|url| remote_repo(url).is_some_and(|r| r.eq_ignore_ascii_case(&want)));
    if found { Ok(()) } else { Err(PrError::NotInClone(want)) }
}

/// `owner/name` of a remote URL: https, ssh://, scp-like `git@host:owner/name.git`.
/// The host is ignored, so ssh host aliases (`github-work:o/r`) match.
fn remote_repo(url: &str) -> Option<String> {
    let url = url.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let mut parts = url.rsplit(['/', ':']);
    let (name, owner) = (parts.next()?, parts.next()?);
    (!name.is_empty() && !owner.is_empty()).then(|| format!("{owner}/{name}"))
}

/// A PR fetched into `refs/lenz/pr/<number>/{head,base}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched {
    pub number: u64,
    /// The local `git merge-base` of base and head: GitHub's three-dot base (never `baseRefOid`).
    pub merge_base: String,
    pub head: String,
}

impl Fetched {
    /// The Change Set: merge-base ⟷ head, as GitHub's "Files changed" shows it.
    pub fn spec(&self) -> Spec {
        Spec::Git {
            cached: false,
            revs: vec![self.merge_base.clone(), self.head.clone()],
            paths: Vec::new(),
            untracked: false,
        }
    }

    /// The PR's commits, for the log.
    pub fn commit_range(&self) -> String {
        format!("{}..{}", self.merge_base, self.head)
    }
}

/// Fetches PR `number` and its base branch from `url` into private refs. Only those two refs change: no
/// branch, remote-tracking ref, FETCH_HEAD, tag or worktree. Call [`ensure_clone`] first.
pub fn fetch(cwd: &Path, url: &str, number: u64, base_branch: &str) -> Result<Fetched, PrError> {
    let (head_ref, base_ref) = (format!("refs/lenz/pr/{number}/head"), format!("refs/lenz/pr/{number}/base"));
    let refspecs =
        [format!("+refs/pull/{number}/head:{head_ref}"), format!("+refs/heads/{base_branch}:{base_ref}")];
    let mut args =
        vec!["-c", "gc.auto=0", "fetch", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", url];
    args.extend(refspecs.iter().map(String::as_str));
    git::run_network(cwd, &args).map_err(PrError::Git)?;
    let rev = |args: &[&str]| -> Result<String, PrError> {
        let out = git::run(cwd, args).map_err(PrError::Git)?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    };
    let head = rev(&["rev-parse", "--verify", &format!("{head_ref}^{{commit}}")])?;
    let merge_base = rev(&["merge-base", &base_ref, &head_ref]).map_err(|e| match e {
        PrError::Git(msg) if msg.is_empty() => {
            PrError::Git(format!("pull request #{number} shares no history with {base_branch}"))
        }
        e => e,
    })?;
    Ok(Fetched { number, merge_base, head })
}

/// Deletes every `refs/lenz/pr/*` ref, e.g. when Lenz closes. The objects stay, as after any fetch.
pub fn remove_refs(cwd: &Path) -> Result<(), PrError> {
    let refs = git::run(cwd, &["for-each-ref", "--format=%(refname)", "refs/lenz/pr/"]).map_err(PrError::Git)?;
    for name in String::from_utf8_lossy(&refs).lines() {
        git::run(cwd, &["update-ref", "-d", name]).map_err(PrError::Git)?;
    }
    Ok(())
}

/// Lenz's state directory: `$XDG_STATE_HOME/lenz` (else `~/.local/state/lenz`); macOS
/// `~/Library/Application Support/lenz`.
pub fn state_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/lenz"));
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local/state")))
        .map(|d| d.join("lenz"))
}

/// A file's Viewed Mark against its current head blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewStatus {
    Viewed,
    /// Marked at another blob: a push changed it since.
    ChangedSinceViewed,
    Unviewed,
}

/// Viewed Marks in `<dir>/viewed.json`: per PR URL (remote + number), path → the head blob id marked viewed.
/// Every change re-reads the file before writing it (temp file + rename), so two Lenz windows don't drop
/// each other's marks. ponytail: no lock (a same-millisecond race can lose one mark) and no pruning of old PRs.
pub struct ViewedMarks {
    file: PathBuf,
    prs: BTreeMap<String, PrMarks>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PrMarks {
    /// GitHub's viewed state was copied in once; never again, so local unmarks stick.
    #[serde(default)]
    seeded: bool,
    #[serde(default)]
    files: BTreeMap<String, String>,
}

impl ViewedMarks {
    /// Reads `<dir>/viewed.json` (missing = no marks); `dir` is [`state_dir`] outside tests.
    /// An unreadable file is an error, never silently overwritten.
    pub fn open(dir: &Path) -> Result<ViewedMarks, String> {
        let file = dir.join("viewed.json");
        let prs = load(&file)?;
        Ok(ViewedMarks { file, prs })
    }

    pub fn status(&self, pr_url: &str, path: &str, head_blob: &str) -> ViewStatus {
        match self.prs.get(pr_url).and_then(|pr| pr.files.get(path)) {
            Some(blob) if blob == head_blob => ViewStatus::Viewed,
            Some(_) => ViewStatus::ChangedSinceViewed,
            None => ViewStatus::Unviewed,
        }
    }

    #[cfg(test)]
    pub fn is_viewed(&self, pr_url: &str, path: &str, head_blob: &str) -> bool {
        self.status(pr_url, path, head_blob) == ViewStatus::Viewed
    }

    pub fn mark(&mut self, pr_url: &str, path: &str, head_blob: &str) -> Result<(), String> {
        self.update(pr_url, |pr| {
            pr.files.insert(path.to_string(), head_blob.to_string());
        })
    }

    pub fn unmark(&mut self, pr_url: &str, path: &str) -> Result<(), String> {
        self.update(pr_url, |pr| {
            pr.files.remove(path);
        })
    }

    /// The first time a PR is opened: files GitHub calls VIEWED are marked at their current head blob
    /// (`head_blob(path)`); DISMISSED and UNVIEWED add nothing. Later calls do nothing.
    pub fn seed(
        &mut self,
        pr_url: &str,
        files: &[ChangedFile],
        head_blob: impl Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        if self.prs.get(pr_url).is_some_and(|pr| pr.seeded) {
            return Ok(());
        }
        self.update(pr_url, |pr| {
            if pr.seeded {
                return; // another window got there first
            }
            pr.seeded = true;
            for file in files.iter().filter(|f| f.viewer_viewed_state == GithubViewed::Viewed) {
                if let Some(blob) = head_blob(&file.path) {
                    pr.files.insert(file.path.clone(), blob);
                }
            }
        })
    }

    fn update(&mut self, pr_url: &str, change: impl FnOnce(&mut PrMarks)) -> Result<(), String> {
        self.prs = load(&self.file)?;
        change(self.prs.entry(pr_url.to_string()).or_default());
        let fail = |e: io::Error| format!("lenz: cannot save Viewed Marks to {}: {e}", self.file.display());
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir).map_err(fail)?;
        }
        let tmp = self.file.with_extension(format!("json.{}.tmp", std::process::id()));
        let json = serde_json::to_vec_pretty(&self.prs).expect("marks serialize");
        fs::write(&tmp, json).and_then(|()| fs::rename(&tmp, &self.file)).map_err(fail)
    }
}

fn load(file: &Path) -> Result<BTreeMap<String, PrMarks>, String> {
    match fs::read(file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("lenz: {}: {e}", file.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(format!("lenz: {}: {e}", file.display())),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mistakes_the_user_fixes_are_not_reported_as_failures() {
        for e in [PrError::GhMissing, PrError::GhLoggedOut, PrError::NoPullRequest, PrError::NotInClone("o/r".into())] { assert!(e.is_user_error(), "{e:?}"); }
        for e in [PrError::Gh("boom".into()), PrError::Git("boom".into())] { assert!(!e.is_user_error(), "{e:?}"); }
    }

    #[test]
    fn a_pull_request_from_another_repository_says_where_to_run() {
        let e = PrError::NotInClone("octo-org/private-app".into());
        assert!(e.to_string().contains("inside a clone"));
        assert!(e.next_step().contains("inside a clone"), "{}", e.next_step());
        assert!(!e.next_step().contains("auth"));
    }

    use super::*;
    use crate::{
        changeset::ChangeSet,
        git::testing::{commit_all, git, repo, write},
    };

    const VIEW: &str = include_str!("../fixtures/pr/view.json");
    const LIST: &str = include_str!("../fixtures/pr/list.json");
    const REVIEW_1: &str = include_str!("../fixtures/pr/review-1.json");
    const REVIEW_2: &str = include_str!("../fixtures/pr/review-2.json");

    /// A `gh` that answers the first `case` glob matching `$*` with (output, exit code) — stdout on 0,
    /// stderr otherwise — and logs each call's arguments to `gh.log`.
    fn fake_gh(dir: &Path, cases: &[(&str, &str, i32)]) -> Gh {
        let mut script = format!("echo \"$*\" >> '{}'\ncase \"$*\" in\n", dir.join("gh.log").display());
        for (i, (pattern, out, code)) in cases.iter().enumerate() {
            let file = dir.join(format!("out{i}"));
            fs::write(&file, out).unwrap();
            let to = if *code == 0 { "" } else { " >&2" };
            script += &format!("  {pattern}) cat '{}'{to}; exit {code};;\n", file.display());
        }
        script += "esac\necho \"unexpected: gh $*\" >&2\nexit 1\n";
        let path = dir.join("gh.sh");
        fs::write(&path, script).unwrap();
        // via `sh`, not exec'd: no ETXTBSY from a just-written script while other tests fork
        Gh { cmd: vec!["sh".into(), path.into()], cwd: dir.to_path_buf() }
    }

    fn calls(dir: &Path) -> String {
        fs::read_to_string(dir.join("gh.log")).unwrap()
    }

    #[test]
    fn view_reads_a_pr_from_gh_json() {
        let dir = tempfile::tempdir().unwrap();
        let gh = fake_gh(dir.path(), &[("'pr view'*", VIEW, 0)]);
        let pr = gh.view(Some("14130")).unwrap();
        assert_eq!(calls(dir.path()), format!("pr view 14130 --json {VIEW_FIELDS}\n"));
        assert!(!VIEW_FIELDS.contains(char::is_whitespace));
        assert_eq!((pr.number, pr.title.as_str(), pr.author.login.as_str()), (14130, "Add JSON output to gh foo", "iulia-b"));
        assert_eq!((pr.state, pr.is_draft, pr.additions, pr.deletions, pr.changed_files), (State::Open, false, 412, 37, 16));
        assert_eq!((pr.base_ref_name.as_str(), pr.head_ref_name.as_str()), ("trunk", "feature/json-output"));
        assert_eq!(pr.head_ref_oid, "45f237e8d0c1b9a4e6f2a3c5d7e9f1b2c4d6e8f0");
        assert!(pr.is_cross_repository && pr.body.starts_with("Fixes #14000."));
        assert_eq!(pr.head_repository_owner.unwrap().login, "iulia-b");
        assert_eq!(pr.head_repository.unwrap().name, "cli");
        let repo = gh.view(Some("14130")).unwrap().repo().unwrap();
        assert_eq!(repo, Repo { host: "github.com".into(), owner: "cli".into(), name: "cli".into() });
    }

    #[test]
    fn gh_failures_become_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let gh = fake_gh(
            dir.path(),
            &[
                ("'pr view --json'*", "no pull requests found for branch \"trunk\"", 1),
                ("'pr view 9 '*", "To get started with GitHub CLI, please run:  gh auth login", 4),
                ("'pr view 404 '*", "GraphQL: Could not resolve to a PullRequest with the number of 404.", 1),
                ("'auth status'", "You are not logged into any GitHub hosts.", 1),
            ],
        );
        assert_eq!(gh.view(None), Err(PrError::NoPullRequest));
        assert_eq!(gh.view(Some("9")), Err(PrError::GhLoggedOut));
        assert_eq!(
            gh.view(Some("404")),
            Err(PrError::Gh("GraphQL: Could not resolve to a PullRequest with the number of 404.".into()))
        );
        assert_eq!(gh.check(), Err(PrError::GhLoggedOut));

        let missing = Gh { cmd: vec!["lenz-test-no-such-gh".into()], cwd: dir.path().into() };
        assert_eq!(missing.view(None), Err(PrError::GhMissing));
        assert!(PrError::GhMissing.to_string().contains("https://cli.github.com"));
        assert!(PrError::GhLoggedOut.to_string().contains("gh auth login"));
        assert!(PrError::NotInClone("cli/cli".into()).to_string().contains("inside a clone of it"));
    }

    #[test]
    fn list_open_asks_for_the_picker_fields() {
        let dir = tempfile::tempdir().unwrap();
        let gh = fake_gh(dir.path(), &[("'pr list'*", LIST, 0)]);
        let list = gh.list_open(60).unwrap();
        assert_eq!(
            calls(dir.path()),
            "pr list --state open --limit 60 --json number,title,author,headRefName,baseRefName,isDraft,updatedAt\n"
        );
        assert_eq!(list.len(), 2);
        assert_eq!((list[1].number, list[1].is_draft, list[1].author.login.as_str()), (14127, true, "app/dependabot"));
        assert_eq!(list[0].updated_at, "2026-09-30T14:02:11Z");
    }

    #[test]
    fn fetch_url_follows_the_git_protocol() {
        let urls = r#"{"sshUrl":"git@github.com:cli/cli.git","url":"https://github.com/cli/cli"}"#;
        let repo = Repo { host: "github.com".into(), owner: "cli".into(), name: "cli".into() };
        for (protocol, want) in [("ssh\n", "git@github.com:cli/cli.git"), ("https\n", "https://github.com/cli/cli")] {
            let dir = tempfile::tempdir().unwrap();
            let gh = fake_gh(dir.path(), &[("'config get git_protocol'*", protocol, 0), ("'repo view'*", urls, 0)]);
            assert_eq!(gh.fetch_url(&repo).unwrap(), want);
            assert_eq!(
                calls(dir.path()),
                "config get git_protocol --host github.com\nrepo view github.com/cli/cli --json url,sshUrl\n"
            );
        }
    }

    #[test]
    fn head_moved_compares_head_ref_oid() {
        let dir = tempfile::tempdir().unwrap();
        let gh = fake_gh(dir.path(), &[("'pr view'*", r#"{"headRefOid":"bbb"}"#, 0)]);
        let url = "https://github.com/cli/cli/pull/14130";
        assert!(!gh.head_moved(url, "bbb").unwrap());
        assert!(gh.head_moved(url, "aaa").unwrap());
        assert!(calls(dir.path()).starts_with(&format!("pr view {url} --json headRefOid\n")));
    }

    #[test]
    fn review_pages_and_places_threads() {
        let dir = tempfile::tempdir().unwrap();
        let gh = fake_gh(
            dir.path(),
            &[("*filesAfter=Y3Vyc29yOjI=*", REVIEW_2, 0), ("'api graphql'*", REVIEW_1, 0)],
        );
        let repo = Repo { host: "github.com".into(), owner: "cli".into(), name: "cli".into() };
        let review = gh.review(&repo, 14130).unwrap();

        let log = calls(dir.path());
        let requests: Vec<&str> = log.split("api graphql").skip(1).collect();
        assert_eq!(requests.len(), 2, "{log}");
        assert!(requests[0].starts_with(" --hostname github.com -f owner=cli -f name=cli -F number=14130 -f query=query("));
        assert!(!requests[0].contains("After="));
        // page 2: files continue; finished threads keep their cursor (GitHub returns nothing past it)
        assert!(requests[1].contains("-f filesAfter=Y3Vyc29yOjI= -f threadsAfter=Y3Vyc29yOnYyOpK0MjAyNi0wOS0zMA=="));

        let states: Vec<_> = review.files.iter().map(|f| (f.path.as_str(), f.viewer_viewed_state)).collect();
        assert_eq!(
            states,
            [
                ("pkg/cmd/foo/foo.go", GithubViewed::Viewed),
                ("pkg/cmd/foo/foo_test.go", GithubViewed::Dismissed),
                ("docs/README.md", GithubViewed::Unviewed)
            ]
        );

        let at = |side, line| review.placed.get(&("pkg/cmd/foo/foo.go".to_string(), side, line));
        let right = &at(Side::Right, 42).unwrap()[0];
        assert_eq!((right.start_line, right.is_resolved), (Some(40), false));
        let authors: Vec<_> = right.comments.iter().map(|c| c.author.as_ref().unwrap().login.as_str()).collect();
        assert_eq!(authors, ["williammartin", "iulia-b"]);
        let left = &at(Side::Left, 10).unwrap()[0];
        assert!(left.is_resolved && left.comments[0].author.is_none());
        assert_eq!(review.placed.len(), 2);

        let outdated = &review.listed["pkg/cmd/foo/foo.go"];
        assert_eq!((outdated.len(), outdated[0].is_outdated, outdated[0].original_line), (1, true, Some(17)));
        let on_file = &review.listed["docs/README.md"];
        assert_eq!((on_file.len(), on_file[0].is_outdated, on_file[0].line), (1, false, None));
    }

    #[test]
    fn review_reads_paginated_discussion_and_complete_thread_replies() {
        let dir = tempfile::tempdir().unwrap();
        let mut first: serde_json::Value = serde_json::from_str(REVIEW_1).unwrap();
        let pr = &mut first["data"]["repository"]["pullRequest"];
        pr["files"]["pageInfo"] = serde_json::json!({"hasNextPage":false,"endCursor":"files-end"});
        pr["comments"] = serde_json::json!({"pageInfo":{"hasNextPage":true,"endCursor":"comment-1"},"nodes":[{"author":{"login":"alice"},"body":"General discussion first","createdAt":"2026-10-01T10:00:00Z"}]});
        pr["reviews"] = serde_json::json!({"pageInfo":{"hasNextPage":true,"endCursor":"review-1"},"nodes":[{"author":null,"body":"Review summary first","submittedAt":"2026-10-01T11:00:00Z","state":"CHANGES_REQUESTED"}]});
        let thread = &mut pr["reviewThreads"]["nodes"][0];
        thread["id"] = "thread-long".into();
        thread["comments"]["pageInfo"] = serde_json::json!({"hasNextPage":true,"endCursor":"reply-100"});
        let second = serde_json::json!({"data":{"repository":{"pullRequest":{
            "files":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]},
            "reviewThreads":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]},
            "comments":{"pageInfo":{"hasNextPage":false,"endCursor":"comment-2"},"nodes":[{"author":null,"body":"General discussion last","createdAt":"2026-10-02T10:00:00Z"}]},
            "reviews":{"pageInfo":{"hasNextPage":false,"endCursor":"review-2"},"nodes":[{"author":{"login":"bob"},"body":"Approved summary last","submittedAt":"2026-10-02T11:00:00Z","state":"APPROVED"}]}
        }}}});
        let replies = serde_json::json!({"data":{"node":{"comments":{"pageInfo":{"hasNextPage":false,"endCursor":"reply-end"},"nodes":[{"author":null,"body":"Reply beyond first page","createdAt":"2026-10-02T12:00:00Z"}]}}}});
        let (first,second,replies)=(first.to_string(),second.to_string(),replies.to_string());
        let gh = fake_gh(dir.path(), &[("*id=thread-long*", &replies, 0),("*commentsAfter=comment-1*", &second, 0),("'api graphql'*", &first, 0)]);
        let repo = Repo { host:"github.com".into(),owner:"cli".into(),name:"cli".into() };
        let review = gh.review(&repo,9).unwrap();
        let thread = &review.placed[&("pkg/cmd/foo/foo.go".into(),Side::Right,42)][0];
        assert_eq!(thread.comments.last().unwrap().body,"Reply beyond first page");
        assert_eq!(review.discussion.iter().map(|entry|entry.comment.body.as_str()).collect::<Vec<_>>(),["General discussion first","Review summary first","General discussion last","Approved summary last"]);
        assert_eq!(review.discussion[1].review_state.as_deref(),Some("CHANGES_REQUESTED"));
        assert!(review.discussion[2].comment.author.is_none());
        assert!(calls(dir.path()).contains("reviewsAfter=review-1"));
    }

    #[test]
    fn review_rejects_partial_errors_stalled_pages_and_obsolete_requests(){
        let repo=Repo{host:"github.com".into(),owner:"o".into(),name:"r".into()};
        let dir=tempfile::tempdir().unwrap();
        let error=serde_json::json!({"data":{"repository":{"pullRequest":null}},"errors":[{"message":"Access denied"}]}).to_string();
        let gh=fake_gh(dir.path(), &[("*",&error,0)]);
        assert!(gh.review(&repo,9).unwrap_err().to_string().contains("Access denied"));
        let mut page:serde_json::Value=serde_json::from_str(REVIEW_1).unwrap();
        page["data"]["repository"]["pullRequest"]["files"]["pageInfo"]["endCursor"]=serde_json::Value::Null;
        let page=page.to_string();let gh=fake_gh(dir.path(), &[("*",&page,0)]);
        assert!(gh.review(&repo,9).unwrap_err().to_string().contains("did not advance"));
        let before=calls(dir.path());
        assert!(gh.review_while(&repo,9,||false).is_err());
        assert_eq!(calls(dir.path()),before,"obsolete work must not launch gh");
    }

    #[test]
    fn missing_pr_in_graphql_is_an_error() {
        let reply = br#"{"data":{"repository":{"pullRequest":null}}}"#;
        assert!(matches!(review_page(reply), Err(PrError::Gh(_))));
    }

    /// Guard: Lenz never writes to GitHub. The only GraphQL is a query, GitHub's per-file viewed state is
    /// only read (its mark-as-viewed mutation must never appear here), and no writing gh subcommand is used.
    #[test]
    fn never_writes_to_github() {
        assert!(REVIEW_QUERY.starts_with("query(") && !REVIEW_QUERY.contains("mutation"));
        let src = include_str!("pr.rs");
        assert!(!src.contains(concat!("markFile", "AsViewed")) && !src.contains(concat!("unmarkFile", "AsViewed")));
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(code.matches("\"api\"").count(), 1, "one `gh api` call: the review query");
        for write in ["checkout", "comment", "review", "merge", "edit", "close", "ready", "create"] {
            assert!(!code.contains(&format!("\"{write}\"")), "gh {write}");
        }
    }

    #[test]
    fn the_review_reads_the_overview_review_ids_and_code_context() {
        let page = serde_json::json!({"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]});
        let reply = serde_json::json!({"data":{"repository":{"pullRequest":{
            "createdAt":"2026-10-08T10:00:00Z",
            "labels":{"nodes":[{"name":"ui","color":"0366d6"}]},
            "reviewRequests":{"nodes":[{"requestedReviewer":{"login":"carol"}},{"requestedReviewer":{"name":"core-team"}}]},
            "latestReviews":{"nodes":[{"author":{"login":"bob"},"state":"APPROVED"}]},
            "commits":{"totalCount":1,"nodes":[{"commit":{"oid":"abc","message":"Fix it\n\nBody","authoredDate":"2026-10-08T09:00:00Z","author":{"name":"Mona","user":{"login":"octocat"}}}}]},
            "checks":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE","contexts":{"nodes":[{"name":"build","status":"COMPLETED","conclusion":"FAILURE"},{"context":"ci/legacy","state":"SUCCESS"}]}}}}]},
            "files":page,"comments":page,
            "reviews":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"id":"R1","author":{"login":"bob"},"body":"LGTM","submittedAt":"2026-10-09T00:00:00Z","state":"APPROVED","url":"https://x/r1"}]},
            "reviewThreads":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"id":"T1","isResolved":false,"isOutdated":false,"path":"a.rs","diffSide":"RIGHT","line":2,"startLine":null,"originalLine":2,
                "comments":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"author":{"login":"bob"},"body":"nit","createdAt":"2026-10-09T00:00:00Z","url":"https://x/c1","diffHunk":"@@ -1 +1,2 @@\n a\n+b","pullRequestReview":{"id":"R1"}}]}}]}
        }}}});
        let page = review_page(reply.to_string().as_bytes()).unwrap();
        let o = page.overview();
        assert_eq!(o.created_at, "2026-10-08T10:00:00Z");
        assert_eq!(o.labels, [Label { name: "ui".into(), color: "0366d6".into() }]);
        assert_eq!(o.requested, ["carol", "core-team"]);
        assert_eq!(o.reviewers, [("bob".to_string(), "APPROVED".to_string())]);
        assert_eq!((o.commit_total, o.commits[0].author.as_str(), o.commits[0].message.as_str()), (1, "octocat", "Fix it\n\nBody"));
        assert_eq!(o.checks.as_deref(), Some("FAILURE"));
        assert_eq!(o.check_runs, [("build".to_string(), "FAILURE".to_string()), ("ci/legacy".to_string(), "SUCCESS".to_string())]);
        let comment = &page.review_threads.nodes[0].comments.nodes[0];
        assert_eq!((comment.diff_hunk.as_str(), comment.pull_request_review.as_ref().map(|r| r.id.as_str())), ("@@ -1 +1,2 @@\n a\n+b", Some("R1")));
        assert_eq!((page.reviews.nodes[0].id.as_str(), page.reviews.nodes[0].url.as_str()), ("R1", "https://x/r1"));
    }

    #[test]
    fn remote_urls_name_their_repo() {
        for url in [
            "https://github.com/cli/cli",
            "https://github.com/cli/cli.git",
            "https://user@github.com/cli/cli/",
            "git@github.com:cli/cli.git",
            "ssh://git@github.com:22/cli/cli.git",
            "github-work:cli/cli",
        ] {
            assert_eq!(remote_repo(url).as_deref(), Some("cli/cli"), "{url}");
        }
        assert_eq!(remote_repo("cli"), None);
    }

    /// A bare "GitHub" with `main` and `refs/pull/7/head`, and the user's clone of it (main only).
    /// History: A ── M (main);  A ── F1 ── F2 (the PR).
    fn github_and_clone() -> (tempfile::TempDir, PathBuf, PathBuf, [String; 3]) {
        let root = tempfile::tempdir().unwrap();
        let (src, bare, user) = (root.path().join("src"), root.path().join("bare.git"), root.path().join("user"));
        fs::create_dir(&src).unwrap();
        git(&src, &["init", "-q", "-b", "main"]);
        write(&src, "a.txt", "one\n");
        commit_all(&src, "A");
        let a = git(&src, &["rev-parse", "HEAD"]).trim().to_string();
        git(&src, &["checkout", "-q", "-b", "feature"]);
        write(&src, "a.txt", "one\ntwo\n");
        commit_all(&src, "F1");
        write(&src, "f.txt", "feature\n");
        commit_all(&src, "F2");
        let f2 = git(&src, &["rev-parse", "HEAD"]).trim().to_string();
        git(&src, &["checkout", "-q", "main"]);
        write(&src, "m.txt", "main moved on\n");
        commit_all(&src, "M");
        let m = git(&src, &["rev-parse", "HEAD"]).trim().to_string();
        git(root.path(), &["clone", "-q", "--bare", "src", "bare.git"]);
        git(&bare, &["update-ref", "refs/pull/7/head", &f2]);
        git(&bare, &["branch", "-D", "feature"]);
        git(root.path(), &["clone", "-q", "--no-local", "bare.git", "user"]); // reachable objects only
        (root, bare, user, [a, m, f2])
    }

    #[test]
    fn fetch_makes_the_merge_base_change_set_and_cleans_up() {
        let (_root, bare, user, [a, _m, f2]) = github_and_clone();
        assert!(!git::is_commit(&user, &f2), "the clone doesn't have the PR yet");
        let refs = || git(&user, &["for-each-ref", "--format=%(refname) %(objectname)"]);
        let before = refs();

        let fetched = fetch(&user, bare.to_str().unwrap(), 7, "main").unwrap();
        assert_eq!(fetched, Fetched { number: 7, merge_base: a.clone(), head: f2.clone() });
        let added: Vec<_> = refs().lines().filter(|l| !before.contains(l)).map(str::to_string).collect();
        assert_eq!(added.len(), 2, "{added:?}");
        assert!(added.iter().all(|r| r.starts_with("refs/lenz/pr/7/")));
        assert!(!user.join(".git/FETCH_HEAD").exists());

        // three-dot: the PR's own changes, not main's m.txt
        let set = ChangeSet::load(&fetched.spec(), &user).unwrap();
        let paths: Vec<_> = set.comparisons.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "f.txt"]);
        assert_eq!(git(&user, &["rev-list", "--count", &fetched.commit_range()]).trim(), "2");

        remove_refs(&user).unwrap();
        assert_eq!(refs(), before);
    }

    #[test]
    fn a_pr_without_shared_history_is_an_error() {
        let (_root, bare, user, _) = github_and_clone();
        let other = repo();
        write(other.path(), "x", "x\n");
        commit_all(other.path(), "unrelated");
        git(other.path(), &["push", "-q", bare.to_str().unwrap(), "HEAD:refs/pull/8/head"]);
        let err = fetch(&user, bare.to_str().unwrap(), 8, "main").unwrap_err();
        assert_eq!(err, PrError::Git("pull request #8 shares no history with main".into()));
    }

    #[test]
    fn ensure_clone_wants_a_remote_at_the_base_repo() {
        let (_root, _bare, user, _) = github_and_clone();
        let repo = Repo { host: "github.com".into(), owner: "cli".into(), name: "cli".into() };
        assert_eq!(ensure_clone(&user, &repo), Err(PrError::NotInClone("cli/cli".into())));
        git(&user, &["remote", "add", "upstream", "git@github.com:CLI/cli.git"]);
        assert_eq!(ensure_clone(&user, &repo), Ok(()));
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(ensure_clone(outside.path(), &repo), Err(PrError::NotInClone("cli/cli".into())));
    }

    const PR: &str = "https://github.com/cli/cli/pull/14130";

    #[test]
    fn viewed_marks_key_on_pr_path_and_blob() {
        let dir = tempfile::tempdir().unwrap();
        let mut marks = ViewedMarks::open(dir.path()).unwrap();
        assert_eq!(marks.status(PR, "a.go", "b1"), ViewStatus::Unviewed);
        marks.mark(PR, "a.go", "b1").unwrap();
        assert!(marks.is_viewed(PR, "a.go", "b1"));
        // a push changed the file: the mark lapses
        assert_eq!(marks.status(PR, "a.go", "b2"), ViewStatus::ChangedSinceViewed);
        assert!(!marks.is_viewed(PR, "a.go", "b2"));
        // other paths, PRs and remotes are separate
        assert_eq!(marks.status(PR, "b.go", "b1"), ViewStatus::Unviewed);
        assert_eq!(marks.status("https://github.com/cli/cli/pull/1", "a.go", "b1"), ViewStatus::Unviewed);
        assert_eq!(marks.status("https://github.com/fork/cli/pull/14130", "a.go", "b1"), ViewStatus::Unviewed);
        marks.mark(PR, "a.go", "b2").unwrap();
        assert!(marks.is_viewed(PR, "a.go", "b2"));
        marks.unmark(PR, "a.go").unwrap();
        assert_eq!(marks.status(PR, "a.go", "b2"), ViewStatus::Unviewed);
    }

    #[test]
    fn viewed_marks_persist_and_merge_across_windows() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("lenz"); // created on first write
        let mut one = ViewedMarks::open(&state).unwrap();
        let mut two = ViewedMarks::open(&state).unwrap();
        one.mark(PR, "a.go", "b1").unwrap();
        two.mark(PR, "b.go", "b2").unwrap();
        let reopened = ViewedMarks::open(&state).unwrap();
        assert!(reopened.is_viewed(PR, "a.go", "b1") && reopened.is_viewed(PR, "b.go", "b2"));
        let leftovers: Vec<_> = fs::read_dir(&state).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers, ["viewed.json"]);
    }

    #[test]
    fn a_corrupt_marks_file_is_reported_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("viewed.json"), "{not json").unwrap();
        assert!(ViewedMarks::open(dir.path()).is_err());
        let mut marks = ViewedMarks { file: dir.path().join("viewed.json"), prs: BTreeMap::new() };
        assert!(marks.mark(PR, "a.go", "b1").is_err());
        assert_eq!(fs::read_to_string(dir.path().join("viewed.json")).unwrap(), "{not json");
    }

    #[test]
    fn github_viewed_state_seeds_marks_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut marks = ViewedMarks::open(dir.path()).unwrap();
        let file = |path: &str, state| ChangedFile { path: path.into(), viewer_viewed_state: state };
        let files = [
            file("viewed.go", GithubViewed::Viewed),
            file("dismissed.go", GithubViewed::Dismissed),
            file("unviewed.go", GithubViewed::Unviewed),
        ];
        let blob = |path: &str| Some(format!("blob-of-{path}"));
        marks.seed(PR, &files, blob).unwrap();
        assert!(marks.is_viewed(PR, "viewed.go", "blob-of-viewed.go"));
        assert_eq!(marks.status(PR, "dismissed.go", "blob-of-dismissed.go"), ViewStatus::Unviewed);
        assert_eq!(marks.status(PR, "unviewed.go", "blob-of-unviewed.go"), ViewStatus::Unviewed);

        // the user unmarks it; reopening the PR (even in a new process) doesn't re-seed
        marks.unmark(PR, "viewed.go").unwrap();
        let mut reopened = ViewedMarks::open(dir.path()).unwrap();
        reopened.seed(PR, &files, blob).unwrap();
        assert_eq!(reopened.status(PR, "viewed.go", "blob-of-viewed.go"), ViewStatus::Unviewed);
    }
}

/// Per-root offline providers used only by owned adapter fixtures.
#[cfg(any(test, feature="gui-test"))]
pub mod test_support {
    use super::*;
    use std::{collections::HashMap,sync::{Mutex,OnceLock}};
    fn providers()->&'static Mutex<HashMap<PathBuf,Vec<OsString>>> {
        static PROVIDERS:OnceLock<Mutex<HashMap<PathBuf,Vec<OsString>>>>=OnceLock::new();
        PROVIDERS.get_or_init(Default::default)
    }
    /// Keyed by the canonical root: sessions see git's toplevel, which resolves macOS's `/var` temp directories to `/private/var`.
    fn key(root:&Path)->PathBuf {root.canonicalize().unwrap_or_else(|_|root.to_path_buf())}
    pub(super) fn command(root:&Path)->Option<Vec<OsString>> {providers().lock().unwrap().get(&key(root)).cloned()}
    pub(crate) fn state_dir(root:&Path)->Option<PathBuf> {command(root).map(|_|root.join(".git/markdown-provider"))}
    pub struct Provider {root:PathBuf}
    impl Drop for Provider {fn drop(&mut self){providers().lock().unwrap().remove(&key(&self.root));}}
    /// Exact responses and request logs live under the disposable repository's .git.
    pub fn markdown_provider(root:&Path)->Provider {
        let run=|args:&[&str]|{git::run(root,args).unwrap()};
        let head=String::from_utf8(run(&["rev-parse","HEAD"])).unwrap().trim().to_string();
        run(&["branch","base","HEAD~1"]);run(&["update-ref","refs/pull/9/head",&head]);run(&["remote","add","origin","https://github.com/o/r.git"]);
        let dir=root.join(".git/markdown-provider");std::fs::create_dir(&dir).unwrap();
        let view=serde_json::json!({"number":9,"title":"Markdown PR","author":{"login":"tester"},"state":"OPEN","isDraft":false,"body":"# Literal PR description","baseRefName":"base","headRefName":"topic","headRefOid":head,"url":"https://github.com/o/r/pull/9","additions":3,"deletions":2,"changedFiles":3,"isCrossRepository":false,"headRepository":null,"headRepositoryOwner":null});
        let page=serde_json::json!({"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[]});
        let review=serde_json::json!({"data":{"repository":{"pullRequest":{"files":page,"comments":page,"reviews":page,"reviewThreads":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"id":"thread-9","isResolved":false,"isOutdated":false,"path":"ref.md","diffSide":"RIGHT","line":1,"startLine":null,"originalLine":1,"comments":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"author":{"login":"reviewer"},"body":"Source line thread","createdAt":"2026-10-05T00:00:00Z"}]}}]}}}}});
        for (name,value) in [("view.json",view),("review.json",review),("repo.json",serde_json::json!({"url":root.to_string_lossy(),"sshUrl":root.to_string_lossy()})),("head.json",serde_json::json!({"headRefOid":head}))] {let response=value.to_string();std::fs::write(dir.join(name),&response).unwrap();eprintln!("OFFLINE provider response {name}: {response}");}
        std::fs::write(dir.join("gh.sh"),r#"dir="$(dirname "$0")"
printf '%s\n' "$*" >> "$dir/requests.log"
case "$*" in
  'auth status') exit 0;;
  'config get git_protocol'*) echo https;;
  'repo view'*) cat "$dir/repo.json";;
  'pr view'*'--json headRefOid') cat "$dir/head.json";;
  'pr view'*) cat "$dir/view.json";;
  'pr list'*) echo '[{"number":9,"title":"Markdown PR","author":{"login":"tester"},"headRefName":"topic","baseRefName":"base","isDraft":false,"updatedAt":"2026-10-05T00:00:00Z"}]';;
  'api graphql'*) cat "$dir/review.json";;
  *) echo "unexpected offline provider call: $*" >&2; exit 1;;
esac
"#).unwrap();
        assert!(providers().lock().unwrap().insert(key(root),vec!["sh".into(),dir.join("gh.sh").into()]).is_none());
        Provider{root:root.to_path_buf()}
    }
    #[cfg(test)]
    #[test]
    fn provider_registry_isolates_roots_and_removes_override_on_drop() {
        let first=tempfile::tempdir().unwrap();let second=tempfile::tempdir().unwrap();
        providers().lock().unwrap().insert(key(first.path()),vec!["first".into()]);
        providers().lock().unwrap().insert(key(second.path()),vec!["second".into()]);
        let first_guard=Provider{root:first.path().into()};let second_guard=Provider{root:second.path().into()};
        assert_eq!(command(first.path()).unwrap(),vec![OsString::from("first")]);drop(first_guard);
        assert!(command(first.path()).is_none());assert_eq!(command(second.path()).unwrap(),vec![OsString::from("second")]);drop(second_guard);assert!(command(second.path()).is_none());
    }
}
