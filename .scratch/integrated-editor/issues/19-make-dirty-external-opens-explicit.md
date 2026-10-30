# 19: Make dirty external opens explicit

**What to build:** Choose Save and Open, Open Saved Version or Cancel before opening a dirty document in another editor or application.

**Blocked by:** 14: Protect dirty tab, window and all-view closure.

**Status:** resolved

- [x] Both adapters expose the three shared choices for editor and default-app handoffs.
- [x] Launch exactly once only after confirmed Save; saved-version opening keeps draft/history and uses confirmed saved coordinates or omits them.
- [x] Cancel, failed/uncertain Save and obsolete choices launch nothing; external comparisons label their saved disk inputs.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 51, 52, 53, 54.

## Implementation claim

Merged66f6290 via70d5fb: shared choices, Written-receipt Save continuation, durable ready/take delivery, Cancel/stale/permission/close fences, saved-disk comparison labels and both adapters' modal controls. Focused final core7/actualGUI3/standaloneTUI3 and strict all3 featurelint pass; decisive GUI/TUI delivery RED retained. Report docs/research/code-external-open-decisions.md; raw target/perf/editor-implementation/19-external-open/delivery-fix/identity.json. Await ONE combined cohort gate/performance before resolution; production writing remains gated pending23.
