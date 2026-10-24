# 67: Inspect and revert marked changes in the shared draft

**What to build:** Inspect prior content, open the editable comparison and revert a marked change with undo.

**Blocked by:** 66: Mark draft changes against HEAD or the last save.

**Status:** ready-for-agent

- [ ] Provide marker and keyboard/menu access to prior content, Show Diff and Revert This Change in both adapters.
- [ ] Label HEAD versus last-save baseline and share the existing draft/history in the comparison.
- [ ] Revert changes the draft as one undoable action, writes only through Save, leaves the Git index unchanged and rejects obsolete baseline/hunk coordinates.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 107, 108, 109, 110.
