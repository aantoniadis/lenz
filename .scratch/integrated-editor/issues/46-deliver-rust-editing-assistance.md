# 46: Deliver Rust editing assistance

**What to build:** Use type-aware Rust completion/imports, diagnostics, quick fixes, signatures and formatting.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Verify an actual Rust project and configured toolchain with real provider fixtures.
- [ ] Show member/type-aware suggestions and apply safe current-file imports/fixes/formatting with one-action undo.
- [ ] Replay through both adapters, including missing/indexing/error states and cancellation during editing.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 77.
