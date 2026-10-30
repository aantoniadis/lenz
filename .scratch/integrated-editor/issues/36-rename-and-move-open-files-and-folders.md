# 36: Rename and move open files and folders

**What to build:** Move or rename project entries while retaining affected drafts and reading state.

**Blocked by:** 35: Create files and folders from the explorer.

**Status:** resolved

- [x] Protect dirty files and reconcile every affected buffer/path, view, history anchor, watch and provider environment after confirmed movement.
- [x] Detect collisions, concurrent moves and stale requests; failure leaves recoverable state and does not retarget another file.
- [x] Both adapters handle file and directory moves without claiming semantic reference updates.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 111, 112.

## Comments

Exercise rename/reopen for both loaded and released cached buffers, plus extension changes and undo/redo after rename. Restoring historical text must retain the current buffer path/context rather than reviving old snapshot SourceMeta paths. Invalidate language/provider/project identities where appropriate while preserving buffer/history ownership.

## Answer

Merged in 83494b3 (branch work/editor-36-rename-move). `crates/code-core/src/relocate.rs` holds the pure destination rules (one-segment names, project-relative folders, no move into itself); `Session::relocate_refusal` adds the disk and document checks both prompts show (missing source, taken destination, missing folder, a running or unconfirmed save inside the moved path, an undecided recovered draft) and `relocate_entry` re-checks them before one `lenz_content::write::rename_noclobber` (Linux `renameat2(RENAME_NOREPLACE)`, macOS `renamex_np(RENAME_EXCL)`; a new audited FFI site listed in the content crate's `lib.rs`), so a collision or a racing entry, an empty folder included, is never replaced. After a confirmed move every affected tab (shown, released, comparison side) follows through the watcher's extracted `follow_move` with a fresh refresh token, the confirmed save target follows only when device and inode still match, a new extension redetects the language, and history, bookmarks/recent places, the project inventory/expanded and empty folders, documents' language-server sync and watches follow. A dirty draft moves with its file and is never written; references in other files are not changed. `CommandId::RenameExplorerEntry`/`MoveExplorerEntry` (ALL = 167), `InterfaceAction::Relocate`, the explorer row menu and F10 → View; no default chord.
Evidence: content `a_no_clobber_rename_moves_files_and_folders_but_never_replaces`; core `session::tests::relocate` 4 (dirty rename with extension change, undo/redo keeping the new path and Save to it; folder move carrying shown and released tabs; taken, stale, impossible and recovery-blocked requests leaving disk and tabs unchanged; disabled reasons), `relocate::tests` 3, `a_moved_folder_moves_every_history_entry_inside_it`; core filtered relocate/create/rename/followed/moved/places/history/keymap/menu/catalog/actions/project 187 passed. GUI `gui_the_explorer_renames_and_moves_an_open_dirty_file_through_real_input` (row menu, F10, prefilled dialog, refusal in the dialog, 1×1/200×80 windows, Save at the new path) and explorer/create/keymap/menu/commands 33 passed; TUI `explorer_rename_and_move_use_the_row_menu_and_survive_tiny_terminals` and the parity replay `explorer_rename_and_move_carry_the_draft_identically_in_both_adapters` (rename with a refused collision, move into a folder, folder rename with Escape first, Save; state and bytes compared in both), explorer/create/keymap/menu/choices/commands/parity 55 passed. Clippy `-D warnings` on lenz-content, code-core, code-gui, code-tui (all targets, and the core/content libraries without test-support) clean; panic-site ratchets at baseline. Performance deferred to the end-of-epic campaign; costs and limits in docs/editor-follow-ups.md (ticket 36 section), user docs in docs/tui.md. Native macOS (the `renamex_np` route compiles but did not run), Wayland, physical terminals, case-only renames and cross-device moves unverified.
