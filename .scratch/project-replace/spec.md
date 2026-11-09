# Project replacement and symbol rename
Status: resolved

Project text replacement uses a completed project search (including its file filter and case/word/regex options). Replacement text is literal, matching in-file Replace. Symbol rename asks the active language server for a workspace edit. Both prepare a bounded preview before making any edit, offer file selection, and apply to drafts with one undo step per affected file. Save uses existing guarded writes. Cancel changes nothing. Changes to the project, captured drafts or disk invalidate application; never apply a subset after a validation failure. Both GUI and TUI expose the same commands and preview.

First implementation accepts text edits to regular files within the project. Unsupported workspace operations and projection coordinates are refused as a whole with a reason. Limits and provider failures must be visible. Required checks cover stale/dirty sources, Unicode protocol positions, overlapping edits, cancel, selection, undo, encoding/line endings and actual input in both adapters.

## Answer

Implemented and validated on the combined PR #50 tree. The
[final validation ticket](issues/01-final-validation.md) is resolved; the
[validation report](../../docs/research/project-replace-rename-validation.md)
records the passing complete Linux gate, actual GUI/TUI workflows, production GUI
replays and normal-release/sustained measurements, including noisy TUI RSS and
unverified platforms. Inactive retained/evicted tabs now apply safely without
losing draft identity or earlier Undo history. Merging and releasing remain separate.
