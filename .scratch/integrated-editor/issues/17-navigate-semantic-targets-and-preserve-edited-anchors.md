# 17: Navigate semantic targets and preserve edited anchors

**What to build:** Keep definitions, usages, peek and multi-result navigation correct when the source or a different target has unsaved changes.

**Blocked by:** 16: Search current drafts and validate result snapshots.

**Status:** resolved

- [x] Synchronize providers to admitted versions and validate target identity independently from the origin request, including A navigating to dirty B.
- [x] Re-request obsolete target coordinates or leave selection unchanged with a clear notice; never reopen disk text over a draft.
- [x] Transform selections/bookmarks/history anchors with defined bias and invalidate/recompute stale folds, find and expansion state in both adapters.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 45, 46, 47, 48.
