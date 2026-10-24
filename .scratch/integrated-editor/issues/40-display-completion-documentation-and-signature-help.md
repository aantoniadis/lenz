# 40: Display completion documentation and signature help

**What to build:** Inspect suggestion documentation and current-call parameters while typing or browsing completion.

**Blocked by:** 38: Show automatic and manual completion safely.

**Status:** ready-for-agent

- [ ] Show documentation and signature/active-parameter details with usable controls in both adapters.
- [ ] Honor negotiated encodings and request/version/context identity, with bounded async work and no stale popup resurrection.
- [ ] Exercise overloaded/nested calls, Unicode, missing help and narrow modal layouts with real protocol payloads.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 63.
