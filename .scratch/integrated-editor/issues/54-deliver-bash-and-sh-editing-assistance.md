# 54: Deliver Bash and sh editing assistance

**What to build:** Receive variable/function help, diagnostics and formatting appropriate to the selected shell.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** ready-for-agent

- [ ] Identify Bash versus sh from explicit language/project/source context and configured tools.
- [ ] Verify real shell-specific suggestions, diagnostic ranges and formatting without running the edited script.
- [ ] Both adapters show tool identity, apply safe edits and retain ordinary editing when tools are absent.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 88.
