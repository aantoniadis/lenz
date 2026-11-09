# Project replacement and symbol rename: validation

Validated on Linux in a fresh environment on 2026-11-09. The branch is based on
PR #50's squash merge, `b69b3b44c62b755377bd7a3bed4e666a38b9d6f5`.
Runtime fixes are in `c188313`; the subsequent commits correct and extend test
coverage. All evidence below was collected here; the previous environment's
passing runs and raw logs are not used to validate this tree.

## Integration fixes and correctness

Apply previously treated an inactive tab as still loading, even when its dirty
or undoable buffer remained retained. It could refuse a multi-file rename or
replacement after ordinary navigation. The core now validates retained buffers
and clean reload candidates through the same read-only/recovery guards as loaded
views. Only after every selected source validates does it restore the necessary
views and edit the existing buffers. It preserves tab/buffer identity, earlier
unsaved text, per-file Undo and Save When Leaving a File behavior.

Fresh checks cover:

- 17 focused core workspace-edit cases, including a real offline LSP-process
  rename round trip, unsynchronized siblings, Unicode positions, unsupported and
  overlapping edits, dirty/stale sources, expansion limits, empty/literal regex
  replacement, file filters and case/word options, selection, cancellation,
  exact per-file Undo, legacy encoding and mixed line endings on Save.
- Three actual input/render adapter replays: replacement and multi-file rename,
  popup paste/caret/Home/Delete editing, pending cancellation, exclusion/empty
  selection, preview scrolling/navigation, typing after Apply, and exact draft,
  Undo and disk results. Both appearances and narrow layouts pass; the terminal
  additionally renders at 8 × 3 cells. The replacement replay crosses PR #50's
  scratch-kind picker, tab navigation and Save When Leaving a File, then replaces
  an inactive dirty draft from the scratch tab.
- Normal production GUI replay on owned private Xvfb displays in actual light and
  dark colors, with disposable projects outside Git. It checks popup editing,
  cancellation, inclusion controls, no-selection refusal, a 360 × 280 preview
  including End/Up scrolling, selected-only Apply, and exact mixed-ending Save/Undo
  bytes. Raw and OCR-preprocessed screenshots are retained.

Test corrections wait for actual rename availability instead of capability
advertisement during initialization, create fixtures before starting watchers,
rerun a search invalidated by a preceding Save notification, reveal an off-screen
context-menu entry before clicking it, guarantee split-tab overlap geometry, and
expect the new missing-rename capability label. Existing safety assertions remain.

## Process health and validation setup

The initial snapshot contained 8 processes and no zombies. Initial unsupervised
checkout operations left one Git zombie owned by PID 1. A supervised control
reproduced detached maintenance after a fast-forward merge and collected its
orphan. A 100-commit synchronous batch requested no detached maintenance. All
subsequent settled snapshots retain that one initial PID-1 zombie; transient
children owned by running Cargo/test processes are distinguished in the raw data.

The eight supervisor lifecycle tests and Rust Trace2 fixture regression pass.
The full Python suite runs 85 tests with one macOS-only skip. PostgreSQL's upstream
500-thread concurrency executable also passes directly, with its executable hash
retained. No fork/thread-creation failure occurred. Individual Rust/Python test and benchmark
commands and the Linux CI gate ran under the child-reaping supervisor.

The image's cached tools were outside PATH. Validation uses the cached Rust
1.99.1 and rootless libraries, removes the setup script's Cargo profile overrides,
and supplies matching Clang resource headers for bindgen. An initial attempt with
the setup overrides is excluded. Missing-tool/header failures, early test failures,
an explicitly interrupted obsolete gate, and failed native OCR attempts remain in
the evidence. Dark-mode OCR needed contrast preprocessing; screenshots showed the
application's menu was present even when the initial OCR oracle missed it.
Cargo profiles, compiler flags and production dependencies were not changed.
Disposable baseline build output and inactive debug incremental caches were
cleaned after retaining executables/identities to maintain disk headroom.

## Complete local gate

`scripts/ci-local.sh` passes on `95d358f` (all final production sources). Strict
Clippy passes for both feature graphs, as do all three panic-site ratchets.
The workspace and serial memory invocations report 2,951 passed and 143 ignored;
the dedicated adapter invocations report 922 passed and 30 ignored. These are
invocation totals, including overlapping coverage, not distinct test counts.
Python reports 85 tests and one macOS-only skip. The gate separately builds both
terminal applications without GUI features and builds the normal release GUI.
Stripped `lenz` is 68,909,136 bytes against its 80,000,000-byte limit.

The gate takes 429.231 seconds including supervisor cleanup, collects 817 orphaned
descendants and needs no forced cleanup. The settled snapshot afterward contains
9 processes and the same one pre-existing Git zombie. There is no hosted test CI;
this passing local gate is the repository gate. Earlier failing gates are retained:
missing Clang headers, an off-screen context-menu test target, a split-tab overlap
precondition, and the new rename capability in an expected details label. One
obsolete gate was explicitly interrupted. The final gate includes those fixes.

## Normal-release comparison

Baseline is current main after PR #50, `b69b3b44c62b755377bd7a3bed4e666a38b9d6f5`,
built in its own checkout and target directory. Candidate production sources are
those of `95d358f`; runtime changes last occurred in `c188313`. Default production
executables contain neither `gui-test` nor performance instrumentation. Both
standalone terminal dependency graphs exclude GPUI. Rebuilding the candidate with
exactly the baseline's `cargo build --release --locked -p code-tui` command yields
the identical measured SHA-256. All builds use the repository's normal Cargo profiles.

Measurements run sequentially without compilers or test suites in the background,
on the same five-vCPU AMD EPYC Linux/glibc 2.41 environment with Rust 1.99.1.
GUI uses an owned Xvfb and the same Mesa lavapipe ICD. Application state is private
and fixtures are disposable. Sources, full executable hashes, build commands,
environment, collector hashes, raw samples and timestamps are in the evidence.
The GUI startup collection excludes one explicit warmup per build, then alternates
seven measured launches per build. GUI editing alternates three processes per
build, 20 rounds each. TUI alternates eight processes per build, 30 key samples,
five resizes including 1 × 1, and at least 20 seconds of actual scrolling each.

Values below are medians; p95 is nearest rank, with no significance claim.

| Endpoint | Main | Candidate | Samples per build |
| --- | ---: | ---: | --- |
| GUI mapped window, ms | 107.256 | 104.197 | 7 processes |
| GUI startup settled RSS, MiB | 158.691 | 156.555 | 7 processes |
| GUI edit first damage, ms | 13.047 | 13.096 | 60 actions |
| GUI Undo first damage, ms | 6.591 | 4.521 | 60 actions |
| GUI Redo first damage, ms | 5.827 | 5.571 | 60 actions |
| GUI Save exact bytes, ms | 12.852 | 9.221 | 60 actions |
| GUI editing settled RSS, MiB | 210.004 | 206.375 | 3 processes |
| GUI peak RSS, MiB | 210.004 | 210.883 | 3 processes |
| TUI first filename feedback, ms | 7.559 | 7.426 | 8 processes |
| TUI first content, ms | 20.949 | 19.385 | 8 processes |
| TUI key first bytes, ms | 0.805 | 0.806 | 240 actions |
| TUI resize first bytes, ms | 0.450 | 0.471 | 40 actions |
| TUI settled RSS, MiB | 46.146 | 51.062 | 8 processes |
| TUI peak RSS, MiB | 56.520 | 61.109 | 8 processes |

GUI mapped-window p95 is 118.858 → 144.431 ms; seven launches give little tail
resolution. GUI exact Save/Undo/Redo assertions pass on the 100,000-line Unicode /
CRLF fixture. TUI preserves every fixture byte except the explicit external-change
step, restores observable termios, and records zero idle CPU ticks and output in
every strict half-second idle window. Scrolling runs last 20.007–20.064 seconds on
main and 20.006–20.071 seconds on candidate (243–245 / 244–245 checked turns).
No speedup is claimed from the small or noisy repaint/PTY differences.

### TUI memory investigation

The first three pairs measured settled RSS 42.281 → 50.996 MiB and peak
52.309 → 61.023 MiB. Five additional identical alternating pairs reproduce both
approximately 42 MiB and 51 MiB settled states in **each** executable. Across all
eight, settled medians are 46.146 → 51.062 MiB (+10.7%), with ranges
41.414–51.645 → 42.496–51.945 MiB. Peak medians are 56.520 → 61.109 MiB (+8.1%).
Every process stays flat through its scrolling phase. The initial percentage
increases and the changed proportions between modes are retained, not hidden by
a threshold or replaced with a memory-preservation claim.

A separate four-pair diagnostic preloads an inspection thread, samples glibc
`malloc_info` at three seconds and calls `malloc_trim` at four seconds. Its estimate
of allocated arena space plus mmap blocks is 6.415–6.432 MiB on main and
6.424–6.429 MiB on candidate; roughly 32 MiB of arena space is already free.
Trim changes observed residency markedly and variably in both executables.
These are diagnostic runs with an altered allocator state, not normal-release
performance samples or an exact live-payload census. The final diagnostic cases
overlap an experimental build; no diagnostic latency comparison is made.
An initial preload-scope mistake was interrupted and retained.

Six additional uninstrumented reload-allocation trials per build use the same
100,000-line input/navigation/reload/resize sequence but only 0.1 seconds of
scrolling. Main and candidate each reproduce five high-mode and one low-mode
process; settled medians are 50.076 → 50.461 MiB, and ranges are
42.367–51.562 → 42.367–51.844 MiB. These shorter diagnostic runs are not pooled
with the eight normal 20-second stress processes. They do not reproduce the
initial difference in mode frequency. A third executable creates the cancellation
token lazily; all six trials remain in the high mode (50.359–52.012 MiB). This
experiment does not reduce RSS and is **reverted**; its patch, build log, executable
and samples are retained. Production cancellation behavior is unchanged.

This finite campaign finds no accumulating live workspace-edit state in ordinary
reading. It does not establish identical default RSS distributions: allocator
retention and its run-to-run mode remain a limitation. No deliberate increase in
live ownership, production allocator tuning, profile override or loosened guard
was accepted. The measured default RSS differences above remain visible for review.

## Sustained replacement workload

The normal-release core test executable runs workload version 2 for
120.003 seconds and completes **4,220 cycles**, each replacing 4,000 matches across
40 files. Every cycle verifies every draft, independent per-file Undo and exact
unchanged disk bytes for all 40 files. Preparation median/p95 is 2.332 / 3.551 ms;
Apply is 10.454 / 14.396 ms. Completion polling is included. This candidate-only
new-feature workload excludes GUI/TUI rendering, language-server requests and Save.

Sampled RSS peaks at 26.910 MiB; quarter medians are 22.748, 23.965, 25.176 and
26.176 MiB (last versus preceding +1.000 MiB). The probe retains per-cycle timing
and memory samples, so this includes collector storage and allocator residency.
The finite run is not leak proof. Actual duration, every sample, correctness,
source and executable SHA-256 are retained in `workspace-sustained.log` and
`workspace-probe-executable.json`.

## Evidence index and unverified coverage

All new raw evidence is local under ignored `perf-results/pr51/`; it is not
committed or substituted with the previous environment's logs.

- `health-initial.json`, `health.jsonl`, `git-batch.log`, `reap-tests.log`,
  `fixture-test-normal-profile.log`: process health and lifecycle/Trace2 checks.
- `core-workspace-5.log`, `paired-workspace-3.log`, `ci-local-6.log`,
  `ci-summary.json`, `python-collectors.log`, `pg-native-concurrency.log`:
  final correctness/gate evidence. Earlier numbered logs retain failed attempts.
- `native-dark-verified/`, `native-light-verified/`, `native-workspace.py`:
  successful production GUI results, screenshots, OCR crops and collector identity.
- `baseline-source.json`, `candidate-source.json`, `binaries.json`, `machine.json`,
  `renderer.json`, build logs and `standalone-tui-dependencies.log`: identities/configuration.
- `paired-gui-startup/`, `gui-input-{0..2}-{baseline,candidate}/`,
  `tui-{0..7}-{baseline,candidate}/`, `performance-commands.json`,
  `performance-summary.json`: raw release samples and derived summaries.
- `allocator-*`, `memory-experiment-*`, `workspace-sustained.log`:
  separate allocation investigation and checked sustained workload.

GUI startup ends at a named viewable X window, not completed paint or analysis.
GUI input ends at first XDamage, which can include background repaint; exact-byte
Save has a separate completion oracle and 2 ms polling. Neither XDamage nor PTY
bytes measure physical display latency. RSS excludes GPU allocation accounting
and does not prove ownership or leak freedom. Native macOS, Windows, Wayland,
hardware GPU/frame pacing, the optional old-Git container, prolonged external
provider workloads and an indefinitely long session are unverified. Rename uses a
real offline fixture LSP process; no latency claim is made for external providers.

The [acceptance ticket](../../.scratch/project-replace/issues/01-final-validation.md)
records the completed Linux checks and these limits. Merging and releasing remain
separate actions.
