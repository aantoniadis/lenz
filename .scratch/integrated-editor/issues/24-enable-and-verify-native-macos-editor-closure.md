# 24: Enable and verify native macOS editor closure

**What to build:** Use the editor on native macOS with real input-method and normal native/Dock Quit protection.

**Blocked by:** 23: Verify and enable the foundation on validated targets; 03: Provide a vetoable native macOS termination route.

**Status:** ready-for-agent

- [ ] Wire the vetoable termination route into the shared all-session batch before shutdown.
- [ ] With two dirty windows, approve one then cancel the other and verify both survive; repeat for save failure/uncertainty, permit invalidation and successful final termination.
- [ ] Run native input/composition, lifecycle and relevant performance checks; keep platform acceptance open if hardware execution is unavailable.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 8, 37.
