# Reader measurements

Measured on Linux on 2026-10-17, from source `9f440be4c0201a9d98ff131dbab0506e371acf24`.
This measures the Reader in the combined workspace; it is not a comparison with
another editor or an optimization claim.

| Workload | Median | p95 |
|---|---:|---:|
| Open a 2,000-line Rust file, warm Reader cache | 153 ms | 156 ms |
| Same file, empty Reader cache | 152 ms | 156 ms |
| Ten kept tabs, settled process RSS | 164 MiB | — |

The ten-tab run's process high-water mark was also 164 MiB, rounded to the nearest
MiB. [Raw samples and executable identity](reader-measurements.json) include every
measured launch, the Reader's internal timing, all memory samples and fixture hashes.

## Method

- Release build: `cargo build --release --locked -p reader-gui --features perf-probe`.
  The probe executable was copied aside, then the ordinary production binary was
  rebuilt without the feature. Screenshots use the production binary.
- An owned 1920×1200 Xvfb display, X11, Mesa lavapipe software rendering, Catppuccin
  theme and default Reader window size/font preferences. Configuration, state and
  caches were isolated from the user's session. No physical display was used.
- Twenty measured launches per cache mode, after three discarded warmups in each.
  OS pages and Mesa shaders stayed warm. The warm mode reused the Reader cache;
  the fresh mode used an empty Reader cache for each launch. Neither is a cold OS boot.
- Time starts in the parent immediately before spawning the process and ends when
  it receives `lenz-reader: readable` on stderr, polled every 2 ms. The marker is
  emitted during construction of the first frame containing document content,
  before painting its text rows and GPU presentation. It does not wait for syntax
  highlighting, project indexing or language-server readiness. The raw
  `reader_ms` samples use the app's own clock instead.
- The fixture contains ten Rust files, each with 500 four-line functions (2,000
  lines). Each function is `pub fn item_N() -> usize {`, an indented `N`, `}`, then
  a blank line, for N from 0 through 499. Startup opens the first file with the
  generated folder as its browsing root.
- Memory opens all ten files as kept tabs, waits for the `tabs ready` marker, then
  takes 120 Linux `VmRSS` and `VmHWM` samples with half-second pauses. Including
  collection overhead, the run lasted 68.5 seconds. Settled RSS is the median of
  the final 60 samples (roughly 34 seconds); peak is the greatest sampled process
  high-water mark. Inactive tabs release their document content by design: this
  is ten accessible tabs, not ten fully resident syntax trees.
- The Reader's executable search path was empty, disabling language servers.
  Assertions checked that it had no child-process RSS and that fixture bytes were
  unchanged. Language servers can consume substantially more memory; their
  [provider measurements](providers.md) are separate.

The collector reused `launch`, `wait_for`, `status`, `descendants` and `stop` from
[`scripts/code-launch-bench.py`](../../scripts/code-launch-bench.py), with the owned
display helper from [`scripts/capture-screenshots.py`](../../scripts/capture-screenshots.py).
The exact local driver and logs remain under `target/readme-reader-evidence/`;
published samples contain no personal paths, host names or hardware identifiers.

## Running the existing acceptance collector

Build a separate probe executable and run the collector against disposable source
files. It selects up to ten Rust files from the corpus for its memory run:

```sh
cargo build --release --locked -p reader-gui --features perf-probe
mkdir -p target/reader-bench
cp target/release/lenz-reader target/reader-bench/lenz-reader-probe
cargo build --release --locked -p reader-gui
python3 scripts/code-launch-bench.py \
  --binary "$PWD/target/reader-bench/lenz-reader-probe" \
  --file /path/to/disposable-corpus/module_0.rs \
  --corpus /path/to/disposable-corpus \
  --out target/reader-bench/results --trials 20 --geometry 1920x1200x24
```

Choose an unused X display with `--display` (default `:91`). This collector uses
the same content marker but starts its external timer just after process creation;
its external times exclude the spawn call. It also allows installed language
servers to start and reports their memory separately. To reproduce the server-free
workload, run it with a restricted `PATH` containing Xvfb and `ps`, but no language
servers, and invoke Python by its absolute path. Keep configuration, fixture,
cache mode and rendering backend equivalent when comparing results.

## Limits

These are startup and short idle-memory measurements, not physical display latency,
GPU-memory accounting or proof of leak freedom. They do not measure project search,
semantic navigation, large files or long sessions. Native macOS and hardware GPU
rendering remain unverified for this workspace trial. The [Diff performance
figures](../performance.md) use different fixtures and endpoints, so the tables
are not a direct speed comparison.

Earlier [shared-feature checks](../shared-features-performance.md) cover paired
before/after timing, input and 30-second Find stress. Their window-observation
endpoint is earlier than the content-frame marker used here.

## Theme chooser parity (2026-10-18)

Reader's theme chooser now previews the highlighted theme through a shared core
command, without changing or saving the selected theme. Commit and cancellation
use the existing theme service. GUI and TUI expose **Change theme** outside the
View group; the GUI app menu opens it directly. Current-source/help rows, keyboard
paging, pointer input and narrow layouts follow Diff's chooser. Wheel movement
keeps the highlight and preview unchanged.

The complete local CI gate passed, including workspace/core tests, actual Diff
and Reader GUI adapter tests, Reader TUI parity checks, panic checks and separate
production terminal builds. Real input regressions first reproduced the missing
preview in both adapters. The new cases cover cancellation after native appearance
updates, retaining theme IDs when custom themes arrive, removal of a previewed
theme, inert headers, current selection, persistence and 1×1 layouts.

Local raw evidence is in `target/perf/reader-theme-parity/` (ignored): original
production binaries/source, compiler identity, regression logs, full CI/build
logs, screenshots, private PTY output and production executable hashes. The
production probes use isolated settings and disposable source files, rather than
the user's desktop or repositories. A first GUI probe passed its assertions and
60-second exercise but failed while reading an already-closed log; its evidence
is retained as `native-gui-initial.*` and the corrected collector was rerun.

Production GUI checks passed after the collector fix, including viewport color
changes before commit, header/outside clicks, wheel stability, reopening and tiny
resizes. Its preview/cancel exercise completed 87 cycles in
60.016 seconds; sampled RSS ranged
170.574–170.598 MiB, with
0.023 MiB final-quarter growth and
170.598 MiB process high-water RSS. The private PTY completed
147 cycles in 60.166 seconds, with RSS
8.773→8.809 MiB
and 8.809 MiB high-water RSS. Source and appearance bytes
were unchanged during cancellation loops, and terminal modes were restored.
These candidate-only checks overlapped other correctness/build work and are not
paired performance comparisons.

Adjacent before/after release comparisons used 100,000 plain-text lines with no
language server, the same private Xvfb/lavapipe setup and Rust 1.99.1. Seven GUI
pairs followed one discarded warmup pair; ten TUI pairs alternated launch order.
The GUI startup endpoint is the first non-uniform client image received by the
collector; menu feedback ends at a changed client image and includes capture
cost. Terminal endpoints are received PTY bytes. These differ from the historical
content-frame marker above and do not measure physical presentation.

| Measurement | Before | After |
|---|---:|---:|
| GUI first client image | 157.02 ms | 154.04 ms |
| GUI menu feedback | 25.11 ms | 23.39 ms |
| GUI sampled RSS / median high-water at sample | 173.61 MiB | 173.44 MiB |
| TUI first content | 7.971 ms | 7.984 ms |
| TUI arrow feedback | 0.473 ms | 0.439 ms |
| TUI sampled RSS | 13.539 MiB | 13.604 MiB |
| TUI median high-water at sample | 19.527 MiB | 19.539 MiB |

The paired menu/status exercises ran 15.021/15.017 seconds for GUI
(192/189 cycles) and 15.025/15.023 seconds for TUI (182 cycles each). Final-quarter
RSS growth was zero in all four runs. Raw samples, exact executable hashes,
terminal streams and configuration are in `paired-gui.json` and
`paired-tui.json`. No repeatable startup, response or memory regression appeared
in these workloads. The full Diff performance suite was not rerun for this Reader
change. Native macOS/Windows, hardware GPU allocations, physical display latency
and long-session leak behavior remain unverified.


## Reader chrome parity (2026-10-18)

Reader's GUI and TUI now put frequent shared commands in a dedicated toolbar
above the tabs. The footer separates LSP recovery, the primary message,
position and Commands. Both adapters use the same status content and
column-aware ellipsis; clicking the message opens a frozen, wrapped snapshot
with full-text copying. Routine provider RSS samples stay in details. The GUI
also stops painting its LSP label twice, and independent terminal views retain
the footer rather than overwriting it with view-switching instructions.

The full local gate passed, including workspace/core, Diff GUI, Reader GUI
(124 passed, 7 ignored), Reader TUI (59 passed), terminal parity, collector tests,
and separate terminal release builds. Reader's normal GUI release was built
separately after the gate. Red/green evidence includes the old footer click
opening LSP recovery instead of message details, the absent terminal Menu
control and changing memory figures in the brief status. The initial lint and
panic-site failures were corrected without raising a baseline; their logs remain.

Two sequential release comparisons used the same generated 100,000-line plain
text, disposable HOME/XDG directories, alternating before/after order, and no
concurrent builds or other collectors. Each GUI round had seven measured pairs
and one warmup pair on an owned 1920×1200 Xvfb/lavapipe display; each terminal
round had ten pairs on owned PTYs. GUI feedback measured six F10/Escape changes
per launch; terminal feedback measured five Down inputs. Startup is receipt of
first nonuniform client pixels or source text, respectively; these are not
completed-action or physical-display timings.

| Median metric | Round 1 before → after | Round 2 before → after |
| --- | --- | --- |
| GUI first client image, ms | 153.90 → 156.36 | 157.35 → 159.47 |
| GUI menu feedback, ms | 22.92 → 23.24 | 24.13 → 24.40 |
| GUI initial sampled RSS, MiB | 173.73 → 174.99 | 173.68 → 174.30 |
| TUI source receipt, ms | 8.63 → 8.73 | 8.88 → 8.80 |
| TUI arrow feedback, ms | 0.691 → 0.740 | 0.703 → 0.741 |
| TUI initial sampled RSS, MiB | 13.52 → 13.62 | 13.65 → 13.66 |
| TUI sampled VmHWM, MiB | 19.79 → 19.65 | 19.76 → 19.91 |

Timing differences changed sign between individual pairs; resampling paired
launch medians across both rounds did not separate any timing difference from
zero (95% percentile bootstrap, 10,000 resamples). The small initial GUI RSS
increase is recorded rather than hidden: the two final 15-second menu exercises
instead settled at 175.75–175.76 → 174.99 MiB and 175.93 → 174.73 MiB. Initial
samples are not settled memory or GPU memory. Each terminal round also retained
15 seconds of LSP-dialog exercise for each binary. All fixture bytes stayed
unchanged and each terminal restored its input modes on exit.

Production correctness replay additionally covered actual menu/Find file/Commands
and message clicks, separate F2 LSP recovery, native GUI clipboard/terminal OSC52,
immutable multiline snapshots, modal input isolation, both appearances, End then
Up, 320×220 GUI / 36×12 terminal layouts, 1×1 resize and terminal independent views.
A 400 ms quiet footer image comparison / no-idle-output check passed. A sustained
GUI status open/scroll/copy/dismiss/resize run lasted 60.004 seconds / 111 cycles,
with sampled RSS 173.52–173.65 MiB (final-quarter range 0.078 MiB). The terminal
run lasted 60.222 seconds / 159 cycles, RSS 9.00–9.77 MiB (final-quarter range
0.00 MiB). These smaller correctness fixtures are separate from the timing
fixture above; the figures must not be compared as one workload.

Raw evidence is in `target/perf/reader-chrome/`: `comparison-summary.json`,
`paired-{gui,tui}{,-first}.json`, collectors and their hashes, native reports/raw
PTY bytes/screenshots, visual inspection notes, source before/after archives,
patches, executable identities, configuration and full gate logs. The first
GUI collector's full-window OCR missed a visible Actions tab; its failure is
retained under `native-gui-initial-ocr-failed/`, and focused tab OCR passed the
rerun. The native OS Open chooser is unavailable on this display; its toolbar
routing has adapter-test coverage, while actual Find file popup controls passed
wide/narrow production replay. Native macOS, hardware rendering, physical
latency, screen-reader behavior and long-session leak behavior remain unverified.
