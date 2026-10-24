# Foundation acceptance tools

These tools support editor ticket23; they do not enable production input. The native Linux-X11
correctness supplement has passed. The cohort performance campaign, 180-second editing workload and
combined full gate remain pending.
Native macOS remains blocked by03/24; prior11–18 performance costs still await an explicit decision.

## Shared-core editing/history workload

`crates/reader-core/examples/foundation_edit.rs` uses the common12/13 public Session Edit/Undo/Redo/Save
APIs, so the same source can be compiled against the measured11–18 baseline and the new candidate.
For a historical worktree, copy this example only as a recorded tooling overlay, retain its SHA, and
build in that worktree's own target. Do not label the overlay as an application source change.

After explicit campaign release, compile an ordinary release example, then invoke the PREBUILT executable:

```sh
cargo build --release --locked -p reader-core --example foundation_edit
target/release/examples/foundation_edit <NEW-output> <readiness-fixtures> 180 <source-SHA>
```

Fixtures are the existing `target/perf/editor-readiness/fixtures`:2000-line Rust (`rust/module_0.rs`),
100000-line text (`large.txt`) and900001-byte Unicode line (`long-unicode.txt`). The harness verifies
sizes, copies them into the new output and edits only its copies. Each mixed cycle visits every case,
performs boundary/distant Unicode edits/paste, periodically Undo/Redo and Save with a newer edit,
checks split buffer sharing, and checks a second independent dirty Session. Syntax analysis is active;
external language providers are disabled. Final complete Undo/Redo verifies every independently counted
transaction remains available. There is no history cap and no reset-to-original between every cycle.

`samples.jsonl` schema1 streams raw dispatch/Save samples, exact workload/fixture/executable hashes,
transaction and byte counts, current storage, RSS/HWM and actual editing duration. Full history validation
time is separate. RSS includes deterministic oracle strings and legitimate history; it is not a core-only
allocation count or a leak verdict. This is shared-command timing, not GUI typing/IME or display latency.
The180-second minimum is enforced for the CLI. A fixed two-cycle, three-small-file unit tracer validates
the harness without running a timed/stress campaign.

## Native Linux correctness example and driver

`foundation-native` is a Reader GUI example requiring `test-support`. Its constructor helper delegates
exactly to the installed production `open_reader_window`, including close coordination, focus callbacks
and veto hook. Opening through the helper does not enable input; only this disposable example explicitly
calls the existing test opt-in. It also explicitly starts the same completion/theme listeners used by
production; TestPlatform continues to poll manually because it rejects cross-thread wakeups. No shipped
CLI/config/defaults or normal input gates change.

After cohort release:

```sh
cargo build --release --locked -p reader-gui --features test-support --example foundation-native
python3 scripts/reader-foundation-native.py --binary target/release/examples/foundation-native \
  --out <NEW-output> --source <source-SHA> --scenario both
```

The driver owns a new Xvfb display, isolated XDG/runtime/shader paths and fixed disposable files. It
preserves HOME, removes user Wayland/D-Bus routes, and never accesses the user's display. Xlib/XTest
input edits two independent native windows. It sends a real `WM_PROTOCOLS` ClientMessage containing
`WM_DELETE_WINDOW`, verifies native veto/Cancel retains both drafts/positions/files, then verifies a
fresh explicit Discard removes only the requested window. It does not use XDestroyWindow/XKillClient.
Surviving fixture drafts are explicitly discarded for normal last-window cleanup; failed runs reap
only owned children and retain their logs.

XKB compose maps one unused key to dead_acute on ONLY the private server before app startup. A native
dead_acute+e sequence proves preedit leaves authority/history unchanged. Two consecutive compositions
commit `éé`, then Undo restores `é` and the original draft in separate steps. The example records bounded (≤4096-byte) authoritative prefixes and state changes;
small fixture text is checked exactly. Screenshots and native window IDs supplement those observations.
This observer is acceptance instrumentation, not a production typing latency endpoint.

The positive native preedit oracle depends on ticket22's read-only
`ReaderView::marked_text_for_test()->Option<(absolute UTF16 range, bounded preedit String)>`.
Ticket22 is integrated and that getter supplies the example's `marked` state. A missing oracle fails
the scenario; it never counts as successful preedit.
`--scenario close` is independent. Native composition cancellation is not exercised: the pinned XKB
Cancelled branch may commit a dead-key prefix, so physical Escape must not be assumed to cancel.
Coordinate the native cancellation oracle with22 before any stronger input-method claim.

`result.json` schema1 retains binary/driver/source identity, exact assertions, failures, owned process/
window IDs and elapsed duration. It establishes X11 protocol/XKB delivery only when actually executed;
not physical titlebar input, Wayland, macOS/Dock behavior, real engine coverage or physical display latency.

## Focused tooling self-checks

```sh
cargo test -p reader-core --example foundation_edit
cargo test -p reader-gui --features test-support --example foundation-native
cargo clippy -p reader-core --example foundation_edit -- -D warnings
cargo clippy -p reader-gui --features test-support --example foundation-native -- -D warnings
python3 -m unittest discover -s scripts -p test_reader_foundation_native.py
```

The constructor test uses TestPlatform and is deliberately not the native delivery proof. Its current
gate-off assertion pins preparation safety; only the later validated-target enablement may change that
expectation. The Python tests are pure protocol/environment/state-parser checks; they start no server,
application or timed workload. All production and cohort acceptance remains separate.

Collector revision for this cohort: `code-tui-bench.py` schema3 dismisses the palette with Escape before Ctrl-Q, matching the shared modal-first close contract. Its latency endpoints remain first observed PTY output; environment revision2 preserves HOME while isolating XDG. The archived startup-pair wrapper advances schema1→2 for the same environment change. Historical drivers and samples remain untouched; staged driver hashes are recorded under `merge-14-23/performance/staged-collector-identity.json`.

The native observer now records22's real bounded marked-text metadata. The supplement requires two consecutive dead-key compositions with no intervening Undo/navigation, exact `éé`, then one Undo per commit. These assertions pass on owned Xvfb/X11 with actual XTest input and production window close callbacks; physical display, Wayland, native macOS and native cancellation remain unverified. The corrected collector has focused pure/owned-PTY checks only; no cohort timings or stress have run.

The whole-chain replay is `reader-tui/src/foundation_parity_tests.rs`: both actual production constructors start empty, render dark/light and1x1 modal states, then actual Open/typing/paste/split/both-pane edit/Undo/Redo/Save controls run. Two17MiB clean sources force observable weak-cache eviction before returning to the saved buffer and undoing. Exact draft/disk/history/dirty/action state, shared split snapshot, independent pane positions and independent dirty Sessions are asserted throughout. Each adapter owns mirrored disposable files so their explicit Saves do not race each other. This focused TestPlatform replay is distinct from native OS delivery and from performance acceptance.


The completed native supplement uses application source `2ad66f6` and driver source `3fa80f0`.
It passes real WM_DELETE veto, repeated requests and modal input isolation, Cancel preserving two exact
independent drafts/positions/files, two positive marked phases with independent Unicode Undo, fresh
Discard removing only its approved window, and normal last-window process exit. Raw evidence is under
`target/perf/editor-implementation/merge-14-23/performance/native-preflight-complete/`.

Both failed attempts remain alongside it. The first stopped before document loading: the test-support
build intentionally omitted production receivers until the native example explicitly started them.
The second passed every semantic assertion but failed cleanup when a window disappeared between X11
enumeration and attribute lookup. The driver now ignores only that BadWindow race; a focused pure
regression checks both disappearance points and propagation of other errors. Five pure helper checks,
the existing constructor check and strict GUI default/test-support lint pass. These corrections add no
polling loop or executor. No stress or timing workload was repeated for them.
