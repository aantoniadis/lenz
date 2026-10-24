# 09: Migrate terminal rendering to bounded source windows

**What to build:** Render and navigate the existing terminal Reader through the new current-text boundary.

**Blocked by:** 07: Migrate Reader core reads to current snapshots.

**Status:** resolved

- [x] Migrate terminal source/layout/hit-testing reads while preserving reading shortcuts until editing is deliberately enabled.
- [x] Long-line horizontal/wrapped movement preserves absolute source coordinates and clamps scroll after input, render and resize.
- [x] Real terminal event/render checks cover empty and 1×1 layouts, End then Up, Unicode and modal dismissal.

**Spec coverage:** user stories 3, 7, 114, 116, 117, 118.

## Comments

Adapter geometry guards need TabId, current SnapshotKey, viewport/metric and decoration/fold/sticky identities. Real render→reload→click regression was reproduced before the initial fix. Hit/source reads must use View.snapshot, with measured cell origins referencing the same shared projected cursor used by paint. Baseline owned PTY at1d874ae: first content7.116ms,60.000s/740pagecycles,settledRSS36.91MiB, sampledpeak40.18MiB, idle0bytes/0ticks; raw evidence in target/perf/editor-implementation/09.

Shared pathological-grapheme contract: after normal control/mark projection and hint insertion, escape an extending scalar as ASCII [U+XXXX] when the projected grapheme would exceed256UTF-8bytes. Maximum projected grapheme257bytes because a Prepend run can absorb the first [. Guard state persists across checkpoints/windows independently of viewport width; exact original source/copy/edit coordinates remain. Ordinary below-threshold emoji/combining text keeps its rendering. Keep a separate whole-shaping-input byte budget. Test Prepend directly at the guard layer, plus ZWJ/Indic/hint boundaries. This bounded display fallback is within the approved task, not a new permission gate.

## Answer

Resolved with integrated runtime29fc0a, final fixturebb162738 and batch documentation807b27b. The phased local gate completed all required workspace/core/real GUI/TUI/parity, writer/provider, isolated memory, collector and release/size checks. Failures were repaired with focused reruns; no full-suite or stress rerun per ticket. Raw commands/sources/statuses: target/perf/editor-implementation/merge-08-10/. Reports: docs/research/reader-snapshot-batch-08-10.md and linked scoped reports. User explicitly accepted the documented08–10 performance costs on2026-10-24. Scope and unverified native macOS/physical display/GPU/external-provider limits remain explicit; editing is still gated.
