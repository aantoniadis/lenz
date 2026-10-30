# Native macOS termination prerequisite

Investigated 2026-10-23 for integrated-editor tickets 03 and 24. **Unresolved:**
no native macOS runner is available. This note supplies a pinned integration
boundary and acceptance procedure; it adds no termination hook or editor support.

## Current boundary

The workspace pins GPUI and `gpui_platform` to Zed tag `v1.24.2`, resolved to
`fb69067729cb16a7f6135e2c1600c26a28c59659`. Inspected source:

- [Native delegate registration](https://github.com/zed-industries/zed/blob/fb69067729cb16a7f6135e2c1600c26a28c59659/crates/gpui_macos/src/platform.rs#L87)
  registers `applicationWillTerminate:`, with no `applicationShouldTerminate:`.
- [MacPlatform::quit](https://github.com/zed-industries/zed/blob/fb69067729cb16a7f6135e2c1600c26a28c59659/crates/gpui_macos/src/platform.rs#L594)
  asynchronously sends `terminate:` so application-state borrows have unwound.
- [Native termination callback](https://github.com/zed-industries/zed/blob/fb69067729cb16a7f6135e2c1600c26a28c59659/crates/gpui_macos/src/platform.rs#L1472)
  invokes the platform quit callback during the termination notification. Its
  Boolean return is ignored there; it is not a consent result.
- [App::shutdown](https://github.com/zed-industries/zed/blob/fb69067729cb16a7f6135e2c1600c26a28c59659/crates/gpui/src/app.rs#L1084)
  invokes cleanup observers, clears windows, then waits for their futures.
  [App::on_app_quit](https://github.com/zed-industries/zed/blob/fb69067729cb16a7f6135e2c1600c26a28c59659/crates/gpui/src/app.rs#L2592)
  explicitly documents that cancellation is unavailable.

The Reader's [application startup and quit observer](../../crates/code-gui/src/lib.rs)
therefore cannot protect drafts from ordinary native Quit using that observer.
Individual-window close callbacks do not intercept application termination.

Apple documents that returning `NSTerminateCancel` from
[`applicationShouldTerminate:`](https://developer.apple.com/documentation/appkit/nsapplicationdelegate/applicationshouldterminate(_:))
returns control to the main event loop. Returning
[`NSTerminateLater`](https://developer.apple.com/documentation/appkit/nsapplication/terminatereply/terminatelater)
instead enters a modal run-loop mode until a reply. A custom GPUI confirmation
must not assume its normal asynchronous work continues in that modal mode.

## Candidate integration for native review

Prefer a tracked, pinned GPUI change exposing a pre-termination request callback,
with the macOS delegate implementing `applicationShouldTerminate:`. Initially
cancel the native request, queue the application's shared close decision on the
foreground executor, and keep GPUI's existing teardown untouched. Cancellation,
failed or uncertain saves must leave every participating window alive. Only an
approved, still-current close batch may arm one subsequent native termination;
consume that approval once. Repeated requests must not create competing batches.
The callback must not call app mutation while a GPUI borrow is held.

This is a candidate, not a selected dependency patch. A framework fork needs an
owned repository and immutable revision; a local Cargo-checkout edit is not a
reproducible integration. A runtime method replacement or delegate proxy would
couple Lenz to Objective-C ABI, delegate ownership and launch ordering without a
supported GPUI seam. Neither approach is installed by this change. Native
compilation and execution are required before choosing and shipping one.

## Native acceptance harness contract

Use an owned macOS account/display and disposable project. Record OS, architecture,
source/dependency revisions, executable hash, event transcript and actual duration.
Drive the real application with two windows containing independently dirty files:

1. Request Quit through the Dock, cancel the shared close decision, then type and
   navigate in both original windows. Assert both window identities and drafts
   survive, file bytes are unchanged, and background completions still arrive.
2. Repeat through the application menu and the normal keyboard Quit action.
   Repeated requests while a decision is pending must leave one active decision.
3. Fail a Save using a controlled filesystem fault. Both drafts and windows must
   remain usable; the disk outcome must match the shared writer result. Exercise
   uncertain outcomes separately, without treating uncertainty as permission.
4. Start another Quit, approve the shared close batch, and observe ordinary native
   teardown and successful exit. Verify final bytes and cleanup exactly once.
5. Repeat from an empty window and with a window added or edited during a pending
   batch; stale approval must not authorize the changed set of drafts.

A Linux state-machine replay or mocked cleanup callback cannot establish this
native result. Dock integration, input methods, native rendering and performance
remain unverified. Forced termination remains outside ordinary Quit acceptance.
