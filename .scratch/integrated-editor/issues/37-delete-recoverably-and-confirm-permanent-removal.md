# 37: Delete recoverably and confirm permanent removal

**What to build:** Delete project entries with recoverable deletion where available and an explicit permanent-delete alternative.

**Blocked by:** 36: Rename and move open files and folders; 32: Recover unsaved drafts with explicit discard tombstones.

**Status:** resolved

- [x] Protect all affected dirty/shared buffers before deletion and reconcile explorer, watchers and provider state.
- [x] Recoverable deletion can be restored where the platform supports it; permanent deletion requires an explicit action/confirmation.
- [x] Deleted or explicitly discarded work cannot reappear from recovery; cancellation and partial failures preserve remaining data in both adapters.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 41, 111, 112, 113.

## Answer

Merged in f70e788 (branch work/editor-37-delete). Per the user's decision (2026-10-24) recoverable deletion is Lenz's own freedesktop.org Trash implementation (`crates/reader-core/src/trash.rs`, no new dependency): the home trash on the same device, else the mount's sticky `.Trash/$uid` or `.Trash-$uid`; a `.trashinfo` reserved with `create_new`, then the ticket 36 no-replace rename; restore renames back without replacing anything. Elsewhere (macOS, a file system without a usable trash) Delete removes nothing and says so. `CommandId::DeleteExplorerEntry`, `DeleteExplorerPermanently` and `RestoreDeleted` (ALL = 170) with `InterfaceAction::ConfirmDelete`; both adapters confirm through their own dialog showing the core's `delete_prompt` and `delete_refusal` (Enter confirms, Escape cancels, nothing else acts). An unsaved draft, a running or unconfirmed save or an undecided recovered draft inside the entry refuses both deletes; what is gone closes its clean tabs, drops their buffers and cached text, tombstones their recovery keys and leaves the inventory; a partial permanent failure reconciles what went and rescans what remains.
Evidence: core `trash::tests` 2 (record, name slots, percent-encoding, restore refused by a new entry, folder round trip, date shape) and `session::tests::delete` 3 (folder to trash with tab closure and restore; drafts refusing both deletes, permanent removal without a trash entry, stale request; discarded and deleted work not recovered at the next start and a blocked restore kept offered); core filtered delete/trash/relocate/create/recovery/places/project/keymap/menu/catalog/actions 184 passed. Parity replay `explorer_delete_refuses_drafts_trashes_restores_and_deletes_permanently_alike` (draft refusal with Enter and typing ignored, undo then trash, restore, permanent delete with Escape first; state, project and trash bytes compared in both adapters); GUI explorer/create/keymap/menu/commands/recover 36 and TUI explorer/create/keymap/menu/choices/commands/parity/recovery 61 passed. Clippy `-D warnings` clean (all targets, and core/content libraries without test-support); panic-site ratchets at baseline. Performance deferred to the end-of-epic campaign; limits (UTC deletion date, unverified mount-level trash, macOS) in docs/editor-follow-ups.md, user docs in docs/tui.md.
