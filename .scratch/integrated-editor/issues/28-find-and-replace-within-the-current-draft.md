# 28: Find and replace within the current draft

**What to build:** Search and replace text locally with results that track the unsaved document.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Expose replace-current and replace-in-file with current options and clear invalid-pattern handling in both adapters.
- [x] Validate the captured version, make each explicit replace action undoable, and keep selections/find results coherent after changes.
- [x] Do not search/replace other files; reject stale matches and preserve read-only/modal behavior.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 15, 48.

## Answer

Merged in 8bacc66 (branch work/editor-28-find-replace, 4ea6053, from e145ca8; clean). Replace is built on the existing find model: `Command::SetReplaceText` / `Command::ReplaceFind { all }`, `CommandId::ReplaceMatch`/`ReplaceAll` (ALL = 155, Find group, disabled reasons). `Session::replace_find` applies one `EditKind::Replace` transaction against the captured `find.version` and refuses stale matches, pending searches ("Wait for the search to finish"), invalid patterns, empty lists, read-only/preview views and non-focused drafts. GUI replace row (Tab/Shift-Tab/click/Enter, Alt+A) and TUI second prompt field; Replace All carries existing carets through the one transaction while replace-current collapses to the next match. Core 801, lenz-gpui 7, GUI 190, TUI 137/167, cross-adapter replay passing; clippy clean; panic-site ratchets ok. Limits: no preset chord, literal replacement, no line-crossing matches, 10 000-match cap — see docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.
