# 03: Provide a vetoable native macOS termination route

**What to build:** Allow ordinary native/Dock Quit to be deferred while Lenz decides whether dirty sessions may close.

**Blocked by:** None (can start immediately).

**Status:** needs-info

- [ ] Deliver a tracked and pinned framework/platform integration that requests consent before teardown while keeping the event loop alive.
- [ ] A native test can cancel an initial Quit, keep two windows alive and permit a later approved Quit; a cleanup callback is not counted as a veto.
- [ ] Record supported targets and actual native execution; keep this ticket unresolved if required native validation is unavailable.

**Spec coverage:** user stories 37.

## Comments

Native macOS runner unavailable, confirmed by the user during implementation. Source integration may proceed, but required native acceptance remains unverified and this ticket must not be resolved without it.

Source investigation is complete on `work/editor-03-native-quit` at `c3a37cf`; research-only integration merged at `e63d1a0`. A fresh advisor confirmed retaining the pinned design and withholding a production native interceptor/framework patch until native compilation and execution are available. Root independently verified the pinned shutdown/delegate source: observers run before windows are cleared, returned futures are awaited afterward, and the native callback return cannot veto termination. This ticket waits on a native test environment; no further user answer is currently requested. Preserve the macOS editing gate while Linux/shared work proceeds.
