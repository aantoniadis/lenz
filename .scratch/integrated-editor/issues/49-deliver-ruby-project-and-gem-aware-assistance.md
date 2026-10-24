# 49: Deliver Ruby project and gem-aware assistance

**What to build:** Use Ruby constants/methods, signatures and configured diagnostics/formatting in the project’s gem context.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Verify real bundled/non-bundled project discovery with installed gems and stable indexing startup.
- [ ] Resolve provider encoding/signature behavior for exact current-file edits and demonstrate configured lint/format tools.
- [ ] Expose the same supported actions/readiness in both adapters without installing tools or hiding missing capabilities.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 78.
