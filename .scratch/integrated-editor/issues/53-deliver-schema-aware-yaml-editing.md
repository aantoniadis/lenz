# 53: Deliver schema-aware YAML editing

**What to build:** Receive schema keys/values, descriptions, validation and useful indentation while editing configuration.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Use explicit/project-resolved schemas and verify expected keys, values and diagnostic locations.
- [ ] Respect YAML indentation and configured formatting without changing unrelated files or resolving untrusted inputs as commands.
- [ ] Both adapters display suggestions/descriptions/errors and apply current-version edits safely.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 87.
