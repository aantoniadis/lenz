# 16: Search current drafts and validate result snapshots

**What to build:** Make project text and declaration search use current open-file drafts with matching previews and activation.

**Blocked by:** 12: Edit one shared draft and history across views.

**Status:** resolved

- [x] Draft-only tokens/declarations appear, removed saved matches disappear, and unopened files still use disk.
- [x] Capture project/draft-set and per-target identity so results, previews and activation agree; changed targets recompute or report stale results without moving incorrectly.
- [x] Replay through both adapters with shifted lines, an edit after results arrive and unchanged disk bytes.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 43, 44, 46, 47.


## Resolution — 2026-10-24

Resolved against the phased full local correctness gate at runtimebb2f55f4, actualGUI/TUI/core regressions, equivalent release measurements and retained stress evidence. The user explicitly accepted this batch’s documented costs and requested continued implementation. Evidence: docs/research/code-editor-batch-11-18.md, code-editor-batch-11-18-performance.md and target/perf/editor-implementation/merge-11-18/. NativeMac and the report’s other limits remain unverified; latercohort acceptance is separate.
