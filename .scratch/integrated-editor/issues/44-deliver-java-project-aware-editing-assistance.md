# 44: Deliver Java project-aware editing assistance

**What to build:** Use completion/imports, signatures, diagnostics, quick fixes and formatting in Java projects.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Verify real Maven/Gradle/dependency project discovery with configured tools and Java source fixtures.
- [ ] Apply current-file completion/import/fix/format transactions through both adapters with exact undo and stale-response checks.
- [ ] Document tested environments and actual capability gaps without treating a connected server as a pass.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 75, 76.
