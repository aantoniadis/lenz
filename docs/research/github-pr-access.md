# GitHub PR access through `gh` and git (research)

Ticket: `.scratch/lenz-0.2/issues/03-github-pr-access-research.md`. Date: 2026-10-02.
Tested with gh 2.101.0, git 2.55.0, on Linux. The account was logged in (OAuth token, scopes `repo read:org gist workflow`). All GitHub calls were reads against the public `cli/cli` repo. Nothing was created, commented on or marked as viewed. The fetch experiments ran in a throwaway blobless clone under `/tmp`.

## Summary

- **Split of work:** `gh` supplies PR metadata, the picker, comments and the GitHub viewed state. Plain `git fetch` brings the commits into the user's own repo under a private ref namespace. Lenz computes the merge-base diff locally with the diff code it already has.
- **Base side:** never use `baseRefOid` (GraphQL) or `base.sha` (REST) as the diff base. Fetch the base *branch*, compute `git merge-base` locally and diff `merge-base..head`. On PR 14130 this matched GitHub's file list exactly.
- **Fork PRs:** `refs/pull/N/head` is served by the *base* repo, so forks need no extra remote. Fetch by URL rather than by remote name.
- **Viewed marks:** GitHub's per-file viewed state can be read (`viewerViewedState`). Setting it is a write (`markFileAsViewed` mutation). Lenz should read at most, and keep its own viewed marks locally.

## Findings

### 1. Resolving a PR (number, URL, branch, current branch)

`gh pr view [<number> | <url> | <branch>]` accepts all three forms. With no argument it finds the PR for the current branch (`gh pr view --help`). Observed:

- **URL:** `gh pr view https://github.com/cli/cli/pull/14130 --json number,headRefOid` works from any directory, including outside a git repo.
- **Number:** `gh pr view 14130 -R cli/cli --json …` takes about 0.6 s. Without `-R`, gh picks the base repo from the cwd's remotes. `gh repo view --json nameWithOwner,url,defaultBranchRef` shows which repo it picked.
- **Branch name:** `gh pr view williammartin-extension-browse-bubbletea -R cli/cli` resolves to PR 14578.
- **Current branch:** works when the branch's upstream config points at the PR head. After `gh pr checkout 14130`, a plain `gh pr view` resolved to PR 14130. On `trunk` it prints `no pull requests found for branch "trunk"` and exits 1.
- **Exit codes:** 0 ok, 1 failure, 2 cancelled, 4 auth required (`gh help exit-codes`).

The JSON fields Lenz needs from `gh pr view --json` are: `number, url, state, isDraft, title, baseRefName, baseRefOid, headRefName, headRefOid, headRepository, headRepositoryOwner, isCrossRepository, changedFiles, files`. `reviewThreads` is **not** among them, so comments need `gh api graphql` (see §5).

### 2. Base/head SHAs: `baseRefOid` is not the diff base

For PR 14130 (a fork PR from `iulia-b/cli` into `cli/cli`):

| value | SHA | what it is |
|---|---|---|
| `headRefOid` / REST `head.sha` | `45f237e` | PR head |
| `baseRefOid` / REST `base.sha` | `a526307` | a *recorded* trunk tip (11 commits past the merge base) |
| live `trunk` tip | `6fc1c29` | `gh api repos/cli/cli/branches/trunk` |
| merge base | `78de863` | `gh api repos/cli/cli/compare/trunk...<head>` → `merge_base_commit.sha`, and local `git merge-base` |

`baseRefOid` is neither the live base tip nor the merge base. GitHub's docs say "Pull requests on GitHub show a three-dot diff", comparing "the most recent common commit of both branches (merge base) and the most recent version of the topic branch" (*About comparing branches in pull requests*). So Lenz should:

1. fetch `refs/heads/<baseRefName>` and `refs/pull/N/head`;
2. compute `git merge-base <base> <head>`;
3. diff that merge base against head (the same as `git diff base...head`).

**Verified:** `git diff --name-status refs/lenz/pr/14130/base...refs/lenz/pr/14130/head` produced exactly the same 16 files and statuses as `gh api repos/cli/cli/pulls/14130/files`.

Lenz's existing Change Set is a pair of revisions, so the PR Change Set is `(merge-base SHA, head SHA)`. No new diff machinery is needed.

### 3. Fetching without disturbing the user's repo

The command that worked:

```sh
git -C <repo> fetch --no-tags --no-write-fetch-head --no-recurse-submodules \
    https://github.com/cli/cli.git \
    +refs/pull/14130/head:refs/lenz/pr/14130/head \
    +refs/heads/trunk:refs/lenz/pr/14130/base
# 1.5 s on a blobless clone of cli/cli
```

What it left behind:

- **Refs:** a `for-each-ref` snapshot before and after differed by exactly two added lines, `refs/lenz/pr/14130/{base,head}`. No branch, remote-tracking ref, HEAD, index or worktree changed.
- **FETCH_HEAD:** `--no-write-fetch-head` kept it from being written (the flag exists since git 2.29).
- **Reflog:** `refs/lenz/*` gets none. `core.logAllRefUpdates` only covers `refs/heads`, `refs/remotes`, `refs/notes` and HEAD, and `.git/logs/refs/lenz` did not exist after the fetch.
- **Cleanup:** `git update-ref -d refs/lenz/pr/N/head` (and `/base`), for example when the window closes.
- **Visibility caveat:** the refs show up in `git log --all`, `gitk --all` and similar while they exist.
- **Background gc:** `fetch` may start `gc --auto`. Pass `-c gc.auto=0` if that matters.

Other facts:

- **Forks:** `refs/pull/N/head` is published by the base repo, including for fork PRs. PR 14130's head lives in `iulia-b/cli` and was fetched from `cli/cli.git`. Lenz never has to add the fork as a remote.
- **Fetch by URL, not remote name:** this avoids guessing which local remote is the base repo (origin may be the user's fork). Credentials still apply because git credential helpers are keyed by host. On this machine, `gh auth setup-git` had installed `credential.https://github.com.helper=!/usr/bin/gh auth git-credential`, so private repos work too.
  - **SSH users:** https URLs only work for private repos when such a helper exists. Pick the URL to match the user's protocol. `gh config get git_protocol` returns `https` or `ssh`, and `gh repo view <o/r> --json url,sshUrl` gives both URLs (`https://github.com/cli/cli`, `git@github.com:cli/cli.git`). Use `sshUrl` when the protocol is `ssh`.
- **Fetch by SHA also works:** `git fetch <url> 45f237e…:refs/lenz/sha-test` succeeded. That is useful for re-fetching an exact `headRefOid`.
- **`refs/pull/N/merge` is unreliable:** it exists only while GitHub has a test-merge commit. `git ls-remote` showed it for open PR 14130 but not for old PR 1, whose `refs/pull/1/head` still exists. Do not build on it.
- **`gh pr checkout` is not acceptable:** it creates a local branch, rewrites `.git/config` (`branch.<name>.remote = https://github.com/iulia-b/cli.git`, `pushRemote`, `merge`) and switches the worktree. That is exactly what "don't disturb the user" rules out.
- **The `refs/pull/` namespace is read-only on GitHub:** pushing to it is rejected with `deny updating a hidden ref` (*Checking out pull requests locally*).
- **Ref-less alternative:** fetching with no destination and using the SHA keeps refs untouched entirely. But the objects are then unreferenced and can be pruned by gc, so the private ref is the safer default.

### 4. Listing open PRs for a picker

```sh
gh pr list [-R owner/repo] --state open --limit 50 \
  --json number,title,author,headRefName,baseRefName,isDraft,isCrossRepository,headRepositoryOwner,updatedAt
# 0.7 s. The default --limit is 30.
```

`gh pr list --search "<query>"` accepts GitHub search syntax (for example `review-requested:@me`). `gh pr status --json …` gives "created by me / needs my review" for the current repo.

### 5. Existing review comments and their positions

**REST:** `GET /repos/{o}/{r}/pulls/{n}/comments`, run as `gh api "repos/o/r/pulls/N/comments?per_page=100" --paginate`.

- **Comment fields:** `path`, `line`, `side` (LEFT/RIGHT), `start_line`, `start_side`, `original_line`, `original_start_line`, `commit_id`, `original_commit_id`, `diff_hunk`, `in_reply_to_id`, `subject_type` (line/file).
- **`position`** is a diff-hunk offset that GitHub describes as "closing down". Ignore it.
- **Outdated comments** *may* come back with `line: null` while `original_line` stays set. That is not reliable: comment 3658443294 is outdated but still had `line: 34`. The reliable REST signal is `commit_id != headRefOid`. On PR 13982, the outdated comments carried `21abdcf5`/`538b4682` and the current ones carried the head `a560bea9`.
- **Missing pieces:** REST has no resolved flag and no thread grouping beyond `in_reply_to_id`.

**GraphQL (recommended):** `pullRequest.reviewThreads` gives the thread grouping plus `isResolved`, `isOutdated`, `isCollapsed`, `path`, `diffSide`, `line`, `startLine`, `startDiffSide`, `originalLine`, `originalStartLine` and `subjectType` (LINE/FILE). Each comment adds `databaseId`, `author`, `body`, `createdAt`, `outdated`, `commit`, `originalCommit`.

Observed on PR 13982 (14 threads):

- **Current threads** have `line` relative to the current head on `diffSide`. They can be placed on Lenz's right/left Side directly, because RIGHT means the head file and LEFT means the merge-base file.
- **Outdated threads:** `isOutdated: true`, sometimes with `line: null`. Anchor them by `originalCommit` + `originalLine`. That commit may be gone after a force-push, so show them in a per-file "outdated" list rather than inline.
- **`startLine` differs between the APIs:** GraphQL returned `startLine == line` for single-line threads, where REST returns `start_line: null`.

One GraphQL call can fetch the SHAs, the files with their viewed state, and the threads (cost 1 point, 0.86 s). See the query below.

### 6. GitHub's per-file "viewed" state

- **Readable:** `PullRequest.files.nodes.viewerViewedState` returns the enum `FileViewedState`, which is one of:
  - `VIEWED`
  - `UNVIEWED`
  - `DISMISSED` ("The file has new changes since last viewed")

  Confirmed by schema introspection and by a live read (the sampled files of PR 14130 were all `UNVIEWED` for this user). It requires auth (GraphQL only, see §7).
- **Writable:** `markFileAsViewed(input: {pullRequestId: ID!, path: String!})` and `unmarkFileAsViewed` (introspection: "Mark a pull request file as viewed"). These are **mutations**. They change server-side state stored on github.com for the user, which the user then sees in the web UI. *Not called in this research.*
- **Verdict:** calling them is a write to GitHub, and the map lists "Writing to GitHub" as out of scope. Reading `viewerViewedState` is harmless and could seed Lenz's local marks. Syncing back would be a separate, later decision.
- **Local-mark keying:** key local viewed marks on `(repo, PR number, path, head-side blob OID)`. A new push that changes the file then clears the mark automatically, which gives GitHub's `DISMISSED` behaviour for free.

### 7. Rate limits, and behaviour without gh or auth

**Authenticated (the gh token):**

- REST: 5,000 requests/hour.
- GraphQL: 5,000 points/hour. The combined query costs 1 point.
- Secondary limits: at most 100 concurrent requests (REST and GraphQL together), 900 REST points/minute, 2,000 GraphQL points/minute.
- `gh api rate_limit` shows the remaining budget.
- Lenz's usage (a few calls per PR open or refresh) is far below all of these.

**Unauthenticated:**

- REST: 60 requests/hour per IP, public data only. Observed: `curl https://api.github.com/repos/cli/cli/pulls/14130` returned 200 with `x-ratelimit-limit: 60`.
- GraphQL: unusable. An anonymous POST returned 403 with `x-ratelimit-limit: 0`.

**`gh` without auth:** `GH_CONFIG_DIR=<empty> gh pr view …` prints "To get started with GitHub CLI, please run: gh auth login" and exits **4**.

**No gh at all:** the diff itself never needs gh or the API.

- For a public repo, `git fetch https://github.com/o/r.git refs/pull/N/head` works anonymously.
- For a private repo, it works with whatever git credentials the user already has.
- What is lost is metadata: the base branch name, the title, the picker, comments and viewed state.

**Degradation ladder:**

1. gh installed and authenticated: full feature.
2. No gh or no auth, PR given as URL or number on a public repo: unauthenticated REST `GET /repos/o/r/pulls/N` gives `base.ref`, `head.sha` and `title` (60/h). Then fetch with git. No picker, comments or viewed state.
3. Nothing works: tell the user to install `gh` and run `gh auth login`.

The simplest acceptable product choice is to require gh and show the exit-4 message.

**Size limits:**

- REST `pulls/N/files` returns at most 3,000 files.
- GraphQL `files` and `reviewThreads` page at 100. Use `gh api graphql --paginate`, which follows `$endCursor`.
- REST `pulls/N/commits` returns at most 250 commits.
- Lenz's local git diff has none of these limits, which is another reason to diff locally.

### 8. Timings observed (gh 2.101, warm)

| operation | time |
|---|---|
| `gh pr list … --json` (5 PRs) | 0.72 s |
| `gh pr view N --json …` | 0.62 s |
| `gh api repos/…/pulls/N` (REST) | 0.98 s |
| combined GraphQL (meta + files + viewed + threads) | 0.86 s |
| GraphQL threads only | 1.16 s |
| `git clone --filter=blob:none` cli/cli (not needed in practice) | 4.9 s |
| `git fetch` PR head + base into `refs/lenz/…` | 1.5 s |

## Exact commands and queries

Resolve a PR (any input form) and get metadata:

```sh
gh pr view <number|url|branch|""> [-R owner/repo] \
  --json number,url,title,state,isDraft,baseRefName,headRefName,headRefOid,isCrossRepository,headRepository
```

Base repo for the cwd:

```sh
gh repo view --json nameWithOwner,url,defaultBranchRef
```

Fetch into a private namespace:

```sh
git -c gc.auto=0 fetch --no-tags --no-write-fetch-head --no-recurse-submodules <base-repo-url> \
  +refs/pull/<N>/head:refs/lenz/pr/<N>/head +refs/heads/<baseRefName>:refs/lenz/pr/<N>/base
```

Change Set and cleanup:

```sh
git merge-base refs/lenz/pr/<N>/base refs/lenz/pr/<N>/head   # left revision
# right revision = refs/lenz/pr/<N>/head (check it equals headRefOid)
git update-ref -d refs/lenz/pr/<N>/head; git update-ref -d refs/lenz/pr/<N>/base   # cleanup
```

Picker:

```sh
gh pr list --state open --limit 50 --json number,title,author,headRefName,baseRefName,isDraft,updatedAt
```

One GraphQL round trip for everything else. Page `files` and `reviewThreads` with `after:` cursors when `hasNextPage` is true.

```sh
gh api graphql -F owner=O -F name=R -F number=N -f query='
query($owner:String!, $name:String!, $number:Int!) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      id number title url state isDraft isCrossRepository
      baseRefName baseRefOid headRefName headRefOid
      baseRepository { nameWithOwner url }
      headRepository { nameWithOwner url }
      files(first:100) { totalCount pageInfo { hasNextPage endCursor }
        nodes { path changeType viewerViewedState } }
      reviewThreads(first:100) { totalCount pageInfo { hasNextPage endCursor } nodes {
        isResolved isOutdated path diffSide line startLine originalLine originalStartLine subjectType
        comments(first:50) { nodes { databaseId author { login } body createdAt originalCommit { oid } } } } }
    }
  }
  rateLimit { cost remaining }
}'
```

## Recommendation

1. **Require `gh`.** Shell out to `gh pr view`/`gh pr list` for resolving and picking, and to `gh api graphql` (the query above) for SHAs, files, threads and viewed state. Treat exit 4 as "run `gh auth login`" and a missing binary as "install gh". Skip the unauthenticated-REST fallback for 0.2; it is a small add if it is ever wanted.
2. **Fetch with plain git by URL** (https or ssh, matching `gh config get git_protocol`) into `refs/lenz/pr/<N>/{head,base}`, using `--no-tags --no-write-fetch-head --no-recurse-submodules` and `-c gc.auto=0`. Delete the refs when the PR window closes. Never use `gh pr checkout`, `refs/pull/N/merge` or `baseRefOid`.
3. **Change Set** = `(git merge-base base head, head)`. This reproduces GitHub's three-dot "Files changed" view with Lenz's existing diff pipeline.
4. **Comments (if shown):** read-only, from GraphQL `reviewThreads`. Place current threads at `line` on the `diffSide` Side. List outdated threads separately per file.
5. **Viewed marks:** local only, keyed on head blob OID. Optionally seed them once from `viewerViewedState` (a read). Never call `markFileAsViewed`; it is a write to GitHub and out of scope per the map.

## Sources

- GitHub docs, *About comparing branches in pull requests* (three-dot vs two-dot): https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/proposing-changes-to-your-work-with-pull-requests/about-comparing-branches-in-pull-requests
- GitHub docs, *Checking out pull requests locally* (`pull/ID/head`, read-only `refs/pull/`): https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/reviewing-changes-in-pull-requests/checking-out-pull-requests-locally
- GitHub REST, *Pull request review comments* (field semantics, `position` closing down): https://docs.github.com/en/rest/pulls/comments
- GitHub REST, *Pulls* (3,000-file and 250-commit caps, `merge_commit_sha`): https://docs.github.com/en/rest/pulls/pulls
- GitHub REST, *Rate limits*: https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api
- GitHub GraphQL, *Rate limits and query limits*: https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api
- Live GraphQL schema introspection via `gh api graphql` of `MarkFileAsViewedInput`, `FileViewedState`, `Mutation.markFileAsViewed`/`unmarkFileAsViewed`, `PullRequestChangedFile.viewerViewedState` and `PullRequestReviewThread` field descriptions (2026-10-02).
- gh manual: `gh pr view --help` (JSON fields list), `gh pr list --help`, `gh help exit-codes`; https://cli.github.com/manual
- git: `git-fetch(1)` (`--no-write-fetch-head`, refspecs), `git-config(1)` (`core.logAllRefUpdates`).
