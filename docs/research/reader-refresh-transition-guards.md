# Reader refresh and workspace transition guards

Ticket 15 preserves current drafts, undo/redo history and reading positions under watcher/focus/index
refresh. A changed disk version produces a shared status notice; deletion keeps the current source
and reports its unavailability. Reactivating a pinned draft retains that observed conflict. Clean
reloadable sources keep the existing refresh path.

Reload, Open Folder, Open Worktree Here, checkout and new-branch checkout reuse ticket 14's current
buffer/source/owner fences and Save/Discard/Cancel decisions. Checkout preparation also captures the
observed repository context. No load, project replacement or Git checkout starts before preparation
completes. Decisions are consumed once; edits, replacement sources, changed owners/context and
failed or uncertain Save cannot revive an old continuation. An earlier confirmed Save stays on disk.

Discard is consent until replacement succeeds. A failed Reload or blocked/failed checkout retains
the draft/history; retries require fresh preparation. A successful asynchronous reply may replace
only its captured current source. An intervening edit remains current even when the submitted Git
operation succeeded; the actual Git outcome is still reported. This does not promise filesystem
rollback or atomicity against external writers.

Temporary saved comparisons resolve back to ordinary owners before workspace preparation or Current
pane Reload. Last saved stays read-only. Opening the first folder from standalone mode retains the
existing ordinary tab ownership behavior; changing an existing project releases only approved orphan
sources.

Focused evidence is retained in `target/perf/editor-implementation/15-refresh/identity.json` and its
raw logs. Core traces cover Unicode/CRLF, exact draft/history/selection/file state, current refresh and
external deletion, clean undo history during index rebuilding, reload failure/stale replies, both folder
entry points, checkout/new-branch creation, blocked Git, changed HEAD, stale decisions, and rejected/
uncertain Save across all five explicit transitions. GUI/TUI traces use real focus/input/render,
folder controls, branch/new-branch controls, modal precedence, tiny sizes and comparison-to-ordinary
Reload. GUI folder selection uses the test platform's installed chooser callback, not native OS delivery.

Performance, the full batch gate/stress and native macOS remain unverified for this ticket until the
scheduled batch acceptance. Production Reader typing remains gated to later editing tickets; these
traces opt into the existing test support. No per-ticket performance tradeoff was requested or accepted.
