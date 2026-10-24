# 47: Deliver Go editing assistance

**What to build:** Use module-aware Go completion/imports, diagnostics, quick fixes and formatting.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Resolve the project’s installed/configured Go language and formatting tools without app-driven installation.
- [ ] Prove real module/dependency context and current-file edits for the accepted capabilities.
- [ ] Both adapters expose readiness, actions and failure behavior; absent tools stay unverified rather than passing acceptance.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 77.
