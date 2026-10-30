# 39: Apply completion imports and snippet edits atomically

**What to build:** Accept completion with supported imports and placeholders as one coherent edit.

**Blocked by:** 38: Show automatic and manual completion safely; 30: Insert snippets and navigate placeholders.

**Status:** resolved

- [x] Handle provider replacement/insert ranges, resolved additional edits and snippet text against one captured document version.
- [x] Reject stale, malformed, overlapping or other-file edits as a whole transaction with an explanation, including server-initiated attempts.
- [x] One undo restores all related edits and imports; test compatible/incompatible multi-cursor contexts and real GUI/TUI acceptance.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 16, 67, 70.

## Answer

Merged in cf31abc (branch work/editor-39-completion-edits). A suggestion's replacement range, its `additionalTextEdits` and a snippet body apply as one edit of one file against the captured version: extra edits are converted against the version the popup was asked about, carried through name-continuation typing, and validated (malformed, overlapping or overlapping-the-name refuse the whole item with its reason; a typed edit touching them refuses). Servers with `resolveProvider` get `completionItem/resolve` on accept (the client now declares `resolveSupport` for `additionalTextEdits`); the answer applies only while the file is still the accepted version, else nothing is inserted and the message says why. Items carrying a server command stay refused (it could edit other files) and server-initiated `workspace/applyEdit` stays declined with a reworded notice. With several carets a plain suggestion applies at every caret on the same name text, imports once; other carets refuse; snippets still need one caret. Snippet placeholders shift by imports inserted above them.
Evidence: core semantics tests (fake server) `a_suggestion_and_its_import_are_one_undoable_transaction_of_the_current_version`, `several_carets_complete_together_only_on_the_same_name`, `a_snippet_with_an_import_keeps_its_placeholder_on_the_inserted_body`, `a_resolved_import_applies_to_the_accepted_version_only`, `an_edit_while_imports_are_resolved_makes_the_late_answer_insert_nothing`, and the refusal test extended to malformed imports and commands; semantics filter 17 passed (two Ruby LSP tests fail environmentally: the `ruby-lsp` gem is not installed in this container), core lib completion/snippet/semantics 66 passed. Parity replay `completion_typing_acceptance_and_dismissal_match_in_both_adapters` extended with an import-carrying acceptance and one Ctrl+Z in both adapters (TUI/parity completion 3 passed), GUI completion/snippet 2 passed. Clippy `-D warnings` clean (all targets and the core library without test-support); panic-site ratchets at baseline. Performance deferred; limits in docs/editor-follow-ups.md (ticket 39 section). Real providers, native macOS, Wayland and physical terminals unverified.
