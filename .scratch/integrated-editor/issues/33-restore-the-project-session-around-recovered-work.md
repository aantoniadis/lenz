# 33: Restore the project session around recovered work

**What to build:** Reopen tabs, splits, positions and recovered drafts as a coherent session.

**Blocked by:** 32: Recover unsaved drafts with explicit discard tombstones.

**Status:** ready-for-agent

- [ ] Restore independent view state and shared buffer identity while keeping separately opened sessions independent.
- [ ] Identify recovered/new/missing/external-changed sources and preserve the recovery decision rather than silently replacing them from disk.
- [ ] Replay clean shutdown, interruption, cancelled recovery and narrow/empty layouts in both adapters.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 39, 40.
