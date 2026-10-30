# 65: Show Git status separately from draft state

**What to build:** Identify modified/added/untracked/conflicted project files without confusing them with unsaved or recovered work.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Render explorer/tab status badges in both adapters, including a dirty draft over a Git-clean disk file.
- [x] Load and refresh committed-baseline identity lazily with cancellation and bounded resources.
- [x] Handle staged changes, HEAD changes, missing files and new/untracked sources while retaining drafts/history and non-color identification.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 104, 105, 106, 110.

## Answer

Merged in 84c1d09 (branch work/editor-65-git-status, 80f0ce8). Focused content/core/GUI/TUI filters + clippy clean. Performance deferred per user. Open: panicked status read leaves git_status_running stuck; no recovered-draft state yet. Follow-ups in docs/editor-follow-ups.md.
