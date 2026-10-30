# 05: Select PostgreSQL assistance that never runs edited SQL

**What to build:** Prove a PostgreSQL-specific offline language route that meets query-local assistance requirements without executing or explaining the document.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Pin expected alias/CTE/subquery/quoted/qualified names, function help, syntax locations and formatting against incomplete and PostgreSQL-specific fixtures.
- [x] Observe the actual request boundary and demonstrate no external database connection offline, no edited-statement execution and no EXPLAIN.
- [x] Record provider identity and measured latency/resources; unavailable tools or a hypothetical adapter do not resolve this ticket.

**Spec coverage:** user stories 93, 94, 99.

## Answer

Merged through8a58ce9 (implementation1908e02, main mergeee8d0f1, checked-slice follow-up8a58ce9). Real native PostgreSQL18.4 syntax parsing, finite query-local scope completion, guarded formatting and curated function/signature help are available without a database client or executor. Merger verified16 SQL tests, strict Clippy, unchanged panic budget and source/executable hashes for retained60-second provider measurements; observed boundary had zero network and no provider children. A fresh isolated integration-target build independently passed16 SQL tests (one measurement probe intentionally ignored), eliminating shared-target artifact ambiguity for the final merged functional checks. Evidence: `target/perf/editor-implementation/05`, `merge-05`, and `05-isolated-integration`. Full local gate last passed177c167 before this provider merge; product GUI/TUI and native macOS are not verified by this prerequisite. Detailed finite-fixture and portability limits are in `docs/research/postgres-offline-assistance.md` and explicitly carried to61.
