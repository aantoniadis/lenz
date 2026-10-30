# 35: Create files and folders from the explorer

**What to build:** Create a file or directory through the project explorer and open new text in the session.

**Blocked by:** 31: Create new documents and Save As safely.

**Status:** resolved

- [x] Use shared validation for names, collisions and unavailable/read-only directories before filesystem mutation.
- [x] A new file opens with the correct buffer/path identity and Save/undo/dirty behavior; watcher/explorer updates select the intended entry.
- [x] Test both adapters’ actual dialogs/actions and narrow, empty and cancelled states.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 5, 111, 112.

## Answer

Merged in 9955ce0 (branch work/editor-35-explorer-create, f3187d4, from 8bacc66; clean). `crates/code-core/src/create.rs` holds the pure name rules and `Session::create_refusal`/`create_entry` re-check them before one no-clobber `create_exact`/`create_dir`, so collisions, missing or foreign folders, read-only mode bits and a raced file all leave the disk untouched. A created file opens with its real path (not an untitled draft), stays clean until edited, saves exact bytes, undoes after Save as dirty, and is never offered as recovered work; `Project::add_file`/`add_folder` add it to the inventory and `State::tree_reveal()` selects the row in both adapters. `Command::CreateEntry` with `CommandId::NewExplorerFile`/`NewExplorerFolder` (ALL = 157, "No folder is open" reason) and a shared explorer row menu. Filtered core 9, GUI 1, TUI 1 + parity replay passing; clippy clean; panic-site ratchets ok. Limits: empty folders are session-local knowledge, read-only detection uses Unix mode bits only, reveal does not scroll — see "Explorer creation" in docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.
