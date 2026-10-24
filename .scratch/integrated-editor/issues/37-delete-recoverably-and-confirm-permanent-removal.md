# 37: Delete recoverably and confirm permanent removal

**What to build:** Delete project entries with recoverable deletion where available and an explicit permanent-delete alternative.

**Blocked by:** 36: Rename and move open files and folders; 32: Recover unsaved drafts with explicit discard tombstones.

**Status:** claimed

- [ ] Protect all affected dirty/shared buffers before deletion and reconcile explorer, watchers and provider state.
- [ ] Recoverable deletion can be restored where the platform supports it; permanent deletion requires an explicit action/confirmation.
- [ ] Deleted or explicitly discarded work cannot reappear from recovery; cancellation and partial failures preserve remaining data in both adapters.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 41, 111, 112, 113.
