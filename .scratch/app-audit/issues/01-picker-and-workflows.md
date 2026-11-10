# Fix branch inventory loading and reproduced workflow issues
Status: claimed
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
