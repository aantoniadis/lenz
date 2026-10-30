# 18: Attribute the displayed draft correctly

**What to build:** Show blame for current text and disable commit actions while that attribution is stale or pending.

**Blocked by:** 12: Edit one shared draft and history across views.

**Status:** resolved

- [x] Insertion/deletion/undo immediately invalidates old annotations; recomputation uses admitted draft bytes, existing Git cancellation and ignore-revs behavior.
- [x] Changed/untracked lines are Not committed while unchanged lines retain correct commits; stale batches and failures cannot revive old actions.
- [x] Both adapters display pending/failure state and exact current-line attribution without writing files.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 49, 50.

## Implementation handoff

Cleanfe64e06 merged87f53df. Current draft bytes drive Git blame through admitted sharing; edits/Undo/rename invalidate old annotations/actions; version/meta/stamp/project fences reject stale batches; GUI/TUI pending/failure/current-line traces pass. Representable1252 commits preserved via shared strictencoder2b31dab; unencodable/lossy reports Failed with actions disabled, limitation recorded.22focusedcorecases covered acrossruns; one existingUTCmidnightdatefixturefalsefailure rerunexactfreshpasswithoriginalassertion preserved, cleanupdeferredMD. Newrealadapter edit/newlinedelete/Undo/failure, existingcolumn/menuparity andaffectedstrictClippygreen. Raw target/perf/editor-implementation/18-blame. Final fullgate/performance/stress acceptance pendingbatch11/12/13/16/18.


## Resolution — 2026-10-24

Resolved against the phased full local correctness gate at runtimebb2f55f4, actualGUI/TUI/core regressions, equivalent release measurements and retained stress evidence. The user explicitly accepted this batch’s documented costs and requested continued implementation. Evidence: docs/research/code-editor-batch-11-18.md, code-editor-batch-11-18-performance.md and target/perf/editor-implementation/merge-11-18/. NativeMac and the report’s other limits remain unverified; latercohort acceptance is separate.
