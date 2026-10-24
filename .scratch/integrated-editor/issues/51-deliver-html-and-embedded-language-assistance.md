# 51: Deliver HTML and embedded-language assistance

**What to build:** Complete tags/attributes, close tags and format markup with appropriate embedded-language help.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Verify actual HTML completion replacement/closing-tag behavior and configured formatting.
- [ ] Exercise supported script/style embedding and correct original-source ranges with real provider messages.
- [ ] Both adapters accept/undo edits safely and explain missing embedded capabilities.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 83.
