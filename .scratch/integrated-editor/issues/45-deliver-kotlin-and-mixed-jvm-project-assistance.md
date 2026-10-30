# 45: Deliver Kotlin and mixed JVM project assistance

**What to build:** Edit Kotlin with project/dependency context and verified mixed Java/Kotlin behavior.

**Blocked by:** 44: Deliver Java project-aware editing assistance.

**Status:** resolved

- [x] Exercise Kotlin completion/imports, parameter help, diagnostics and formatting in actual supported project imports.
- [x] Verify both directions of cross-language navigation/assistance and report concrete provider limits; do not merge unrelated providers into an answer.
- [x] Both adapters expose the same supported actions, safe current-file transactions and environment/readiness state.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 75, 76.

## Answer

Merged in f84f381 (branch work/editor-45-kotlin). Verified with real Kotlin LSP 263.6379.0-EAP (installed for this ticket from JetBrains' archive, SHA-256 checked) on the mixed Maven fixture with Kotlin 2.2.21. Kotlin LSP applies completion, fixes and organize imports only through its own commands, so with the user's decision (2026-10-25, allowlisted commands) Lenz now runs exactly `jetbrains.kotlin.completion.apply`, `applyModCommand` and `kotlin.organize.imports` for that server when the user picks the item, and applies the edit it sends back only when it changes the open file alone at the version the command was sent for, as one Undo step (otherwise refused whole and answered `applied: false`); other commands stay refused. Kotlin LSP also stops publishing problems after the first versions, so Lenz now asks `textDocument/diagnostic` for each version sent to servers that declare `diagnosticProvider`. Verified: the LocalDate import fix, completion of `Random`, Kotlin completing the project's Java method (`twice`), parameter information, Format Document, Organize Imports, problems resolving for the shown version, and a fix whose edit returned after typing refused. Cross-language: Kotlin to Java definitions and usages including Java callers, and Java to Kotlin not followed (JDT LS does not index Kotlin; said, never combined), re-verified with `real_kotlin_with_jdtls`.
Evidence: ignored real tests `real_kotlin_editing` (36.6 s), `real_kotlin` and `real_kotlin_with_jdtls` pass here; fake-server core tests `an_allowlisted_completion_command_inserts_the_name_its_server_sends_back_as_one_undo_step`, `an_edit_a_command_sends_back_for_another_file_is_refused_whole`, `allowlisted_fix_and_organize_commands_apply_and_other_commands_stay_refused`, `a_command_edit_for_text_changed_meanwhile_is_refused`, `problems_are_asked_of_a_server_that_stops_publishing_them`; parity replay `server_command_fixes_and_completion_apply_alike_in_both_adapters` (Alt+Enter, Enter, Ctrl+Z, Ctrl+Space, Down, Enter in both). Core lib filter 53 passed; semantics filter passing except three environmental Ruby LSP tests; TUI 7 and reader GUI 3 passed; clippy clean; panic-site ratchets at baseline. Findings and gaps (cold `--version` past the 3-second probe, unmeasured diagnostic-request churn, Gradle Kotlin unverified) in docs/editor-follow-ups.md (ticket 45 section) and docs/code/providers.md (Kotlin LSP, Editing row).
