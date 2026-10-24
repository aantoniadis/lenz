# 39: Apply completion imports and snippet edits atomically

**What to build:** Accept completion with supported imports and placeholders as one coherent edit.

**Blocked by:** 38: Show automatic and manual completion safely; 30: Insert snippets and navigate placeholders.

**Status:** ready-for-agent

- [ ] Handle provider replacement/insert ranges, resolved additional edits and snippet text against one captured document version.
- [ ] Reject stale, malformed, overlapping or other-file edits as a whole transaction with an explanation, including server-initiated attempts.
- [ ] One undo restores all related edits and imports; test compatible/incompatible multi-cursor contexts and real GUI/TUI acceptance.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 16, 67, 70.
