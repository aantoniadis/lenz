# 63: Use an explicit PostgreSQL metadata connection

**What to build:** Opt into private read-only PostgreSQL catalog assistance with visible freshness and connection state.

**Blocked by:** 61: Deliver PostgreSQL offline assistance.

**Status:** resolved

- [x] Configure/connect/refresh/disconnect explicitly in both adapters; opening files never connects and credentials stay private.
- [x] Fetch only identified metadata operations and demonstrate expected tables, columns/types and function signatures without executing functions or edited statements.
- [x] Observe the actual database boundary with DROP/UPDATE/export/quote-escape fixtures, unchanged data, no EXPLAIN and no sentinel; reject stale catalogs and show connection failures.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 96, 97, 98, 99, 100.

## Answer

Merged at `bf4060d` (implementation `2631db4`). Both adapters expose Configure/Connect/Refresh/Disconnect in the shared SQL menu. A configured libpq service stays session-private; configuring/opening never connects. Explicit fetches use fixed pg_catalog operations in a read-only transaction with bounded time/output, returning qualified relations, columns/types and up to eight matching function declarations. The status bar shows age/state; details explain failure/disconnection. Refresh, Disconnect, expiry and scope changes invalidate old assistance and late replies. Snapshots expire after five minutes; connections close after fetch.

PostgreSQL 17.11 native core and actual GUI/TUI input/render replays pass: destructive, export, quote and psql escape drafts never reach the server; only fixed metadata transactions appear in the server statement log. The fixture remains `42|unchanged`, an exception-raising user function is never called and no sentinel/export appears. Completion, type docs, Parameter Info, refresh/disconnect, invalid service/failure, expiry and stale replies are checked. Twelve filtered SQL core regressions, 21 PostgreSQL API regressions, empty catalog expectations, offline adapter replay, strict touched-crate clippy and both ratchets pass. Evidence is retained privately under perf-results/editor63. End-of-epic performance/stress and native macOS/Wayland checks remain deferred/unverified; costs and qualified-name limits are documented.
