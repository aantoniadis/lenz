# 33: Restore the project session around recovered work

**What to build:** Reopen tabs, splits, positions and recovered drafts as a coherent session.

**Blocked by:** 32: Recover unsaved drafts with explicit discard tombstones.

**Status:** resolved

- [x] Restore independent view state and shared buffer identity while keeping separately opened sessions independent.
- [x] Identify recovered/new/missing/external-changed sources and preserve the recovery decision rather than silently replacing them from disk.
- [x] Replay clean shutdown, interruption, cancelled recovery and narrow/empty layouts in both adapters.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 39, 40.

## Answer

Merged in d0d8cf2 (branch work/editor-33-session-restore). The recovery dialog waits while a project's last tabs are restored, so Restore lands in them: every tab of the file (both panes of a split) shares the draft's buffer, is marked `⟲` and keeps its pane, selection and top line, clamped to the draft. Per the user's decision, a file whose recovered draft is undecided (dialog open or Later) is not editable — `A recovered draft of this file is undecided…` — and its record is never replaced or forgotten meanwhile (edit, Reload, Save or undo of the disk version). Disk state stays identified per offer (matches/differs/missing/unsaved, from ticket 32); workspace tabs whose files are gone are named in a message. The workspace is kept with the places after every tab or layout command, so an interruption reopens that arrangement; a New File made before the workspace arrives keeps its tab. Records carry an owner token held by an advisory lock under `recovery/owners/`, so another running window or terminal is not offered a live session's drafts; a crash releases the lock.
Evidence: core `session::tests::recovery` 16 (interruption replay with split and positions, clean shutdown with saved and discarded work, Later with blocked edits/Reload and a third restart, live-owner exclusion, missing tabs and an early New File) and `recovery::tests` 10 (owner liveness, ended-lock cleanup, invalid owner tokens); core filtered places/workspace/recovery/close/transition/save/bookmark/recent 172 passed. GUI `reader-gui` real-input replay (interrupted split project, dialog deferred, 1×1 and 150×60 windows, Esc then typing refused, File menu reopen, `r`, both panes restored) plus the two ticket 32 replays now releasing ownership as a crash would; workspace/places/split/restore filter 9 passed. TUI real-key replays for the same trace and an empty-layout New File restore at 1×1 and 20×5; recovery/workspace/places/split/restore with `gui-test` 19 passed. Clippy `-D warnings` on reader-core/gui/tui clean; panic-site ratchets at baseline. Performance deferred to the end-of-epic campaign; suspected costs and limits in docs/editor-follow-ups.md (ticket 33 section). Native macOS, Wayland and physical terminals unverified.
