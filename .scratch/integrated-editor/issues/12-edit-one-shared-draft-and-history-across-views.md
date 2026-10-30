# 12: Edit one shared draft and history across views

**What to build:** Replace text, undo and redo through the session command boundary and show the same draft in split views while retaining independent view positions.

**Blocked by:** 11: Contract the old current-document ownership path.

**Status:** resolved

- [x] Demonstrate the complete command-to-presentation trace in both real adapters through existing test-support builds; production writable input remains gated.
- [x] Editing retains preview tabs; dirty state follows content equality; compact meaningful history survives actual cache eviction and file switching.
- [x] Check shared buffer/history across splits, independent sessions, invalid/stale/overlapping edits and full Unicode undo/redo.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 17, 18, 19, 20, 21, 22, 23, 24, 35.

## Comments

Initial snapshot chunks may share one original Document Arc<str> backing allocation. Exercise large-source deletion/replacement plus history eviction and later save/baseline changes: a tiny surviving slice can otherwise pin the full original allocation. Preserve bounded memory/compact history without per-edit full-document copies; use measured reclamation/repacking at an appropriate ownership boundary when necessary. If existing admission permits source-text edits for an image-derived format, invalidate/recompute derived pixels instead of displaying retained original SourceMeta pixels; otherwise preserve image read-only behavior. Do not add SVG editing solely for this note.

## Implementation handoff

Clean combined e3d7fa5 merged16c1dc1. Shared source drafts/compact history, exact dirty equality/cache retention, globally unique versions, atomic stale/overlap guards and500ms grouping are implemented. Eight focused core tracers and real GUI/TUI Unicode/CRLF/Paste/undo/caret/dirty/modal/split/independent-session traces pass, including normal read-only mode and strict affected lint. Report docs/research/code-draft-history.md; raw target/perf/editor-implementation/12-core plus adapter evidence. Production writable input remains gated. Final fullgate/performance/stress closure awaits batch11/12/13/16/18;13/16/18 implementation released against merged API.


## Resolution — 2026-10-24

Resolved against the phased full local correctness gate at runtimebb2f55f4, actualGUI/TUI/core regressions, equivalent release measurements and retained stress evidence. The user explicitly accepted this batch’s documented costs and requested continued implementation. Evidence: docs/research/code-editor-batch-11-18.md, code-editor-batch-11-18-performance.md and target/perf/editor-implementation/merge-11-18/. NativeMac and the report’s other limits remain unverified; latercohort acceptance is separate.
