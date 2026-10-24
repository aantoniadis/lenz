# 43: Explain active tools, environments and missing capabilities

**What to build:** See which language environment/tool is active and why particular assistance is unavailable.

**Blocked by:** 25: Respect project editing and formatting preferences.

**Status:** ready-for-agent

- [ ] Expose configured executable/environment/project, readiness, missing capabilities and clear setup guidance through both adapters.
- [ ] Discover installed/project-provided tools lazily without automatic installation or unrelated project changes.
- [ ] Keep editing available while tools fail/index and keep source text/credentials out of logs or shareable configuration.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 42, 73, 74.
