# 38: Show automatic and manual completion safely

**What to build:** Receive and accept current-version symbol/member suggestions through a usable popup in both adapters.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Automatic/manual invocation and an automatic-popup toggle work with real provider responses while ordinary editing stays responsive.
- [x] Tab/Enter accept, Escape dismisses and punctuation types without acceptance; late replies cannot reopen a dismissed popup or target another document.
- [x] Validate completion edits as one transaction and expose unavailable/missing provider state; test modal precedence and narrow layouts.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 61, 62, 64, 65, 68, 74.

## Answer

Merged in 6f66b07 (branch work/editor-38-completion, 222bd59). Focused core 559 / semantics 103 / GUI 177 / TUI 120+143 gui-test, clippy clean, opt-in real rust-analyzer completion pass (untested RA version). Performance deferred to end of epic per user. Also fixed stale empty-session action list from ticket 25. Known flaky watcher-rename tests predate branch. Follow-ups in docs/editor-follow-ups.md; see docs/research/code-completion.md.
