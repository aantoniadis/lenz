# 10: Admit one current version for background analysis

**What to build:** Keep syntax, find, Markdown, index and semantic analysis responsive while they consume explicit shared background Documents.

**Blocked by:** 07: Migrate Reader core reads to current snapshots.

**Status:** resolved

- [x] Use existing workers with one running and one replaceable pending version per buffer plus a bounded global identity queue.
- [x] Reuse one admitted materialization and reject results after buffer/version/request/context changes; derived products never replace current text.
- [x] Exercise cancellation after work starts and release superseded snapshots; preserve real parser/LSP handoff behavior.

**Spec coverage:** user stories 68, 74, 115.

## Comments

An implementation map was prepared as a local note before implementation (not kept). Existing Jobs limits running threads but its closure queues are unbounded and retain stale Documents. Replace actual producer scheduling, not just a standalone test type. Preserve existing provider/parser protocols and independent operation identities within one version.

Verify session teardown cancels in-flight analysis before Jobs joins worker threads: the existing drop joins workers, while version predicates may retain the last shown versions. Use actual parser/worker cancellation and ownership checks so closing a session does not wait for obsolete whole-file work. Keep a deterministic release/finalization path for cancellation and worker panic so admission slots cannot remain stuck.

Investigation correction: Session already has a Drop cancellation implementation. The specific remaining probe is the other split pane's live version predicate; active release alone may leave it valid while Jobs joins. Advisor source verification also confirms that omitting a pending file from Semantics::sync causes didClose, so retain its exact last-sent Document until new readiness is validated and synchronized. Session-owned admission (beside Sources) and bounded per-consumer fanout were selected by the fresh advisor; Jobs stays the executor. Find slots must remain per TabId. Geometry continuation remains snapshot-based and separate from contiguous Document analysis.

The existing1024-message LSP writer queue can retain older full-text Arcs. Exercise a blocked writer plus repeated admitted versions, and demonstrate an explicit bounded retained-text story before treating the future editing pipeline as memory-safe. Preserve protocol ordering; use acknowledgements or version-aware coalescing with request barriers if required. Do not claim an end-to-end two-version memory guarantee from the CPU admission group alone.

## Answer

Resolved with integrated runtime29fc0a, final fixturebb162738 and batch documentation807b27b. The phased local gate completed all required workspace/core/real GUI/TUI/parity, writer/provider, isolated memory, collector and release/size checks. Failures were repaired with focused reruns; no full-suite or stress rerun per ticket. Raw commands/sources/statuses: target/perf/editor-implementation/merge-08-10/. Reports: docs/research/code-snapshot-batch-08-10.md and linked scoped reports. User explicitly accepted the documented08–10 performance costs on2026-10-24. Scope and unverified native macOS/physical display/GPU/external-provider limits remain explicit; editing is still gated.
