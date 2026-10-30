# 15: Protect drafts across refresh and project or branch changes

**What to build:** Retain dirty work under automatic refresh and guard workspace-changing operations before they can replace text or start Git work.

**Blocked by:** 14: Protect dirty tab, window and all-view closure.

**Status:** resolved

- [x] Watcher/focus/index refresh, external deletion and current refresh replies retain drafts/history and expose conflict state; clean refresh preserves existing behavior.
- [x] Reload, Open Folder, Open Worktree Here, checkout and new-branch checkout use the shared decision flow before mutation.
- [x] Cancellation leaves project/working tree/drafts unchanged apart from explicitly confirmed earlier saves; failed/uncertain Save prevents continuation and stale choices cannot resume actions.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 32, 33.
