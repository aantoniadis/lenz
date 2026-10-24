# 11: Contract the old current-document ownership path

**What to build:** Finish the representation migration so every current Reader view resolves one authoritative snapshot.

**Blocked by:** 08: Migrate GUI rendering to bounded source windows; 09: Migrate terminal rendering to bounded source windows; 10: Admit one current version for background analysis.

**Status:** resolved

- [x] Remove the compatibility ownership/read paths only after all migration batches pass; retain contiguous Documents explicitly for worker consumers.
- [x] Run existing core/GUI/TUI parity and shared Diff/merge regressions, including independent sessions and empty constructors.
- [x] Compare equivalent release startup/navigation and memory against readiness baselines, fix reproducible regressions and retain raw evidence.

**Spec coverage:** user stories 114, 115, 118.

## Comments

The07 compatibility implementation charges snapshot storage_bytes as additional index/local-edit storage beside the retained Document, excluding shared original text/metadata. When removing Document ownership, revise cache/buffer accounting to charge each retained backing allocation exactly once; preserve the bounded budget after shared newline indexes as well.

10 found a classification discrepancy: loaded Document::is_large considers original encoded bytes, while Snapshot::is_large considers decoded UTF8; large UTF16 files can disagree. Preserve the existing reader/admission policy when contracting ownership, and make any distinct analysis-cost/current-draft rule explicit.10 will keep the matching-loaded-Document syntax fast path unchanged and add a focused test; do not silently broaden edit admission based on normalized length.

## Implementation handoff

Implementation-ready1cb06bb merged13ef78b. Snapshot-only current ownership/cache and View consumers replace legacy Document getters and wrap cache. Captured result slots share bounded conversion while outliving view/source changes, with target validation before navigation. Focused public semantics98pass/5ignore, current21, materialization4, exactv2/v3/release/ack1, target-current1, scheduler12 and strict core/adapter lint pass; real preview/virtual/source workflows preserved. Evidence docs/research/reader-current-source-ownership.md and target/perf/editor-implementation/11-core,11/session-tests,11/result-readiness,11-adapter-tests. Final fullgate/performance/stress closure remains pending multi-ticket batch11/12/13/16/18. Dependent implementation12 is released against merged API; do not force a per-ticket gate.


## Resolution — 2026-10-24

Resolved against the phased full local correctness gate at runtimebb2f55f4, actualGUI/TUI/core regressions, equivalent release measurements and retained stress evidence. The user explicitly accepted this batch’s documented costs and requested continued implementation. Evidence: docs/research/reader-editor-batch-11-18.md, reader-editor-batch-11-18-performance.md and target/perf/editor-implementation/merge-11-18/. NativeMac and the report’s other limits remain unverified; latercohort acceptance is separate.
