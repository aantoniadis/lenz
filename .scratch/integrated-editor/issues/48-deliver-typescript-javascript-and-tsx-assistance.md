# 48: Deliver TypeScript, JavaScript and TSX assistance

**What to build:** Edit TS, JS and React TSX with imports, diagnostics, formatting and component/prop help.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Verify real project configuration, dependencies, TS/JS language modes and React component/prop completion.
- [ ] Apply completion/import/fix/format edits atomically to the current file and reject wider edits.
- [ ] Exercise both adapters, unsaved targets, Unicode ranges and tool environment/readiness failures.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 81, 82.
