# 32: Recover unsaved drafts with explicit discard tombstones

**What to build:** Recover private unsaved work, including untitled documents, after interruption without writing originals.

**Blocked by:** 31: Create new documents and Save As safely.

**Status:** resolved

- [x] Persist recoverable versions privately with their confirmed baseline and validate recovery data before use.
- [x] Restore a recovered draft with clear labelling and disk-baseline comparison; malformed/stale records fail safely and do not expose source in logs.
- [x] Explicit Discard removes/tombstones recovery ownership so discarded work cannot return; exercise abrupt interruption and save/discard races.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 38, 40, 41, 42.

## Answer

Merged in de1d618 (branch work/editor-32-draft-recovery; feature 3c62f75/b067bcd, keymap-merge c6f3184; merged after ticket 29). Records live in `$XDG_CACHE_HOME/lenz-reader/recovery` (0700 dir, 0600 files, atomic rename, version + key + baseline validated on load, tombstones in `tombstones.json`); restore reports matches/differs/missing/unsaved, marks the tab `⟲` and closes Save until a changed file is rechecked; explicit Discard writes a later tombstone so the work cannot return. Both adapters replay real keys/clicks with modal precedence asserted under the keymap dispatch path.
Merge repairs: `Tab::place_selection` for the selection field ticket 29 made private; `CommandId::ALL` = 146 with both label sets; `Saver::discard` now releases the waiting version on the calling thread, because the recovery writer pinned an intermediate snapshot for the whole debounce and tripped `draft_history_retains_changes_without_pinning_every_intermediate_snapshot` (new regression test `recovery::tests::a_discard_releases_the_waiting_version_without_waiting_for_the_writer`). Core 617 passing 3/3 consecutive runs, GUI 189, TUI 162, clippy clean, panic-site ratchets at baseline. Limits: one record per path, 8 MiB per record, 20 records/32 MiB read at start, cache lifetime, recovery has no dedicated chord (File menu/F10 only) — see docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.
