# 14: Protect dirty tab, window and all-view closure

**What to build:** Offer Save/Discard/Cancel before closing the last owner of a draft, including coordinated closure of independent sessions.

**Blocked by:** 13: Save drafts explicitly and preserve uncertain outcomes.

**Status:** resolved

- [x] Replay tab close, unsplit, native window close, terminal view close and all-view quit through actual runtime input/callback paths.
- [x] Prepare all sessions before removing or discarding any; Cancel, failed/uncertain Save or a new edit invalidates continuation. Previously confirmed explicit saves remain accounted for.
- [x] Active modals consume document/save input, tiny scrolled dialogs dismiss safely, and empty windows remain usable. Native application-termination wiring is completed by the separate native-platform ticket.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 31, 35, 36, 60, 117.


## Approved policy refinement — 2026-10-24

User chose Allow fresh Discard and Close after an uncertain Save, bypassing disk recheck ONLY for a fresh actual closure decision (tab/pane/window/all-view). Discard memory/history and leave current disk bytes untouched. Failedoldclose stayscancelled; current source/permission/consent fences stillapply. Save, externalOpenSaved, Reload/workspace/Git transitions retain their existing recheck/confirmationrules. Worker08 implements focusedcore+actualGUI/TUI regressionproof; finalcohortgate/performancepending.
