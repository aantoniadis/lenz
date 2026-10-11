# Performance suite

Current results are in the [performance overview](performance.md). This guide covers running, comparing and maintaining the suite.

Run from a checkout with the normal [build prerequisites](../README.md#build), Git, and Python 3.10 or later. No benchmark Python packages or network datasets are required. Cargo may download locked build dependencies on the first build. The suite generates its own files and repository; it never benchmarks edits against your working files.

## Commands

All profiles also build and test the terminal frontend independently of GPUI.
Terminal coverage includes a 100,000-line scroll/render loop, resizing, long-line
find rendering, a cached 10,000-file navigator, scrolled help rendering, 300 About
open/close cycles with zero, tiny, narrow and large terminal resizes, and an owned
pseudo-terminal for first paint and content, keyboard feedback, idle output and CPU,
memory, and terminal restoration on normal, error and Ctrl-C exit. Quick uses 1,000
lines in the PTY probe; full and stress use 100,000. Stress sends 500 focus-switch
inputs per PTY run, compared with 30 in quick and full. The terminal probes run on
Linux and macOS without opening desktop windows; macOS execution is unverified.

Standalone terminal checks:

```sh
cargo build --locked --release -p lenz-tui
python3 scripts/tui-bench.py --app target/release/lenz-tui
# After a full/stress suite run: TUI startup, memory and key feedback over the GUI startup fixtures.
python3 scripts/tui-startup-bench.py --app target/release/lenz-tui --fixtures target/perf/full/fixtures
LENZ_PERF_JSON=1 cargo test --locked --release -p lenz-tui terminal_perf -- --ignored --nocapture
```

PTY timings end when the collector receives bytes, and TestBackend timings cover
cell rendering in memory. Neither measures the terminal emulator's display time.
First content is the first visible marker in the generated comparison, before
line-diff or inline refinement finishes, so content feedback is not full diff
readiness. Idle output is asserted zero for half a second after settling; CPU
counters have platform-dependent resolution. The terminal benchmarks cover
text and images, merge, version and conflict resolution, repository workflows,
streamed blame and search, and independent views.

```sh
# Fast component comparison; no display needed, Linux or macOS.
python3 scripts/perf-suite.py --profile quick --output target/perf/before

# Repeat after a code change with identical settings (see "Compare and gate").
python3 scripts/perf-suite.py --profile quick --output target/perf/after \
  --compare target/perf/before

# Comprehensive local run: three measured repetitions, plus untimed warmups.
python3 scripts/perf-suite.py --profile full --output target/perf/full

# Sustained allocation/worker/undo stress: 15 minutes in one process,
# plus 50 forward/backward browsing passes per UI trial on Linux.
python3 scripts/perf-suite.py --profile stress --output target/perf/stress

# Adjust stress length and repetition count explicitly.
python3 scripts/perf-suite.py --profile stress --stress-seconds 3600 --runs 5 \
  --output target/perf/hour
```

Long-session memory for the terminal (owned PTY; directory pair of large files, `]`/`[` through every file per round):

```sh
python3 scripts/tui-soak-bench.py --app target/release/lenz-tui --rounds 200 \
  target/perf/full/fixtures/files/left target/perf/full/fixtures/files/right
```

Standalone workflow and Python collector checks:

```sh
python3 scripts/tui-workflow-bench.py --app target/release/lenz-tui --lines 2000 --rounds 11
python3 -m unittest discover -s scripts -p 'test_*.py'
cargo test -p lenz --features gui-test gui_
```

Output directories must be new or empty; a timestamped directory under `target/perf/` is used when `--output` is omitted. Keep the directory to retain fixtures, per-case stdout and stderr, exact commands, executable hashes, a source manifest, machine and build metadata, raw samples (`results.json`) and a readable report (`summary.md`). `fixtures.json` identifies generated inputs by SHA-256. Results are written after each case and on interruption.

Build time is recorded separately. Tests run as prebuilt executables, sequentially, so compilation and competing benchmark processes do not enter component measurements. Quick runs usually take tens of seconds after building; full runs take several minutes; stress adds its configured sustained duration and longer browsing runs. Large fixtures require a few hundred MiB of disk space, and pathological workloads can use several GiB of RAM. Use `--profile quick` first on smaller machines.

## What runs

| Workload | Quick | Full | Stress |
| --- | --- | --- | --- |
| Generated 100k-line diff, 4 whitespace × 3 highlight modes | 3 internal samples | 11 internal samples | 11 internal samples |
| Unrelated 100k files, word and line modes | Yes | Yes | Yes |
| Uncapped unrelated character refinement (slow pathological case) | No | Yes | Yes |
| Three-way 100k merge, default/ignore whitespace | Yes | Yes | Yes |
| Split 50 MiB of text | Yes | Yes | Yes |
| View construction; shared/independent/padded scroll layouts | Yes | Yes | Yes |
| Refinement polling with 5,000 mostly offscreen blocks, all scroll modes | Yes | Yes | Yes |
| Terminal About open/close and tiny resize, 300 cycles per trial | Yes | Yes | Yes |
| Merge opening and accept/rebuild | Yes | Yes | Yes |
| Syntax query compile, parse, visible spans, retained bytes | Yes | Yes | Yes |
| Find: absent, ordinary and very frequent matches; capped Change Set search | Yes | Yes | Yes |
| 4K image blend and difference computation | Yes | Yes | Yes |
| Million-commit graph layout, visible queries, resident growth | Yes | Yes | Yes |
| 50 MiB ASCII and Windows-1252 detection/decoding | Yes | Yes | Yes |
| Blame: first hunk and completion on 200 commits / 3,000 lines | No | Yes | Yes |
| Mixed sustained model/worker workload in one process | No | 10 seconds | 900 seconds |
| GUI startup, idle CPU and memory matrix, when display probe available | No | Yes | Yes |
| Automated UI scenarios on private Linux display | No | 3 file-switch round trips | 50 file-switch round trips |
| Terminal text/long-line render and resize | Yes | Yes | Yes |
| Terminal 4K modes, syntax viewport, canceled loads and concurrent blame | Yes | Yes | Yes |
| Terminal 100k-line merge, binary picker, save and conflict-list switching | Yes | Yes | Yes |
| Four independent 4K views, F8 render/input and close/drop checks | Yes | Yes | Yes |
| Real PTY history/filter/refresh/search/blame/commit/new-view/conflict workflow | 300 lines × 3 rounds | 2,000 lines × 11 rounds | 20,000 lines × 111 rounds |
| Real PTY startup/input/resize/idle and terminal restoration | 1k lines / 30 inputs | 100k / 30 | 100k / 500 |
| Streamed Change Set search cancellation / retained rows / visible hit lookup | Yes | Yes | Yes |
| Fragmented blame producer / 100ms UI consumer / retained storage | No | Yes | Yes |

The shared-core refinement probe scans 5,000 change blocks in each scroll mode and checks that the viewport still selects the correct candidates. It catches regressions in polling when most blocks are offscreen.

`--runs` repeats the component collection and each UI scenario (default 1 for quick, 3 for full/stress). An additional untimed component process runs first. The mixed sustained workload runs once, for `--stress-seconds`; starting a fresh process every cycle would hide residency growth.

The terminal lifecycle tests verify exact output, shared Arc ownership and weak
references becoming unretainable after cancellation/close. Independent-view RSS
is sampled after one through four 4K comparisons and after closure; it scales
with intentionally retained documents. RSS includes allocator arenas and is not
expected to fall to the initial value immediately. The merge case exercises
100k-line choices, navigation/render/resize, save, cancellation, binary versions
and two-file Git conflicts. The PTY workflow additionally verifies search counts,
changed-only search, commit details, tab activation and a return from conflicts
after external staging. Offline fixtures cover PR state; no authenticated remote
PR timing or mutation runs in the suite.

The mixed workload keeps a four-document working set and repeatedly allocates/evicts 100k-line documents on the real document worker, checks canceled work, changes scroll modes, searches Unicode text, blends 2K images, accepts/undoes/redoes merge changes, and checks compact two-way undo/redo against complete original text. Every iteration verifies results. Its RSS timeline includes elapsed timestamps. This exercises models and the worker, not the renderer, application cache policy, or file writes.

The GUI startup matrix covers standalone About outside Git, small changes, generated and unrelated 100k-line diffs, very long lines, Unicode, CRLF, Windows-1252, binary and oversized placeholders, 4K PNGs, a 100k-line merge, a 500-commit repository, and 1,000 small changed files. Linux additionally runs fresh application-cache cases for small files, merge and images. The oversized case measures the confirmation placeholder; it does not claim to finish comparing the oversized file.

Linux interaction scenarios cover 20 About open/close cycles, scrolling, collapse, all scroll/whitespace/highlight modes, find and find navigation, all four merge accept shortcuts and undo, Base toggling, history selection, image modes/opacity/zoom/fit, and repeated slow/rapid browsing of 12 text or six 4K image comparisons. Memory is sampled after each action group and after settling. The stress profile performs 1,122 text transitions and 510 image transitions per trial. `--check` in the underlying interaction probe always checks repaint completion; it is independent of optional component timing budgets.

The GUI About scenario uses oracle version 3 and metric phases `about-open-v3` and
`about-close-v3`. It opens About from the command palette (F10, then typing `about`)
and verifies dialog-only version, platform, architecture and licence text with
ImageMagick `import` and `tesseract`, both required for this scenario. OCR runs after
the first-XDamage sample and is a correctness check, not a completion-latency
measurement. Terminal repository replay uses F10 and the explicit Filter command:
`/` and `:` in a focused commit list are search text.

Four-file GUI merge probes copy their inputs into a temporary directory outside
Git. Opening a Result equal to Base inside a repository can resume it as an
already resolved custom result, which makes acceptance timing meaningless.
The interaction probe checks unresolved markers, saves all four choices and
requires each Undo to restore the exact initial bytes. Original inputs are not
written.

Tokio 1.52.1 → 1.53.1 is an additional real-source corpus **only when both versions are already in the Cargo registry**. Missing coverage appears in the report; the suite never downloads it. The generated workloads are always present.

GUI input latency ends at the first XDamage notification after input. Coalesced or
pending background repaints can contribute that notification, so it cannot always
be attributed to completion of the requested action. Correctness checks separately
verify saved/undone merge bytes; completed model-work timings have separate metric
names. Preserve this boundary when interpreting sub-frame input numbers.

## Linux and macOS

| Capability | Linux | macOS |
| --- | --- | --- |
| Component timing and sustained model stress | Automated | Implemented; first Mac run still needed |
| Resident memory | `/proc` VmRSS, kernel high-water mark where available | `proc_pid_rusage` physical footprint and its lifetime peak; `ps` resident size kept beside it |
| Startup/idle CPU | Private Xvfb | Explicit native windows |
| Automated input → repaint and renderer browsing stress | XTest/XDamage on owned Xvfb | Not implemented; reported as skipped |
| Physical-display frame pacing and input → presentation | Manual | Manual |
| Actual cold boot / hardware GPU memory | Not measured | Not measured |

Linux GUI probes require `Xvfb`, libX11, libXtst and libXdamage (for example, Debian/Ubuntu packages `xvfb libx11-6 libxtst6 libxdamage1`). Supply `--xvfb /path/to/Xvfb` if needed. They create a private display and never send input to your desktop. `--ui off` collects components and model stress without any windows. `--require-ui` fails if startup or interaction coverage is unavailable; otherwise unavailable capabilities are explicitly skipped.

On macOS:

```sh
# Safe default: no desktop windows or synthetic input.
python3 scripts/perf-suite.py --profile full --output target/perf/mac-components

# Opt in to native application windows for the startup matrix.
python3 scripts/perf-suite.py --profile full --ui native --output target/perf/mac-native
```

The native probe launches and terminates its own windows, so run it when the desktop is free. It does not use Accessibility automation or send keys. Missing memory samples are unavailable, never fabricated as zero. macOS cache isolation and automated interactions remain gaps; `--require-ui` deliberately fails there until both are covered. Follow the [release walkthrough](release-checklist.md) for real scrolling, mouse input and display-refresh checks on either platform.

On macOS, `rss_mib` is the physical footprint (`proc_pid_rusage`, what Activity Monitor and `footprint` report) and `peak_rss_mib` its kernel-kept lifetime maximum; `resident_mib` keeps the `ps` resident size for reference. The resident size is not comparable with Linux `VmRSS`: libmalloc marks freed pages reusable instead of returning them, and the resident size keeps counting them until the kernel reclaims them under pressure, which it may never do. Before workload version 5 the Mac reports used the resident size, which read 254 MiB where the footprint was 122 MiB on the generated 100k-line TUI case. Two macOS allocator behaviours still show in the footprint and have no counterpart on Linux: libmalloc's large cache keeps freed buffers of several MiB (every 4K image buffer) charged until memory pressure, and about half of the pages freed by the first teardown of a large allocation burst can stay dirty with nothing to release them (the syntax arena exists to keep parse trees out of that path). In-process probes (`perf::rss_mib`) follow the same definition on both platforms.

The macOS path has parser and collection logic checks and uses portable Rust workloads; the first native runs are recorded in the [optimization notebook](optimization-notebook.md). Save a Mac report even if a case fails: logs and incomplete coverage remain useful.

## Compare and gate

```sh
# Fail if any comparable metric's median increases by more than 20%,
# or any baseline metric disappears.
python3 scripts/perf-suite.py --profile quick --output target/perf/candidate \
  --compare target/perf/before --fail-regression 20

# Compare different machines informationally; no regression gate allowed.
python3 scripts/perf-suite.py --profile quick --output target/perf/another-machine \
  --compare /path/to/other-machine-results --allow-machine-mismatch

# Preserve the original reference-machine absolute timing limits.
scripts/perf-check.sh
# Or enable those component limits in the portable suite:
python3 scripts/perf-suite.py --profile quick --check --output target/perf/gated
```

Default collection enforces workload correctness, subprocess timeouts and memory-growth limits, while recording timing without hard failure on a slower CPU. `--check` enables existing absolute component timing ceilings. `--compare` and `--fail-regression` require the same machine fingerprint, OS version, toolchain, allocator and compiler environment, generated fixtures, and suite settings: profile, `--runs`, `--stress-seconds`, `--ui`, `--max-growth-mib`, `--check` and `--renderer-label`. Pass the same `--renderer-label` for the baseline and the candidate run. The baseline must be a passing completed run. A different machine is never silently treated as a code regression. Source revision changes are expected; hashes and dirty state are recorded. Any unsupported comparison or malformed measurement fails clearly. A zero baseline followed by a positive value cannot produce a percentage: it is explicitly listed and fails an enabled regression gate.

Median and p95 are shown with sample counts. Some component samples already aggregate many operations (e.g. a thousand view-model scroll steps); their p95 is not per-frame latency. Single-sample cases do not support meaningful tail estimates. Repeated component processes capture some run-to-run variation, but the suite does not provide statistical significance tests. Tiny timings and RSS differences can be noisy: inspect raw distributions and rerun suspected regressions on an otherwise idle, plugged-in machine with the same power settings.

Memory checks default to a configurable `--max-growth-mib 512`: UI file browsing compares initial idle to final settled RSS; model stress compares median RSS in the final quarter against the preceding quarter, after initial allocation warmup. This is a loose leak/regression guard, not a product memory guarantee. Fewer than four model RSS samples result in an explicit skipped memory-growth check. Inspect the whole timeline for gradual growth; a long test is more useful than inflating a threshold.

The default per-case timeout is 300 seconds, configurable with `--timeout`. Builds have one hour and model stress has its duration plus that timeout. Failed commands or malformed measurements stop collection immediately and preserve the report and logs. Cancellation and timeouts terminate owned process groups; no unrelated app processes are targeted.

## Measurement boundaries

- First paint is main entry to a paint callback; it is not scanout to a physical monitor. Text may appear before syntax finishes.
- XDamage latency ends at the first server-visible damage. It may be a loading indicator, not completed asynchronous file/diff work. It is not FPS or input-to-photon latency.
- Component wall time excludes drawing. Process CPU samples exclude child-process CPU (notably Git). Wrapper RSS is diagnostic only; workload-specific RSS comes from the app/test process.
- Warm startup has an untimed launch. Fresh application cache is distinct from OS page cache; the suite never drops system caches or labels the first process launch “cold boot.”
- RSS includes allocator and renderer residency. The recorded Xvfb measurements used software rendering and are not comparable to a native hardware-rendered window. Record the renderer/device/driver with `--renderer-label` and keep it fixed for comparisons; startup logs retain application diagnostics. GPU allocations, energy, thermal throttling and actual cold boot require separate platform tools.
- Network/PR fetching, prolonged external-edit watching, all image formats, save/atomic-write throughput, and physical mouse/trackpad latency are outside the automated matrix. The suite is broad, not exhaustive.

The [optimization notebook](optimization-notebook.md) records approaches kept, rejected and still untried.

## Maintain the suite

Add a deterministic workload with correctness assertions, a stable metric name/unit and raw `LENZ_PERF` samples. Keep instrumentation in benchmark/test targets. Record skips rather than passing zero tests. Change `workload_version` in the runner when fixture semantics or an existing metric's workload changes; do not silently compare incompatible definitions. Keep new large fixtures generated, and avoid optional network inputs in required coverage.

Run `python3 -B scripts/test_perf_suite.py`, the workspace tests, Clippy, `scripts/perf-check.sh`, and a complete `full` run after changing collectors. Test failures, timeout cleanup, malformed records and baseline comparison are part of collector correctness. A successful Linux run does not validate the native Mac path.

The runner's `WORKLOAD_VERSION` is currently **5**. `--compare` refuses results saved with a different workload version; save a new baseline instead of comparing across versions. Version 3 requires the commit inspection modal's author/email line before reporting readiness, and uses the outside-Git merge fixture described above. Version 4 cycles all four Scroll Modes in the GUI diff scenario, so its find phases run in Shared again (version 3 left them in Independent). Version 5 reports the physical footprint instead of the `ps` resident size on macOS, and samples the terminal independent-views `initial` memory before its 4K fixtures are encoded; Linux metrics are unchanged except that `initial`.
