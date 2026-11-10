# Retain syntax feedback when returning to edited tabs
Status: resolved
Type: task

Reproduce an edited syntax error losing its underline or syntax colors after a tab
switch. Fix the shared lifecycle and cover settled and pending analysis, repeated
navigation, dirty buffers, stale results, Undo and Save without extra document copies.


## Answer

Fixed the remaining split-view restoration failure: reuse another loaded view's
exactly current syntax and semantic payloads after restoring its shared source.
No document copy or new cache. The base fails the split replay; the corrected
core and actual GUI/TUI input/render replay pass. Ordinary Java/Rust/Ruby dirty
tab round trips also pass with pending/settled parsing, repeat navigation, exact
version/disk guards and Undo. Light/dark colors and error underlines are checked
in each split, with narrow layouts. See the [validation report](../../../docs/research/editor-feedback-setup-search-validation.md).

Normal-release startup, edit/Undo/Redo/Save, RSS and sustained TUI checks retain
raw identities and samples. Native macOS/Windows/Wayland remain unverified.
