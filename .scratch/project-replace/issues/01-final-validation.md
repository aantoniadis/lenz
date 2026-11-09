# Complete replacement and rename acceptance checks
Status: resolved
Type: task

Validate the combined `feat/project-replace-and-rename` tree against PR #50's
squash merge, `b69b3b44c62b755377bd7a3bed4e666a38b9d6f5`, using normal Cargo profiles.
Required checks include process health, supervisor lifecycle and fixture Trace2
regressions, the complete local gate, actual GUI/TUI input and production GUI
replays, paired normal-release frontend measurements, and the sustained
40-file/4,000-edit apply/Undo workload with exact-byte assertions.

## Answer

Completed on Linux in a fresh environment on 2026-11-09. Final production sources
are those of `95d358f`; the runtime fix is `c188313`. The
[validation report](../../../docs/research/project-replace-rename-validation.md)
records actual results, endpoints, identities, failures and limitations. New raw
evidence is retained under ignored `perf-results/pr51/`; old-environment logs are
not used to validate the combined tree.

- Fixed Apply refusing inactive retained dirty/undoable tabs as loading. Existing
  buffers and Undo history survive; selected sources all validate before edits.
  Added retained/evicted/read-only coverage and PR #50 scratch-picker, navigation
  and Save When Leaving a File integration replays.
- Eight supervisor lifecycle cases and the Rust detached-maintenance Trace2
  regression pass. A fresh 100-commit synchronous batch requests no detach.
  PostgreSQL's upstream 500-thread native concurrency executable passes.
- Full `scripts/ci-local.sh` passes: strict Clippy for both graphs, panic ratchets,
  workspace/serial-memory tests (2,951 passed / 143 ignored across invocations),
  dedicated adapter invocations (922 passed / 30 ignored), Python (85 tests,
  one macOS-only skip), separate terminal release builds and GUI size gate.
- All 17 focused workspace-edit cases and three real GUI/TUI input/render replays
  pass. Offline LSP rename, inclusion, cancellation, stale/dirty sources,
  per-file Undo, legacy encoding/mixed-ending Save, editable popups, both
  appearances and narrow layouts are covered. Normal production GUI replays
  also pass on private Xvfb in light/dark, including a 360 × 280 preview.
- Paired production release GUI startup/edit/Undo/Redo/Save and standalone TUI
  startup/key/resize/idle/20-second scrolling checks complete with correctness
  assertions. TUI default RSS has two residency modes: eight-pair settled medians
  are 46.146 → 51.062 MiB, not a memory-preservation claim. Six further short
  reload trials reproduce five high-mode/one low-mode process in each build;
  allocator diagnostics find similar allocated heap estimates. An ineffective
  lazy-token experiment is reverted. All samples and caveats remain in the report.
- The release replacement probe completes 4,220 checked cycles in 120.003 seconds;
  preparation median/p95 2.332/3.551 ms, Apply 10.454/14.396 ms. Every draft,
  independent Undo and all 40 disk files are checked each cycle. Sampled RSS
  peaks at 26.910 MiB; last-quarter versus preceding-quarter median adds 1 MiB,
  including retained measurement samples and allocator residency.
- Process health starts at 8 processes / zero zombies. An initial unsupervised
  checkout leaves one PID-1 Git zombie; supervised validation never adds to it.
  The full gate collects 817 orphans with no forced cleanup or EAGAIN. Settled
  post-validation count is 9 processes / the same one zombie.

Native macOS/Windows/Wayland, hardware display/GPU measurements, the optional
old-Git container and prolonged external-provider sessions remain unverified.
Finite RSS observations do not prove leak freedom or identical distributions.
No performance tradeoff, profile override or relaxed assertion was accepted.
Merging and releasing are outside this task.

## Previous environment

The previous environment accumulated about 32,000 zombies and could not finish
validation. Detached Git maintenance was reproduced as one source; fixtures now
request synchronous maintenance and Linux local CI runs under the child-reaping
supervisor. Earlier lifecycle and synchronous-commit results did not validate
this rebased tree. The old raw logs remain only in that environment, including
an earlier unpassed PostgreSQL pthread test; the new native run passes here.
