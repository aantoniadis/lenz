# Project replacement and symbol rename
Status: claimed

Project text replacement uses a completed project search (including its file filter and case/word/regex options). Replacement text is literal, matching in-file Replace. Symbol rename asks the active language server for a workspace edit. Both prepare a bounded preview before making any edit, offer file selection, and apply to drafts with one undo step per affected file. Save uses existing guarded writes. Cancel changes nothing. Changes to the project, captured drafts or disk invalidate application; never apply a subset after a validation failure. Both GUI and TUI expose the same commands and preview.

First implementation accepts text edits to regular files within the project. Unsupported workspace operations and projection coordinates are refused as a whole with a reason. Limits and provider failures must be visible. Required checks cover stale/dirty sources, Unicode protocol positions, overlapping edits, cancel, selection, undo, encoding/line endings and actual input in both adapters.

Implementation is present. Final acceptance remains blocked by [validation on a healthy environment](issues/01-final-validation.md); this spec is not resolved until those checks pass.
