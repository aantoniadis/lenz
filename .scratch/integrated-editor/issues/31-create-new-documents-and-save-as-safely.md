# 31: Create new documents and Save As safely

**What to build:** Start an untitled document or save an existing draft to another chosen path from either adapter.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Untitled buffers have stable identity, history and dirty-close protection before a path exists.
- [x] Save As validates the destination, protects an existing target with an explicit decision and updates path/provider/watch/navigation identity only after confirmed success.
- [x] Preserve encoding choices and shared views/history; failures/uncertainty keep the original draft and do not clobber another file.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 5, 6.

## Answer

Merged in 7f1161b (branch work/editor-31-new-save-as, f08cbce; includes 38+65 tip, CommandId::ALL 137, untitled tabs never Git-badged). Focused core/GUI/TUI + clippy clean; performance deferred per user. Follow-ups in docs/editor-follow-ups.md.
