# 61: Deliver PostgreSQL offline assistance

**What to build:** Write PostgreSQL queries with query-local completion, function help, syntax diagnostics and formatting without a database.

**Blocked by:** 05: Select PostgreSQL assistance that never runs edited SQL; 60: Choose and persist a visible SQL dialect; 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings.

**Status:** resolved

- [x] Integrate the proven route and pin expected alias/CTE/subquery/quoted/qualified-name and PostgreSQL-specific behavior.
- [x] Both adapters apply current-version suggestions/formatting and show diagnostic ranges while incomplete SQL remains editable.
- [x] Observe zero metadata connections, zero edited-statement/EXPLAIN execution and rejection after dialect/context changes.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 93, 94, 99, 101.

## Comments

Ticket05 selected the real offline route at integration8a58ce9. Read `docs/research/postgres-offline-assistance.md` and its Required handoff before implementation. Completion currently proves finite alias/CTE/subquery/quoted/qualified fixtures; UNION/recursive CTE/qualified wildcard/function-return inference and arbitrary incomplete-input recovery remain required work where applicable. Help currently covers three curated functions. Extend name visibility, overload/builtin coverage and formatter fixtures; preserve source on rejection. Native PostgreSQL18.4 parsing is syntax-only; schema/semantic validation is separate. Build needs make, C compiler and libclang. Add request identity/cancellation, bounded workers and actual GUI/TUI current-version application. Native macOS, large catalogs and product interaction/performance remain unverified. The prerequisite is not complete product SQL support.


## Answer

Merged at `3f8290c` (implementation `cc64e62`). The editor now invokes the real offline PostgreSQL route through the shared bounded admitted-version scheduler. Both adapters expose current-draft completion, native syntax ranges, builtin Parameter Info and guarded formatting with project indentation; applied edits are one Undo step and write only through Save. Dialect, source, project, document version and request identity fence responses, including captured replies after Escape or an edit. No database connection or execution API is on this route.

Scope support was extended for UNION output names, recursive CTE column lists, qualified wildcards, unfinished quoted identifiers, `pg_catalog` functions, known table-function outputs, non-LATERAL isolation and ambiguous unqualified-column omission. Completion ignores a malformed different statement. The builtin summaries now cover 28 common names. Capability-specific limits remain documented in [PostgreSQL offline editing](../../../docs/code/providers.md#postgresql-offline-editing); arbitrary malformed recovery, exhaustive visibility/type inference and extension/overload catalogs are not claimed.

Verification: 3 session tests, 6 dialect regressions, 21 native/provider tests, 5 DuckDB scope regressions, actual GUI/TUI input/render/Undo replay, touched-crate clippy with warnings denied and both panic-site ratchets pass. The destructive-input test was traced outside the ptrace-restricted sandbox: zero socket/connect calls, one initial test exec and no provider child execution; source/export/program sentinels were unchanged/absent. Red tests established the missing editor route, quoted-name and set-output failures before implementation. Performance/stress and full epic/native-platform validation remain deferred under the handoff policy; costs and limits are in editor follow-ups.
