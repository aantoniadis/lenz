# Lenz Code name and compatibility — 2026-10-27

The user named the integrated editor **Lenz Code**. The rename is implemented in source commits
`9269cfc` and `882cc0a`. Desktop window identity, terminal help, About text, language-server client
identification, error/probe prefixes, Cargo binary targets, packaging and active guides use the name.
Lenz Diff retains its name and command routing.

- `lenz [file-or-folder]` opens Lenz Code; `lenz --tui` selects its terminal adapter.
- Cargo produces `lenz-code` and `lenz-code-tui`, matching their existing installed command names.
- `lenz diff` and `lenz diff --tui` keep selecting Lenz Diff.
- Internal `reader-core`, `reader-gui` and `reader-tui` package/library names remain stable.
- Existing `lenz-reader` settings, cache, recent-project and recovery paths remain stable. No data
  migration or implicit settings reset is introduced. Historical reports retain the original names,
  paths and markers of their captured builds.
- *Superseded for the 0.0.7 release:* the packages became `code-core`, `code-gui` and `code-tui`, and the
  settings, state and cache folders `lenz-code`, moved from `lenz-reader` on first launch
  ([README](../code/README.md)).
- The launch collector accepts both `lenz-code:` and retained `lenz-reader:` probe markers with
  the same readiness boundary. No timing metric definition or performance ceiling changed.

## Verification

Checks used the checked-in release profile. Retained local logs are under ignored
`perf-results/lenz-code-rename/`; they are not public download links.

| Check | Result |
|---|---|
| Python collector/tool tests, including real packaging-script fixtures | 77 tests: 76 passes, one expected platform skip; includes eight packaging cases |
| Launcher argument/environment/PID/exit-status routing | Four tests passed |
| Desktop adapter and CLI | 199 adapter tests and two CLI/version tests passed |
| Terminal and actual GUI/TUI parity | 201 tests passed serially, plus the CLI/version test |
| Fresh complete core batch | 811 passed, 26 ignored |
| Strict touched-package Clippy, default and adapter feature configurations | Both passed, all targets, warnings denied |
| Existing Reader and terminal production panic-site counters | Passed unchanged budgets, using their existing counter logic with the release profile |
| Standalone desktop and terminal production builds | Both passed with the new Cargo executable names |
| Owned native X11 window | Title `Lenz Code`; WM_CLASS instance/class both `lenz-code`; fixture bytes unchanged |
| Owned ordinary terminal replay | Exact Unicode/CRLF typing, paste, Copy/Save, dirty-quit veto, Cancel/Discard and terminal restoration passed |
| Real five-command staging | `scripts/try-launcher.sh` passed; staged launcher help and terminal help route correctly; both Code executables report their proper name/version |

The first parallel core batch timed out in
`session::tests::a_refresh_during_a_pending_load_does_not_cross_it`; its unchanged exact repeat
and the fresh complete default core batch passed. The first parallel terminal batch timed out in
`parity_tests::project_name_navigation_reads_shifted_drafts_and_refuses_stale_disk_symbols_in_both_adapters`;
its unchanged exact repeat and complete serial terminal/parity batch passed. Both failed logs remain
retained. Their causes were not isolated, no timeout was raised and no correctness assertion was
relaxed. Serial success does not establish parallel reliability or explain the original timeout.

The initial private identity smoke probe looked for WM_CLASS immediately after finding the window
title. The pinned framework publishes class identity after view construction. The corrected probe
waits for both properties within its original 15-second bound; no application behavior changed to
accommodate it. The initial probe log remains retained.

This is a naming change; the existing performance campaign was not repeated. The earlier GUI paint
feedback timeout and performance-cost decisions remain recorded in the
[Linux validation report](code-integrated-editor-linux-validation.md). Native macOS/Wayland editing
gates and tickets 03/24/68 remain open. Mocked macOS packaging tests do not establish native execution.
