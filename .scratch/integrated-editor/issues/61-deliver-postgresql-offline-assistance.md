# 61: Deliver PostgreSQL offline assistance

**What to build:** Write PostgreSQL queries with query-local completion, function help, syntax diagnostics and formatting without a database.

**Blocked by:** 05: Select PostgreSQL assistance that never runs edited SQL; 60: Choose and persist a visible SQL dialect; 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings.

**Status:** ready-for-agent

- [ ] Integrate the proven route and pin expected alias/CTE/subquery/quoted/qualified-name and PostgreSQL-specific behavior.
- [ ] Both adapters apply current-version suggestions/formatting and show diagnostic ranges while incomplete SQL remains editable.
- [ ] Observe zero metadata connections, zero edited-statement/EXPLAIN execution and rejection after dialect/context changes.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 93, 94, 99, 101.

## Comments

Ticket05 selected the real offline route at integration8a58ce9. Read `docs/research/postgres-offline-assistance.md` and its Required handoff before implementation. Completion currently proves finite alias/CTE/subquery/quoted/qualified fixtures; UNION/recursive CTE/qualified wildcard/function-return inference and arbitrary incomplete-input recovery remain required work where applicable. Help currently covers three curated functions. Extend name visibility, overload/builtin coverage and formatter fixtures; preserve source on rejection. Native PostgreSQL18.4 parsing is syntax-only; schema/semantic validation is separate. Build needs make, C compiler and libclang. Add request identity/cancellation, bounded workers and actual GUI/TUI current-version application. Native macOS, large catalogs and product interaction/performance remain unverified. The prerequisite is not complete product SQL support.
