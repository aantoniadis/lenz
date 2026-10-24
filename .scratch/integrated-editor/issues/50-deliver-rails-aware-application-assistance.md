# 50: Deliver Rails-aware application assistance

**What to build:** Receive Rails-specific helpers and application context while editing Ruby source.

**Blocked by:** 49: Deliver Ruby project and gem-aware assistance.

**Status:** ready-for-agent

- [ ] Verify actual Rails/add-on/project context with helper/model/controller fixtures rather than inferring Rails support from Ruby startup.
- [ ] Exercise completion/signatures, supported diagnostics/formatting and current-file changes through both adapters.
- [ ] Missing Rails dependencies or runtime evidence keep this ticket open; ordinary editing remains available.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 78.
