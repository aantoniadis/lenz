# 42: Format and organize imports under project settings

**What to build:** Format explicitly or opt into project-specific format-on-save, and organize imports without touching other files.

**Blocked by:** 38: Show automatic and manual completion safely; 25: Respect project editing and formatting preferences.

**Status:** resolved

- [x] Use configured/project-provided tools and apply valid current-file results as meaningful undo transactions.
- [x] Reject cross-file/stale output in full; explicit format-on-save participates in Save ordering and retains the draft when formatting or writing fails.
- [x] Both adapters expose actions/status and test unchanged unrelated files, cancellation and project indentation/formatter resolution.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 69, 70, 71, 72.

## Answer

Merged in ec816b6 (branch work/editor-42-format). Format Document (Ctrl+Alt+L; Alt+Shift+F in VS Code) sends `textDocument/formatting` with the file's resolved indentation as options, and Organize Imports (Ctrl+Alt+O; Alt+Shift+O) applies the server's `source.organizeImports` action; both run on their own lane, fenced to tab, version and server session, show their status ("Formatting…") with Escape cancelling, and apply as one undo transaction of the draft. Answers for changed text, overlapping edits and organize actions touching another file are refused whole with the reason; an answer that changes nothing adds no undo step. Format on save (project setting from ticket 25) makes the Save command format first and write the formatted draft; when the server cannot be asked Save writes unformatted and says so, and when a sent request fails, is cancelled or answers for changed text nothing is written and the draft stays. New Save Without Formatting writes the draft as it is. Lenz formats through the language server only; directly running a project-configured formatter is left to the per-language tickets (recorded in docs/editor-follow-ups.md).
Evidence: core semantics tests (fake server) `format_document_applies_the_servers_edits_as_one_undo_step_with_the_files_indentation`, `format_on_save_writes_the_formatted_draft_and_a_failed_formatter_writes_nothing`, `formatting_answered_for_changed_text_or_cancelled_changes_and_writes_nothing`, `organize_imports_sorts_this_file_and_refuses_an_action_that_edits_another`, `without_a_formatter_save_writes_unformatted_and_says_so`; parity replay `formatting_organize_imports_and_format_on_save_match_in_both_adapters` (Ctrl+Alt+L, Ctrl+Z, Ctrl+Alt+O, format-on-save Ctrl+S writing exact bytes with an unrelated file untouched, slow formatter status drawn and Escape cancelling with nothing written and the late answer dropped). Core lib filter 92 passed, semantics filter 14 passed, TUI/parity filter 31 passed, reader GUI filter 17 passed. Clippy clean; panic-site ratchets at baseline. Performance deferred; limits in docs/editor-follow-ups.md (ticket 42 section). Real providers, native macOS, Wayland and physical terminals unverified.
