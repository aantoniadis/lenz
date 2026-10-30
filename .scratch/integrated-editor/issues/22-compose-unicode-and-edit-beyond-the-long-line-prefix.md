# 22: Compose Unicode and edit beyond the long-line prefix

**What to build:** Use GUI input methods and both adapters’ long-line editing without truncated source access or incorrect coordinates.

**Blocked by:** 21: Enter and replace text through real GUI and terminal input.

**Status:** resolved

- [x] Preedit/cancel/commit and replacement ranges use the captured snapshot and bounded UTF-16 surrounding context, rejecting stale composition and surrogate splits.
- [x] Type, select, hit-test, undo and paste beyond character 10,000 and near the end of the Unicode fixture in split views.
- [x] Assert bounded windows and no input/render full-document materialization while preserving combining/control-character layout and responsiveness.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 7, 8, 114.
