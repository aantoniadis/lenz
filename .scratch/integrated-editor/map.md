# Integrated editor implementation

Integration branch: `integration/integrated-editor`.

Parent: [Integrated Lenz Code and Editor](spec.md). Work is governed by the approved dependency graph in the individual ticket files.

## Notes

The main integration worktree owns tracker state. Implementers work on separate branches/worktrees, merge the latest integration tip before reporting, and provide red/green and regression evidence. Tickets resolve only after their work is merged and verified.

## Decisions so far

- Shared Session/real GUI/TUI seams and focused write/provider/performance checks were confirmed by the user.
- The user confirmed that no native macOS runner is available. Native termination and editor acceptance remain unresolved until required native evidence exists; other work proceeds.

## Fog

Native termination, automatic template/source mapping and complete offline PostgreSQL/DuckDB routes remain required capabilities. A missing tool or environment is not a passing test.

- [Ticket 02](issues/02-share-exact-ending-writes-with-explicit-save-outcomes.md) resolved after writer merge `ebc9354` and full integration gate at `177c167`; exact Reader write outcomes and target identity are available, with existing Diff behavior preserved.

- [Ticket01](issues/01-expand-the-reader-text-snapshot-boundary.md) resolved after snapshot merge/follow-up `177c167` and full integration gate; public indexed TextSnapshot and shared SourceMeta are available. Worker-only materialization and decoded-text construction preserve original metadata.

- [Ticket04](issues/04-resolve-template-projection-and-encoding-blockers.md) resolved after merge `45d3ce2`: bounded automatic template and corrected signature route demonstrated with real providers. Full context/adapter gaps are retained explicitly in57–59.

- [Ticket05](issues/05-select-postgresql-assistance-that-never-runs-edited-sql.md) resolved after merge8a58ce9 and isolated16-test verification: native PostgreSQL syntax plus offline query-local assistance demonstrated; product integration and finite-fixture gaps remain in61.

- [Ticket03](issues/03-provide-a-vetoable-native-macos-termination-route.md) remains blocked on native macOS execution. Source-pinned research and native acceptance procedure merged at `e63d1a0`; no production hook or native pass is claimed.

- [Ticket06](issues/06-resolve-duckdb-query-local-completion-gaps.md) resolved after merge `5032a37`: exact missing-name fixtures and native syntax/help/format boundary verified with private artifacts. Complete inference, adapter integration and product performance remain in62.

- [Ticket07](issues/07-migrate-code-core-reads-to-current-snapshots.md) resolved after runtime merge `39a830c`, full local gate and explicit user acceptance of the documented small opening/cached-operation costs. Shared current snapshots and bounded View reads are available;08–11 complete adapter/analysis ownership migration.

- Tickets08–10 resolved at runtime29fc0a/finalfixturebb162738, phased local checks complete, costs explicitly accepted2026-10-24. Reports807b27b and target/perf/editor-implementation/merge-08-10 retain evidence. Ticket11 claimed/released toworker08; nextmulti-ticketbatch11,12,13,16,18 follows dependencies. Nineof68resolved,59remain;03/nativeMacstillneeds-info.

- Ticket11 implementation-ready1cb06bb merged13ef78b; finalacceptance pending multi-ticket gate/performance. Ticket12 claimed/released to10 in da19 against merged API. Nine fullyclosed, one additional implemented pendingbatch; no productiontypingyet.

- Ticket12 implementation-ready e3d7fa5 merged16c1dc1; finalacceptance pending batch gate/performance.13 Save(08/b67f),16 draftsearch(10/da19),18 blame(09/2d02) claimed/released in parallel.9fullyclosed+2implementedpendingbatch.


- Tickets11/12/13/16/18 resolved2026-10-24 after complete phased correctness/release/stress evidence atbb2f55f4 and explicit user acceptance of the documented costs. **14of68resolved,54remaining.** Native03/24 stillunresolved. Mainc577e1d includes14/15/17/19/20/21/22 and23tooling/wholechain/nativefixes; newcohortfullgate/performancepending.
- User approvedfreshDiscardDraftandCloseafterUncertainSavewithoutrecheck2026-10-24;08implements actualclosureonly, diskuntouched, oldclosecancelled. Otherrecheckrulesstrict.
- User requestedPAUSEaftercurrentWIPfinishesandmerges. RemainingWIP:08freshcloseexception+09private23activation/verification. No newtickets afterthese; preserveallbranches/artifacts.
- [Ticket 33](issues/33-restore-the-project-session-around-recovered-work.md) resolved at merge `d0d8cf2`: recovered drafts restore into the reopened project's tabs, splits and places; an undecided draft's file is not edited (user decision 2026-10-24); a live session's drafts are not offered to another. 37 of 68 resolved.
- [Ticket 36](issues/36-rename-and-move-open-files-and-folders.md) resolved at merge `83494b3`: explorer Rename/Move with a no-replace rename; open tabs, drafts, history, places and the tree follow; no reference updates. 38 of 68 resolved.
- [Ticket 37](issues/37-delete-recoverably-and-confirm-permanent-removal.md) resolved at merge `f70e788`: explorer Delete to an in-house freedesktop trash (user decision 2026-10-24) with Restore, and a separately confirmed Delete Permanently; drafts refuse deletion. 39 of 68 resolved.
- [Ticket 39](issues/39-apply-completion-imports-and-snippet-edits-atomically.md) resolved at merge `cf31abc`: completion name, imports (eager or resolved) and snippet body as one transaction of the accepted version; same-name multi-caret. 40 of 68 resolved.
- [Ticket 40](issues/40-display-completion-documentation-and-signature-help.md) resolved at merge `ee498b1`: Parameter Info (command and trigger characters, version-fenced, overloads, encoding-aware labels) and browsed suggestion documentation in both adapters. 41 of 68 resolved.
- [Ticket 41](issues/41-show-diagnostics-and-apply-single-file-quick-fixes.md) resolved at merge `dbf9d6a`: version-fenced diagnostics with problem navigation and list, and single-file quick fixes applied as one undoable edit (wider fixes explained, never applied) in both adapters. 42 of 68 resolved.
- [Ticket 42](issues/42-format-and-organize-imports-under-project-settings.md) resolved at merge `ec816b6`: Format Document and Organize Imports as one undo step each, project format-on-save before Save (nothing written when formatting fails) and Save Without Formatting, in both adapters. 43 of 68 resolved.
- [Ticket 43](issues/43-explain-active-tools-environments-and-missing-capabilities.md) resolved at merge `bec4a06`: Server Details names the runtime, provider environment variable names (values withheld) and offered versus missing assistance; editing never waits for a server. 44 of 68 resolved.
- [Ticket 44](issues/44-deliver-java-project-aware-editing-assistance.md) resolved at merge `30098cf`: Java editing verified with real JDT LS 1.61.0 on Maven and Gradle projects (dependency import fix, completion import, parameter info, format, organize imports) through the core and both adapters; jdtls completions and parameter info fixed. 45 of 68 resolved.
- [Ticket 45](issues/45-deliver-kotlin-and-mixed-jvm-project-assistance.md) resolved at merge `f84f381`: Kotlin editing verified with real Kotlin LSP through allowlisted server commands (user decision 2026-10-25) and pull diagnostics; mixed Java/Kotlin limits re-verified. 46 of 68 resolved.
- [Ticket 46](issues/46-deliver-rust-editing-assistance.md) resolved at merge `df67744`: Rust editing verified with real rust-analyzer (import assist, typed members, completion with `use`, parameter info, rustfmt) through the core and both adapters; pull-diagnostic refresh fixed. 47 of 68 resolved.
- [Ticket 47](issues/47-deliver-go-editing-assistance.md) resolved at merge `69d023a`: gopls added as Go's provider and verified with real gopls v0.24.0 (module members, organize imports, parameter info, gofmt) through the core and both adapters. 48 of 68 resolved.
- [Ticket 48](issues/48-deliver-typescript-javascript-and-tsx-assistance.md) resolved at merge `96508e6`: TS/JS/TSX editing verified with real typescript-language-server (auto-import completion, fixes read from tsserver actions, format, organize, JSDoc members, TSX props) through the core and both adapters. 49 of 68 resolved.
- [Ticket 49](issues/49-deliver-ruby-project-and-gem-aware-assistance.md) resolved at merge `6e47efd`: Ruby editing verified with real Ruby LSP and RuboCop (fixes, formatting, constants, no-Gemfile projects) through the core and both adapters; incremental-sync servers now receive ranged changes. 50 of 68 resolved.
- [Ticket 50](issues/50-deliver-rails-aware-application-assistance.md) resolved at merge `5c5bbec`: Rails assistance verified with real Ruby LSP and its Rails add-on (columns, `where` completion, routes, associations, Rails cops) through the core and both adapters; just-admitted versions are sent before a request; a server group kill no longer becomes `kill(-1)` (`9fad004`). 51 of 68 resolved.
- [Ticket 51](issues/51-deliver-html-and-embedded-language-assistance.md) resolved at merge `8e8e10f`: HTML editing verified with real vscode-langservers-extracted 4.10.0 (embedded CSS/JS problems at exact ranges, formatting, closing tags through `html/autoInsert`, tag/attribute/CSS/JS completion) through the core and both adapters; the server's `css` settings section answered so pages get problems. 52 of 68 resolved.
- [Ticket 52](issues/52-deliver-python-environment-aware-assistance.md) resolved at merge `b676872`: Python editing verified with real Pyright 1.1.414 in configured virtual environments with installed dependencies; a missing configured environment stops Pyright with what to fix instead of its silent fallback; Python formats and organizes imports with the project's ruff or black, in both adapters. 53 of 68 resolved.
- [Ticket 53](issues/53-deliver-schema-aware-yaml-editing.md) resolved at merge `65fa87a`: yaml-language-server 2.0.0 added and verified (schema keys, values, descriptions and problems at exact ranges, YAML-aware Enter and spaces-only indentation, formatting) through the core and both adapters; no schema is downloaded unless the user allows it; typing before an identical character no longer steps over it; Alt+Enter in find lists all again (`3ce643a`). 54 of 68 resolved.
- [Ticket 54](issues/54-deliver-bash-and-sh-editing-assistance.md) resolved at merge `a704e4d`: bash-language-server 5.8.1 added; Lenz identifies Bash versus sh from directive, shebang, name, settings, `.shellcheckrc` and EditorConfig and runs ShellCheck and shfmt itself for that shell, never running the script, in both adapters. 55 of 68 resolved.
- User decision 2026-10-26: Python Organize Imports keeps running only ruff's import-sorting rules (`--select I`), as ruff's own editor integration does; a full Fix All belongs with ruff's problems and quick fixes (not shown yet), and removing unused imports (F401) is an optional later addition.
- [Ticket 55](issues/55-treat-environment-files-as-editable-data.md) resolved at merge `b941da6`: environment files detected by name, highlighted by a lexer and edited literally (`#` comments, quote pairing only after `=`) in both adapters; their contents reach no language server, index, settings, places or details text (only the private recovery record). 56 of 68 resolved.
- [Ticket 56](issues/56-edit-markdown-with-live-preview-and-authoring-help.md) resolved at merge `82616b5`: Markdown preview beside the draft follows unsaved versions off the UI thread (obsolete renders dropped, positions mapped to current text), list continuation and file/heading link completion as single Undo steps, Show in Source and Follow Link, in both adapters; a re-shown preview no longer stays Preparing. 57 of 68 resolved.
- [Ticket 57](issues/57-route-embedded-language-edits-through-source-maps.md) resolved at merge `fec6f27`: ERB and Haml Ruby (and ERB markup) routed to Ruby LSP and the HTML server through exact source maps under the template's identity; any edit outside one mapped region is refused whole; verified with real servers through the core and, for ERB, both adapters. 58 of 68 resolved.

- [Ticket 59](issues/59-deliver-first-class-haml-editing.md) resolved at merge `2f45554`: Haml-native tag/attribute completion, spaces-only structural Enter, nested Ruby blocks/attributes/interpolation, static Rails helper context and configured haml-lint formatting, through the shared Session and both adapters. Real compiler/provider/formatter and GUI/TUI replays, filtered regressions, clippy and both panic-site ratchets pass; unsupported contexts and end-of-epic costs remain explicit in the provider docs and follow-ups. **59 of 68 resolved.** Ticket 58 remains claimed by another agent; ready frontier: 61, 62, 67. Native 03 still blocks 24/68.

- [Ticket 61](issues/61-deliver-postgresql-offline-assistance.md) resolved at merge `3f8290c`: PostgreSQL offline editing is wired through the shared admitted-version scheduler, with native syntax, query-local completion, Parameter Info and guarded formatting in both adapters. Expanded scope fixtures and captured stale replies pass; the observed destructive-source boundary made no socket/connect calls or provider child execs. **60 of 68 resolved.** Ready frontier: 62, 63, 67; ticket 58 remains claimed elsewhere, native 03 blocks 24/68.

- [Ticket 62](issues/62-deliver-duckdb-offline-assistance.md) resolved at merge `cbedfdb`: real DuckDB offline editor assistance, bounded cancellable encoded stdin, native overloads/diagnostics/formatting, folded quoted names and wildcard RENAME/REPLACE, with real GUI/TUI replay. Six native destructive/dot-command traces made no connections or private fixture opens. **61 of 68 resolved.** Ready frontier: 63, 64, 67; 58 remains claimed elsewhere and native 03 still blocks 24/68.

- Ticket63 resolved at `bf4060d`: explicit read-only PostgreSQL catalog snapshots, real server statement observation and real GUI/TUI controls. 62 of 68 resolved; 58 remains claimed elsewhere.

- Ticket64 resolved at `73e7140`: explicit read-only DuckDB metadata files, actual boundary/syscall observation and GUI/TUI controls. 63 of 68 resolved; 58 remains claimed elsewhere.

- Ticket67 resolved at `6001628`: inspect marked prior content, captured baseline/shared-draft comparison and guarded undoable Revert; actual HEAD/last-save GUI/TUI replays pass. **64 of 68 resolved.** Remaining: 58 claimed elsewhere; 03 needs native macOS execution, blocking24 and68. No other unclaimed dependency-ready ticket remains.

- [Ticket 58](issues/58-deliver-first-class-erb-editing.md) resolved at merge `18c7b8a`: ERB editing with Rails view context (Action View and the app's helpers, strict locals) through the ticket 57 route and the project's erb_lint as formatter, verified with real servers through the core and both adapters; controller instance variables and route-helper suggestions are not offered by the servers. Open bug from ticket 57 recorded: the GUI can lose a template's problems after Undo. 65 of 68 resolved (after ticket 67).

- Ticket 68 [Linux validation checkpoint](../../docs/research/code-integrated-editor-linux-validation.md): fresh full gate, 62 available-provider checks, native safety checks and completed canonical/final Reader phases are recorded; 900-second reading stress passed. Integrated editing stress was interrupted, follow-ups and the single cost decision remain pending. Fixes are on pushed `editor-68-linux-validation`, not integration. Native 03/24 and full 68 remain open; 65 of 68 resolved.

- User-directed merge `422cb9e` integrates Linux repairs/validation with ticket 58 preserved. Full 900-second integrated editing stress passed (30,980 iterations), alongside reading stress and template costs. Remaining performance follow-ups are running; the full-campaign cost decision and native 03/24/68 remain open. 65 of 68 resolved.

- Ticket 68 Linux/shared continuation: [review and measurements](../../docs/research/code-integrated-editor-linux-validation.md) record reproduced production repairs, fresh complete local gate, 62 available-provider checks, native SQL/X11/PTY safety and the end-of-epic release campaign. Final runtime `ff217fc`, fixture-only follow-up `d6fd35b`; canonical cohort `b072d0e`. The single cost decision is pending. Tickets 03/24 and full 68 stay open; 65 of 68 resolved. No macOS/Wayland or fresh Kotlin pass is implied, and no platform gate is removed. Integration merge `422cb9e` was user-directed. The original absolute gate remains failed (baseline view also fails); individual remainder/search results are recorded without relaxing ceilings.

- User decision 2026-10-27: the integrated editor is **Lenz Code**. Product surfaces and build outputs use `lenz-code` / `lenz-code-tui`; the `lenz` / `lenz --tui` command family, internal package names and existing settings/recovery namespaces remain compatible. Native 03/24 and full release 68 remain open.
