# 64: Use an explicit DuckDB metadata file

**What to build:** Opt into read-only catalog assistance from a chosen DuckDB file without executing document SQL.

**Blocked by:** 62: Deliver DuckDB offline assistance.

**Status:** ready-for-agent

- [ ] Explicitly choose/connect/refresh/disconnect in both adapters and keep file opening offline by default.
- [ ] Expose expected schema names/types/functions and freshness using identified metadata operations, disabling unintended extension/network access.
- [ ] Observe statement boundaries, unchanged fixture data and absent export sentinels for destructive/quote-escape documents; reject stale results and show file/connection failures.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 96, 97, 98, 99, 100.
