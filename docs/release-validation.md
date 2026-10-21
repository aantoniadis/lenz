# Release validation — 2026-10-21

The final Linux automated checks passed for the working tree based on
`c0e50c268f6c9377de1dce7044e5a6540634b367`, still version **0.0.2**. This validates
the pending Reader/LSP, shared scrolling, menu, Markdown and Libraries changes;
it does not certify a packaged release or native macOS behavior.

## Checks and collector corrections

`scripts/ci-local.sh` passed: workspace lint/tests, memory assertions, actual
Diff and Reader GUI adapter tests, Reader GUI/TUI parity tests, collector tests,
separate terminal release builds and the production Diff build. Stripped Diff
size was 68,365,912 bytes, below the 80,000,000-byte gate.
`scripts/perf-check.sh` passed. The full performance suite passed **133 cases and
394 metrics, with no skipped cases**, using three repetitions and 60-second
mixed stress. Its elapsed time was 1,012.95 seconds.

Two failures were in collectors, before application behavior could be judged:

- Xvfb writes its display number and newline separately. Closing the pipe after
  one read intermittently killed Xvfb with a broken pipe: 8 failures in 150
  isolated launches. The five collectors now share a bounded, deadline-limited
  reader that waits for the complete line. Regression tests cover fragmented,
  invalid, incomplete and stalled writes. All 150 repeat launches passed without
  retries; the affected image scenario and complete suite then passed. Timing
  endpoints are unchanged, so the full-suite workload version remains 6.
- The Reader TUI collector waited for a removed `Lenz Reader` label after the
  file had already opened. It now records filename feedback as
  `first_feedback_ms`, with **schema 2**. `first_content_ms` still waits for the
  known source line. Old schema-1 `first_paint_ms` is not comparable to the new
  feedback metric. The real 100,000-line workflow passed after this correction.

The final Python collector run completed 65 tests, with one platform skip.
Application Rust sources and Cargo files were unchanged during this validation.
Original failed attempts, fixes and successful reruns are retained together.

## Same-run release comparisons

Measurements used sequential alternating runs on the same shared Linux x86_64
host, Rust 1.99.1, 32 available logical CPUs, private Xvfb displays and Mesa
lavapipe. User configuration and repositories were isolated. Background host
load was not controlled; old reports are not equivalent baselines.

Reader compares the saved pre-Libraries build with the current build, six pairs
after an excluded warmup. GUI input uses 20,000 Markdown lines; TUI uses 100,000.
Diff compares the saved pre-shared-scroll build, three pairs plus warmups. These
baselines include earlier pending work and are **not pristine 0.0.2 releases**.

| Median measurement | Before | Current |
|---|---:|---:|
| Reader GUI first captured nonuniform client frame | 158.13 ms | 159.41 ms |
| Reader GUI wheel to changed captured image | 26.00 ms | 26.01 ms |
| Reader GUI sampled RSS | 171.01 MiB | 171.16 MiB |
| Reader TUI first source bytes | 6.66 ms | 6.74 ms |
| Reader TUI arrow feedback | 0.459 ms | 0.494 ms |
| Diff 100k first paint | 210 ms | 210 ms |
| Merge 100k first paint | 398 ms | 402 ms |
| 4K image comparison first paint | 210 ms | 213 ms |

Diff sampled RSS was 261.3→263.8 MiB for text, 319.6→322.0 MiB for merge and
675.5→676.3 MiB for images; observed peaks were 475.2→474.9, 824.3→824.0 and
675.5→676.3 MiB respectively. These pairs did not reproduce the larger startup
differences seen when comparing against historical runs. The earlier roughly
13 ms Reader GUI startup difference also did not recur in the six fresh pairs.

Separate instrumented Reader runs measured 48.000→47.999 ms median frame
interval during real Discourse/Ruby startup, two pairs. This is software-display
cadence, not native trackpad performance. Production GUI binaries were rebuilt
without the probe and their hashes checked against the saved production copies.

## Rails indexing and checkout recovery

The owned Discourse fixture contained 12,082 local-index candidates. Ruby LSP
0.26.11 startup used three alternating cold/warm pairs, plus cache seeds.
Checkout checks used three pairs, each exercising two A→B→A cycles, with precise
watcher events suppressed and directory/overflow events injected. Timings begin
after checkout and end after exact symbol/definition assertions. The saved
baseline predates the overflow optimization.

| Median elapsed time | Before | Current |
|---|---:|---:|
| Cold local declarations | 8.36 s | 8.61 s |
| Warm saved local declarations | 0.544 s | 0.549 s |
| Cold Ruby LSP Ready | 12.93 s | 12.91 s |
| Warm Ruby LSP Ready | 12.42 s | 12.44 s |
| Recovery after lost watcher events | 6.79 s | 0.95 s |
| Directory-event recovery | 0.607 s | 0.547 s |
| Unchanged overflow reconciliation | 6.61 s | 0.91 s |
| Explicit index rebuild | 12.72 s | 12.40 s |
| Overflow recovery with reuse disabled | 10.12 s | 10.09 s |

Overflow recovery was about **7.2× faster**. Removed-file, exact-definition,
rebuild and opt-out assertions passed, as did a 30-second server-reuse loop per
build. Warm local reuse does not remove Ruby's roughly 12–13 seconds of initial
indexing. Current sampled core/server peak summed RSS was 690 MiB cold, 686 MiB
warm and 1,040 MiB over the complete checkout/rebuild scenario, compared with
674, 691 and 1,059 MiB respectively in the saved baseline.

Nine generated language fixtures, each with 1,202 candidates, used three samples
per mode plus a seed. These are local declaration timings, not full server
indexing measurements:

| Language | Reuse off | Cold cache creation | Warm reuse |
|---|---:|---:|---:|
| Rust | 178 ms | 189 ms | 24 ms |
| Python | 162 ms | 169 ms | 25 ms |
| Java | 191 ms | 196 ms | 24 ms |
| Kotlin | 404 ms | 404 ms | 26 ms |
| JavaScript | 177 ms | 185 ms | 25 ms |
| TypeScript | 234 ms | 239 ms | 25 ms |
| TSX | 234 ms | 237 ms | 26 ms |
| HTML | 170 ms | 176 ms | 24 ms |
| CSS | 170 ms | 182 ms | 24 ms |

All ten live-provider lifecycle checks passed: these nine languages and an
additional TypeScript 6 run alongside the default TSGo. Checks include exact
definitions after coarse changes, rebuild, opt-out and unchanged source bytes.
Cold cache creation has a small cost; warm reuse is the benefit. Generated
fixtures do not establish performance on every real dependency graph.

## Stress, Libraries and terminal behavior

Reader GUI wheel stress lasted 60.015/60.032 seconds before/after, with 988/1,014
cycles. Sampled RSS rose about 0.23 MiB within each run; final-quarter medians
were 171.01/171.70 MiB. TUI menu stress lasted 60.081/60.016 seconds, with 725/724
cycles and constant final-quarter RSS at 15.86/15.81 MiB. The earlier small TUI
memory excess did not recur in this pair.

Shared-core stress exercised 16,406 copied files for three minutes: 32,380
iterations and 647 external rewrites, with ten RSS windows after a 60-second
warmup. Windows ranged from 56.20 to 57.04 MiB and ended at 56.30 MiB; the memory
guard and source/cache assertions passed. Servers were disabled in this test.

Library core reveal took 0.844 ms for 1,000 files, leaving an unopened 10,000-file
subtree unread. Over 10.000 seconds, 22,537 collapse/expand cycles had a 0.417 ms
median and 0.597 ms p95, with bounded cached rows. This excludes LSP, worker
queueing and painting. Live Ruby navigation into rake 13.3.0 passed in both
frontends, with dark/light and narrow GUI screenshots, unchanged source bytes
and restored terminal state.

The schema-2 Reader TUI workflow passed all nine behavioral checks, including
modal isolation, copy, navigation, independent views, reload, 1×1 resizing and
terminal restoration. First feedback/content were 4.10/6.72 ms. A 60.036-second
run completed 739 page cycles, with 36.12 MiB settled and 39.23 MiB peak RSS.
The settled half-second idle sample emitted no bytes and consumed no sampled
CPU ticks. Collector pacing means cycle counts are not frame rates.

## Evidence, reproduction and limits

Raw evidence is retained locally under `target/perf/release-check-20261021/`:
`final-summary.json`, source manifests, working-tree/collector patches,
`baseline-identities/`, hashed `before/` and `after/` binaries, configurations,
drivers, failed attempts, screenshots and unabridged logs. `full-confirmed/`
contains the full suite; `languages/`, `scroll-indexing/`, Rails sample files and
`terminal-workflow-confirmed/` retain the specialized checks. Each command and
duration is recorded in its wrapper JSON. Reproduce the standard gates with:

```sh
bash scripts/ci-local.sh
bash scripts/perf-check.sh
python3 scripts/perf-suite.py --profile full --runs 3 --stress-seconds 60 \
  --require-ui --check --renderer-label 'Xvfb Mesa lavapipe software rendering' \
  --output target/perf/release-recheck
```

Captured GUI frames can contain chrome before content and include capture/poll
overhead. PTY timings measure emitted bytes. Neither measures physical display
latency. RSS is not GPU memory, PSS or proof against leaks. Short repetitions and
shared-host variation limit conclusions about small differences.

Native macOS scrolling/indexing, physical input/display behavior, hardware GPU
memory, sessions beyond the stated durations, old-Git compatibility and release
packaging/install/update remain unverified in this pass. Complete the
[release checklist](release-checklist.md) on the final release commit; no version
bump, publication or manual acceptance boxes were completed here.
