# 56: Edit Markdown with live preview and authoring help

**What to build:** Write Markdown source with a live preview, list continuation and heading/file-link assistance.

**Blocked by:** 38: Show automatic and manual completion safely; 27: Indent, pair, comment, move and duplicate lines.

**Status:** ready-for-agent

- [ ] Preview updates from admitted unsaved versions, navigation maps back to current source and obsolete render results are rejected.
- [ ] List continuation and heading/file-link completion work as undoable edits with both adapters’ actual controls.
- [ ] Preserve existing Markdown rendering/navigation and narrow-layout behavior; rendered content itself remains read-only.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 85, 86.
