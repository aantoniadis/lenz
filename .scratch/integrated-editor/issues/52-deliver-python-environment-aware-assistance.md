# 52: Deliver Python environment-aware assistance

**What to build:** Use Python completion, diagnostics and formatting matching the project’s configured environment.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Resolve configured interpreter/environment and project dependencies with real fixtures.
- [ ] Verify current-file completion/fix/format behavior, signatures and failures without selecting another environment silently.
- [ ] Both adapters expose capability/readiness state and reject stale or wider edits.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 84.
