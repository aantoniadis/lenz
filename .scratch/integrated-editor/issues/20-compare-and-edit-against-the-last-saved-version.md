# 20: Compare and edit against the last saved version

**What to build:** Open an in-window last-save comparison whose current side edits the ordinary tab’s buffer and undo history.

**Blocked by:** 13: Save drafts explicitly and preserve uncertain outcomes.

**Status:** resolved

- [x] The prior side is immutable, the current side resolves the same buffer, and Save updates only a confirmed baseline.
- [x] Edit and undo/redo from ordinary/split/comparison views, then return with positions and history intact; close the comparison without losing the draft.
- [x] Compute with the existing shared diff engine off-thread and reject stale baseline/current results; both adapters provide usable controls.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 2, 109.
