# Fix branch inventory loading and reproduced workflow issues
Status: resolved
Type: task

Confirmed causes: branch inventory waits for changed-path scans of every worktree;
the list is discarded on every close. GUI/TUI render repeated inline group names.
TUI can lose editor focus after scratch creation/activation from search results;
an editable external/scratch source is mislabeled read-only. GUI Scratch Files
has no editable filter although the terminal picker does.

Validate request identity/cancellation, cached refresh and root changes; grouped
navigation/clicks/filtering and tiny layouts; actual scratch editing/Save and
filter selection; core, both adapters, full local CI and relevant equivalent
normal-release startup/input/memory/stress. Retain unverified platform limits.

## Answer

Implemented metadata-only cached inventory with request cancellation/stale-result
rejection, grouped GUI/TUI lists, target-preserving refresh and stale-hit guards.
Fixed narrow GUI result visibility, scratch focus, editable-source origin notices,
GUI scratch filtering and scratch-number exhaustion. Core/adapter regressions,
complete CI11 and owned native light/dark/narrow replays pass. Release branch
stress preserves every file/ref/HEAD and improves usable choices 20.635 → 1.407 ms.
The performance cost decision is tracked separately in ticket 04. Evidence and
limits: [audit report](../../../docs/research/code-branch-picker-audit.md).
