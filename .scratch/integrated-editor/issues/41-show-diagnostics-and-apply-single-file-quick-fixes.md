# 41: Show diagnostics and apply single-file quick fixes

**What to build:** Inspect current-source diagnostics and apply a supported fix as one undoable change.

**Blocked by:** 38: Show automatic and manual completion safely.

**Status:** ready-for-agent

- [ ] Present severity, location and message in both adapters and navigate to the captured/current source safely.
- [ ] Request/apply quick fixes against the current version, reject wider workspace edits/server commands in full and explain unavailable actions.
- [ ] Stale diagnostics and fixes never overwrite drafts; real provider and GUI/TUI tests verify exact text/history.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 69, 70.
