# 63: Use an explicit PostgreSQL metadata connection

**What to build:** Opt into private read-only PostgreSQL catalog assistance with visible freshness and connection state.

**Blocked by:** 61: Deliver PostgreSQL offline assistance.

**Status:** ready-for-agent

- [ ] Configure/connect/refresh/disconnect explicitly in both adapters; opening files never connects and credentials stay private.
- [ ] Fetch only identified metadata operations and demonstrate expected tables, columns/types and function signatures without executing functions or edited statements.
- [ ] Observe the actual database boundary with DROP/UPDATE/export/quote-escape fixtures, unchanged data, no EXPLAIN and no sentinel; reject stale catalogs and show connection failures.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 96, 97, 98, 99, 100.
