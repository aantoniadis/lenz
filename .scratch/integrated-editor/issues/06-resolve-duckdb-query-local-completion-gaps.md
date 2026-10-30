# 06: Resolve DuckDB query-local completion gaps

**What to build:** Produce a verified DuckDB assistance route that handles the names missing from the builtin autocomplete results.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Return the expected alias, CTE, nested-query, quoted and empty-qualified-name completions rather than merely nonempty suggestions.
- [x] Verify DuckDB-specific syntax, malformed/incomplete input, function help, syntax ranges and formatting with edited text passed only as data.
- [x] Retain observed operations, resource costs and source mappings; prove no external database connection offline and no statement/EXPLAIN execution.

**Spec coverage:** user stories 93, 94, 95, 99.

## Answer

Implemented at `953d4bb`, merged at `5032a37` (integration now `e63d1a0`). Query-local alias/CTE/nested/quoted/qualified fixtures now return exact expected names, including empty-qualified completions. Native DuckDB v2.0.0 provides syntax-only diagnostics, guarded formatting and builtin/signature help; edited SQL is quoted data in fixed in-memory assistance operations. Explicit tests distinguish valid non-SELECT serializer limitations from real parser errors, including errors in later statements. Private validation passed 539 core tests, 21 focused SQL tests, four native tests and strict Clippy; merger independently passed 21 + four tests and unchanged panic limits. Source/executable hashes and five new traced native processes (zero connect or fixture/export opens) are retained in `target/perf/editor-implementation/merge-06`; original and private evidence locations are documented in `docs/research/duckdb-query-local-assistance.md`. Shared-target Rust API timing samples are excluded from acceptance; private debug samples and direct native process resource observations are explicitly limited. Product integration, complete inference and release/GUI/TUI/native macOS performance remain work for62 and later gates.

Before eventual worktree cleanup, all private validation logs/JSON and the exact test executable were copied and hash-verified under `target/perf/editor-implementation/06/private-validation`; `retained-paths.json` maps original and retained paths. Documentation follow-up `40bbd76` records the durable local pointer.
