# 36: Rename and move open files and folders

**What to build:** Move or rename project entries while retaining affected drafts and reading state.

**Blocked by:** 35: Create files and folders from the explorer.

**Status:** ready-for-agent

- [ ] Protect dirty files and reconcile every affected buffer/path, view, history anchor, watch and provider environment after confirmed movement.
- [ ] Detect collisions, concurrent moves and stale requests; failure leaves recoverable state and does not retarget another file.
- [ ] Both adapters handle file and directory moves without claiming semantic reference updates.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 111, 112.

## Comments

Exercise rename/reopen for both loaded and released cached buffers, plus extension changes and undo/redo after rename. Restoring historical text must retain the current buffer path/context rather than reviving old snapshot SourceMeta paths. Invalidate language/provider/project identities where appropriate while preserving buffer/history ownership.
