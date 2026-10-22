# Lenz agent instructions

## GUI/TUI behavior parity

Lenz has GUI and TUI adapters over shared application behavior. Both expose
comparison, repository/PR and merge workflows. Before frontend or shared behavior
work, read [current coverage and capability limits](docs/tui.md).

- Put commands, application state transitions, action availability, validation,
  file-write safety, undo, and background-work orchestration in the shared core.
  Sharing only the diff engine is insufficient. Keep core state mutations behind
  its command interface so frontends cannot implement competing behavior.
- Frontends translate input and present core state. Keep pixel/cell layout,
  hit-testing, textures, and terminal resources in their respective adapters.
  Equivalent operations and outcomes must be accessible in both; layouts and
  shortcuts may differ. Document terminal capability limits and any parity gaps.
- Define semantic commands, labels, availability, and disabled reasons centrally
  and expose them in both command menus. Use exhaustive handling where an adapter
  must interpret a shared action; a catch-all must not hide missing support.
- Implement behavioral features and fixes once in the core. Add the necessary
  controls and presentation in both adapters; a shared command alone does not
  establish usable feature parity. Rendering/input bugs need adapter-specific fixes.
- Keep applied changes visible as Applied or Undone, with safe selective undo,
  including after repository refresh. Show folding controls only for multiline
  changes. Independent views retain their own documents until closed; they do
  not synchronize live edits.

## Adapter robustness and resources

- Route input to the active modal before parent menus, toolbars, save actions,
  and nested workflows. Test that underlying documents and file bytes stay
  unchanged. Preserve only explicitly supported global actions through a modal.
- Validate indices against current collections at use time. When switching files
  or refreshing lists, reset dependent selection/search state, cancel obsolete
  work, and reject stale worker results using the current request identity.
- Exercise actual constructors and empty states, not only test-built state.
  Empty standalone windows must support render, Undo, Reload, and dialog dismissal
  without indexing a nonexistent comparison or remaining in a loading state.
- Clamp owned scroll state to the current layout on input, render, and resize.
  Wrap and scroll dialog content in narrow windows; test tiny terminal sizes,
  End followed by Up, and dismissal from a scrolled dialog.
- Load optional artwork only when its view opens. Release both CPU ownership
  and renderer/atlas resources when it closes; verify repeated opening reaches
  a stable memory range. Keep application identity metadata shared and compile-time,
  and asset exports precomputed outside runtime paths. Before changing About or logo handling, read the
  [About logo findings](docs/optimization-notebook.md#about-logo-and-linux-identity--2026-10-04).

## Parity verification

- Test core workflows, then test each adapter's real input-to-command mapping and
  observable presentation. Core tests alone cannot detect incorrectly wired controls.
- Replay common scenarios through both adapters and compare semantic state and
  file output, including apply/undo, save/reload, file switching, and cancellation
  where relevant. Keep platform-specific rendering checks separate.
- Report missing coverage in either adapter explicitly. A feature is complete
  only with usable support in both, passing relevant core/adapter checks, and
  performance validation. Compilation and tests provide evidence of parity,
  not a guarantee that every visual or interaction bug is absent.

There is no hosted test CI: `scripts/ci-local.sh` is the gate (the pre-push hook runs it). A pushed version tag
builds the release tarballs on GitHub Actions (`.github/workflows/release.yml`).
For parity changes, run workspace/core tests, the actual GUI adapter tests
(`cargo test -p lenz --features gui-test gui_`), and relevant TUI event/render
checks. Cover supported appearances and narrow layouts when presentation changes.
Use disposable fixtures and an owned private display for automated GUI input;
keep the user's active desktop and repositories outside stress workloads.

## Preserve performance

- Preserve both frontends' first-paint time, responsiveness, and memory use.
  Use direct in-process Rust calls and efficient borrowed or shared data; keep
  frontend dependencies separate. Preserve bounded caches,
  cancellation, worker reuse, lazy work, compact undo, and viewport-limited work.
  Avoid full-document copies or recomputation on each interaction.
- Before changing architecture or performance-sensitive paths, read
  [the benchmark guide](docs/benchmarking.md) and
  [the optimization notebook](docs/optimization-notebook.md). Save a baseline and
  compare equivalent release workloads on the same machine and configuration.
- Measure relevant startup, interaction, peak/settled memory, and sustained stress
  workloads after performance-sensitive changes. Investigate regressions using
  repeated samples; fix reproducible regressions before accepting the change. Any deliberate
  performance tradeoff requires explicit user agreement; passing a loose ceiling
  alone does not establish preservation of the baseline.
- Maintain deterministic correctness assertions alongside timing, rendering,
  resize, cancellation, and memory stress workloads. When changing collectors,
  run their Python tests; version changed metric semantics as the guide requires.
  Keep benchmark instrumentation and `gui-test` out of production executables,
  and build the TUI separately so its measurements exclude GUI dependencies.
- Retain raw samples, configuration, source/executable identity, and actual stress
  duration. Distinguish first feedback from completed actions and display latency,
  and RSS from GPU memory or leak proof. Report skipped workloads and untested
  platforms, including native macOS, explicitly as unverified.
- After optimization work, update the README's metrics and the notebook with
  successful, unsuccessful, and inconclusive experiments and their evidence.
  Keep detailed measurements in performance reports rather than this file.

For documentation-only changes, verify local links and `git diff --check`;
runtime tests and benchmark reruns are unnecessary unless executable behavior
or benchmark definitions also change.
