# 13: Save drafts explicitly and preserve uncertain outcomes

**What to build:** Save the current file from either adapter while retaining undo history and explaining rejection or uncertainty.

**Blocked by:** 12: Edit one shared draft and history across views; 02: Share exact-ending writes with explicit save outcomes.

**Status:** resolved

- [x] Both adapters expose Save, dirty/read-only/busy/conflict/uncertain state and a recheck path through shared commands.
- [x] A delayed Save of an older version advances only that baseline, keeps newer text dirty, and cannot resume an obsolete action.
- [x] Fault cases retain draft/history/confirmed baseline, block silent retry and prevent any launch that requires confirmed success.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 23, 24, 25, 26, 27, 28, 29, 30.

## Comments

After saving and replacing the baseline, verify old original text/index backing allocations can be reclaimed once history no longer needs them; account any surviving shared source slices. If the admitted text format already has a derived image preview, it must correspond to current text; preserve existing image read-only admission otherwise.

## Implementation handoff

Clean902426c mergeda5c84d2. ExactRead same-descriptor provenance covers promotable file paths; captured-version Save has durable request-owned outcomes outside read/project fences, shared64/6 credits and explicitrecheck/Confirm. History/dirty/newertext preserved, backingretireswithsameconversioncell andoldcapturedweak fallback. Namedpathafterrename/substitutioncannotwronglywrite/cleancurrenttarget; explicitcurrentpathrecheckrequired. FocusedcoreSave15/Analysis18/writer8/realGUI2+TUI2 andstrictaffectedClippygreen; rawtarget/perf/editor-implementation/13, docs/research/code-explicit-save.md. Finalfullgate/performance/stressacceptancependingbatch11/12/13/16/18.16newnavigationworker mustusedecode_readaftermerging13;owner10notified.


## Resolution — 2026-10-24

Resolved against the phased full local correctness gate at runtimebb2f55f4, actualGUI/TUI/core regressions, equivalent release measurements and retained stress evidence. The user explicitly accepted this batch’s documented costs and requested continued implementation. Evidence: docs/research/code-editor-batch-11-18.md, code-editor-batch-11-18-performance.md and target/perf/editor-implementation/merge-11-18/. NativeMac and the report’s other limits remain unverified; latercohort acceptance is separate.
