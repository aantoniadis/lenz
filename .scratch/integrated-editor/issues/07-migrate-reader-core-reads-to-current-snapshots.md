# 07: Migrate Reader core reads to current snapshots

**What to build:** Keep Reader navigation and view state working while resolving current text through session-owned source identity.

**Blocked by:** 01: Expand the Reader text-snapshot boundary.

**Status:** resolved

- [x] Migrate core caret/selection/copy/goto, view helpers and metadata availability as a mechanical batch while keeping the compatibility path for unmigrated consumers.
- [x] Loading, empty, image and virtual-source constructors retain their current observable behavior.
- [x] Core reading tests establish exact source positions and shared snapshot identity without enabling production editing.

**Spec coverage:** user stories 18, 19, 20, 117, 118.

## Comments

Implementation and private functional checks are available on `work/editor-07-core-reads`, but this ticket remains claimed pending performance preservation. Main integration is `40bbd76`. Shared Cargo target collisions were observed; all acceptance builds now use isolated targets and prior shared-target timing samples are explicitly excluded. Fresh measurements and executables are retained under `target/perf/editor-implementation/07/`. A fresh advisor (`snapshot_admission_perf_advice`) reviewed residual opening overhead and recommended sharing existing newline-index ranges after a smaller exact-size/fused-prefix experiment. The smaller experiment passed core tests and a 180.10-second stress run (1,250,920 operations, 25,018 controlled external rewrites), but repeated opening timing still regressed, so further index-sharing evaluation is in progress. No deliberate performance tradeoff is approved.

Next implementation maps for 08, 09 and 10 were prepared as local notes outside the repo (not kept). They are source pointers, not implemented features. Coordinate shared source/display/wrap ownership between08 and09;10 owns actual producer admission/materialization and worker result validation.

Final runtime candidate including the red→green cached-rename fix merged at `39a830c`; the full local gate passed exit0 in444.18 seconds, including actual GUI/TUI, memory, collectors and release/size checks. Performance documentation committed at `c9105eb`: `docs/research/reader-snapshot-migration-performance.md`, README and notebook. All109 local links/diff checks passed. Ten cached-operation repeat pairs reproduced long tails in both baseline and candidate; ordinary candidate medians remain about0.2µs higher. Repeated first-text medians remain6–8µs higher for50,400 bytes and24–88µs higher for945,000 bytes. No cost is approved; awaiting explicit user agreement under AGENTS.md81–83. Ticket stays claimed and08–10 stay prepared but blocked. Current raw/private evidence is retained in main before eventual worktree cleanup.

## Answer

Resolved after final runtime merge `39a830c`, full local gate exit0 (444.18 seconds), documented experiments `c9105eb` and explicit user acceptance of the measured costs on2026-10-23. User chose “Accept measured costs and continue”:6–8µs added core first-text time for50,400 bytes,24–88µs for945,000 bytes and about0.2µs for typical cached operations. This agreement applies to these recorded costs, not future regressions or unverified platforms. Source ownership, semantic reads and bounded View API are implemented; original Document compatibility consumers are mapped for08–11. Red→green cached-rename/reactivation source identity is fixed, preserving retained old snapshots. Full local gate includes real GUI/TUI adapter checks and release/size; native macOS and physical display latency remain unverified. Source/correctness/performance evidence and exact executables are retained under `target/perf/editor-implementation/07` and `merge-07`.
