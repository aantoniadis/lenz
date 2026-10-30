# 44: Deliver Java project-aware editing assistance

**What to build:** Use completion/imports, signatures, diagnostics, quick fixes and formatting in Java projects.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** resolved

- [x] Verify real Maven/Gradle/dependency project discovery with configured tools and Java source fixtures.
- [x] Apply current-file completion/import/fix/format transactions through both adapters with exact undo and stale-response checks.
- [x] Document tested environments and actual capability gaps without treating a connected server as a pass.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 75, 76.

## Answer

Merged in 30098cf (branch work/editor-44-java). Verified with real JDT LS 1.61.0 (installed for this ticket from the Eclipse milestone tarball, checksum verified), OpenJDK 21.0.12, Maven 3.9.11 and Gradle 8.14.3 on Linux. On a Maven and on a Gradle project depending on Gson: problems for the shown version, the dependency's "Import 'Gson' (com.google.gson)" quick fix (proof the build and classpath were discovered) applied as one Undo step, completion of `ArrayList` with its import as one Undo step, parameter information across Gson's `toJson` overloads, Format Document as one Undo step, a format answered after an edit refused, Organize Imports removing an unused import, other-file fixes ("Create class …") listed and not applied, and the files on disk unchanged. Two product fixes: JDT LS's `java.completion.onDidSelect` on every suggestion made Lenz refuse all Java completions (now skipped: it only ranks later suggestions), and JDT LS answers parameter information only when `java.signatureHelp.enabled` is set (now in Lenz's defaults).
Evidence: ignored real-provider tests `real_jdtls_maven_editing` (11.9 s) and `real_jdtls_gradle_editing` (37.2 s) in crates/code-core/tests/providers.rs, and `real_jdtls_import_fix_and_format_match_in_both_adapters` in crates/code-tui/src/parity_tests.rs (Alt+Enter, Down, Enter, Ctrl+Alt+L, Ctrl+Z in both adapters; 23.7 s); all pass here. Filtered gate: core lib 26 passed, semantics completion/parameter filter passing except the two environmental Ruby LSP tests, TUI 6 and reader GUI 2 passed; clippy clean; panic-site ratchets at baseline. Tested environments and gaps in docs/code/providers.md (Eclipse JDT LS, Editing row) and docs/editor-follow-ups.md (ticket 44 section); the older `real_jdtls` navigation test fails at its JDK-source step here (recorded, not chased). Organize-imports choices for ambiguous types, Gradle import side effects, native macOS, Wayland and physical terminals unverified.
