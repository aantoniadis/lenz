# 08: Migrate GUI rendering to bounded source windows

**What to build:** Render and hit-test the existing GUI Reader from the new current-text boundary without changing reading behavior.

**Blocked by:** 07: Migrate Reader core reads to current snapshots.

**Status:** resolved

- [x] Migrate GUI text/layout/hit-testing reads with absolute source ranges and versioned caches; keep unmigrated consumers working.
- [x] Scroll and select beyond character 10,000 without full-tail counting or repeated complete-prefix layout; preserve combining/control-character mapping.
- [x] Real GUI constructor/input/render tests cover images, empty state, light/dark, narrow windows and stale layout rejection.

**Spec coverage:** user stories 3, 7, 114, 116, 117, 118.

## Comments

Shared layout decision: extend the per-Tab cache with resumable source/display cursors and bounded row projection/stepping through View, passing the authoritative snapshot explicitly. Core ScrollRows/caret helpers must stop requiring complete row arrays. Cold seeks use exact Ready/Pending progress on existing Jobs, scanning outside cache locks; publish only for matching snapshot/source metadata, hints, width and navigation intent. Cache strings/maps/pages by bytes; account sparse checkpoint metadata. 08 owns the narrow shared layout producer/completion integration, coordinated with10, and publishes the stable contract to09.

Shared pathological-grapheme contract: after normal control/mark projection and hint insertion, escape an extending scalar as ASCII [U+XXXX] when the projected grapheme would exceed256UTF-8bytes. Maximum projected grapheme257bytes because a Prepend run can absorb the first [. Guard state persists across checkpoints/windows independently of viewport width; exact original source/copy/edit coordinates remain. Ordinary below-threshold emoji/combining text keeps its rendering. Keep a separate whole-shaping-input byte budget. Test Prepend directly at the guard layer, plus ZWJ/Indic/hint boundaries. This bounded display fallback is within the approved task, not a new permission gate.

## Answer

Resolved with integrated runtime29fc0a, final fixturebb162738 and batch documentation807b27b. The phased local gate completed all required workspace/core/real GUI/TUI/parity, writer/provider, isolated memory, collector and release/size checks. Failures were repaired with focused reruns; no full-suite or stress rerun per ticket. Raw commands/sources/statuses: target/perf/editor-implementation/merge-08-10/. Reports: docs/research/code-snapshot-batch-08-10.md and linked scoped reports. User explicitly accepted the documented08–10 performance costs on2026-10-24. Scope and unverified native macOS/physical display/GPU/external-provider limits remain explicit; editing is still gated.
