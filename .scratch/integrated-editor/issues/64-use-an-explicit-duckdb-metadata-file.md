# 64: Use an explicit DuckDB metadata file

**What to build:** Opt into read-only catalog assistance from a chosen DuckDB file without executing document SQL.

**Blocked by:** 62: Deliver DuckDB offline assistance.

**Status:** resolved

- [x] Explicitly choose/connect/refresh/disconnect in both adapters and keep file opening offline by default.
- [x] Expose expected schema names/types/functions and freshness using identified metadata operations, disabling unintended extension/network access.
- [x] Observe statement boundaries, unchanged fixture data and absent export sentinels for destructive/quote-escape documents; reject stale results and show file/connection failures.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 96, 97, 98, 99, 100.

## Answer

Merged at `73e7140` (implementation `2edf1c3`). Both adapters reuse the explicit shared SQL metadata controls with an absolute local file path. Connect/Refresh harden a separate in-memory DuckDB engine before attaching only the chosen canonical database READ_ONLY, with its WAL allowed, extension installation/autoload and external access disabled. Fixed definition operations expose qualified tables/columns/types and function/macro declarations without binding or evaluating document SQL, views or macros. Snapshot age/state/failure is visible; Disconnect, source/scope changes, expiry and file/WAL identity changes reject stale assistance/replies. Missing files fail without creation.

Actual DuckDB 2.0.0 core and GUI/TUI input/render replays pass, including an apostrophe in the selected path, completion with type docs, qualified function completion, declared macro Parameter Info, Refresh/Disconnect, missing-file failure and unchanged document files. Destructive/export/quote/multiline dot-command drafts remain data. Syscall observation records database opens O_RDONLY and zero connect calls; selected database bytes/rows stay unchanged and no export/sentinel exists. File replacement/WAL staleness, 13 SQL core regressions, PostgreSQL 21 API regressions, DuckDB pure/native regressions, strict touched-crate clippy and both ratchets pass. Private evidence is under perf-results/editor64. Performance/stress remain deferred to the agreed end-of-epic campaign; polling/startup/catalog costs and macro inference limits are documented. Native macOS/Wayland remain unverified.
