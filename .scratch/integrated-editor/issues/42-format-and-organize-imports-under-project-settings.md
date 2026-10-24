# 42: Format and organize imports under project settings

**What to build:** Format explicitly or opt into project-specific format-on-save, and organize imports without touching other files.

**Blocked by:** 38: Show automatic and manual completion safely; 25: Respect project editing and formatting preferences.

**Status:** ready-for-agent

- [ ] Use configured/project-provided tools and apply valid current-file results as meaningful undo transactions.
- [ ] Reject cross-file/stale output in full; explicit format-on-save participates in Save ordering and retains the draft when formatting or writing fails.
- [ ] Both adapters expose actions/status and test unchanged unrelated files, cancellation and project indentation/formatter resolution.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 69, 70, 71, 72.
