# Reader measurements

The [2026-10-21 release validation](../release-validation.md) has the latest
paired startup/scrolling, Rails checkout recovery, cross-language, Libraries and
stress results. Earlier experiments below retain their original configurations.

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

## Trackpad scrolling and indexing progress (2026-10-21)

Source panes now accumulate fractional pixel-wheel movement rather than rounding each event away. Fractions
are scoped to the pane and document version, including replacement of a preview tab; overscroll at every
edge is discarded so it cannot delay reversal. Sub-row input no longer dispatches a zero-row scroll command.
This remains row/column scrolling, rather than pixel-offset rendering. Project explorer and results-list
pixel scrolling are outside this change.

LSP work-done progress retains the full title, message and optional percentage across partial reports. Both
footers show the current task, with percentage first when supplied; F2 retains the full description. Multiple
tasks keep the provider busy until all end. A percentage describes one task, not overall project completion.
The [Rails setup guide](providers.md#rails-setup-and-large-projects) explains Ruby environments and optional
indexing exclusions. No default indexing coverage was removed and no server indexing speedup is claimed.

The complete `scripts/ci-local.sh` gate passes, including workspace/core tests, actual GUI input tests,
terminal rendering and combined adapter replays. New regressions went red for discarded quarter-row input,
lost percentages, EOF reversal and a reused preview tab inheriting the previous document's fraction. GUI
checks cover vertical/horizontal movement, source preservation, narrow footers and F2; terminal progress
checks cover both appearances and narrow layouts. The installed Ruby LSP 0.26.11 passes its real-provider
navigation, dependency, external-edit and source-preservation fixture. That fixture is not a Rails app.

Release comparisons use the same machine, an owned 1600×1000 Xvfb display, Mesa lavapipe, Catppuccin and
isolated XDG config/state/cache. The baseline GUI probe was built from the unchanged checkout; the final
candidate includes the fixes above. Each repeated GUI case has an excluded warmup per executable and
alternates order. Raw evidence, source diff/revision, executable hashes, drivers, configuration and timelines
remain under `target/perf/reader-scroll-indexing/` (ignored), principally `paired-verified.json`, `rss.json`,
`paired-tui-verified.json`, `before/dispatch-verified.log`, `after/dispatch-verified.log` and
`ci-local-verified.log`. Production GUI/TUI executables are rebuilt without probe features afterward.

| Final GUI workload | Before | After |
|---|---:|---:|
| First content-frame construction, 200-line text, four pairs | 126.78 ms | 126.65 ms |
| 100k-line text scrolling, two three-second pairs, median frame interval | 48.00 ms | 48.00 ms |
| 2,001-file Ruby project loading and scrolling, two five-second pairs, median frame interval | 48.00 ms | 48.00 ms |
| Same Ruby case, reader CPU per run | 3.285 s | 3.330 s |
| Same Ruby case, reader settled / peak sampled RSS | 182.07 / 183.09 MiB | 181.95 / 183.43 MiB |
| Fifteen-second text scroll, reader CPU | 7.51 s | 7.52 s |
| Same stress, settled / peak sampled RSS | 166.04 / 166.06 MiB | 165.15 / 165.17 MiB |

The Ruby fixture has 2,000 files with twenty methods each plus a 3,000-line active source; it has no Rails
dependencies. Its server's presence is checked through child-process RSS exceeding 60 MiB. That check proves
the server ran, not that every measured frame overlapped indexing or that navigation was ready. Reader CPU
and RSS exclude the server. The scroll probe dispatches core row steps, so these intervals measure rendering
under load, not native trackpad-event latency or physical display feedback; the software renderer's roughly
48 ms cadence is not a claim of 60 Hz performance. The input regressions separately exercise actual GUI
wheel events.

The stress probes each recorded 15.024 seconds of frame intervals (wall durations 15.180 / 15.214 seconds).
The last hundred RSS samples ranged 166.035–166.059 / 165.148–165.172 MiB. This short bounded observation
does not establish leak freedom or GPU-memory use. A separate five-pair idle repeat measured RSS
158.27→158.34 MiB, anonymous RSS 48.95→49.00 MiB and file-backed RSS 92.75→92.59 MiB.

Four terminal pairs against the rebuilt baseline measured first source receipt 6.03→6.00 ms, arrow feedback
0.411→0.400 ms and RSS 13.65→13.91 MiB. Five-second terminal F2/dismiss stress completed 61 cycles per
executable; these samples do not exercise terminal indexing load. The 20,000-progress-message core probe
measured p99 dispatch 0.0042→0.0058 ms and warm fake-server definitions 2.179→2.177 ms median. The latter
includes the harness's polling floor, not just server computation.

Earlier attempts are retained separately. One Ruby run used isolated XDG directories without preserving
RubyGems' search path and never loaded the server; those Ruby results are excluded. Explicit `GEM_PATH`
and the server-presence assertion corrected the fixture. Intermediate builds showed roughly +4 MiB RSS;
the final repeated workloads and separate idle attribution did not reproduce that increase. Early terminal
and semantic measurements used pre-existing artifacts with uncertain source identity; the final baseline
terminal and semantic test are rebuilt from a clean archive of HEAD, with a forced semantic-test rebuild.
These experiments establish neither an overall indexing improvement nor exact timing/memory preservation.
Native macOS/Windows, the reported Rails project, long sessions, hardware GPU behavior and the Rails add-on
remain unverified.

## Shared GUI wheel input (2026-10-21)

`lenz-gpui::scroll` now owns pixel/line conversion, Shift-wheel redirection, fractional-step accumulation and
discarding fractional overscroll. Diff and Reader share those rules; their existing application cores retain
document state and scrolling commands. No new dependency or configuration was added. Source wheel notches
move three rows/columns, Shift redirects vertical pixels sideways at the same speed, and diagonal input
retains both components. Diff keeps its continuous fractional viewport; Reader consumes whole core steps
while retaining the remainder per pane/document.

Diff previously discarded a small vertical component whenever horizontal scrolling occurred. A red/green
adapter regression covers diagonal quarter-row events on both sides in Shared, Aligned, Padded and
Independent modes. Its PR descriptions now use the same fractional-step/clamp helpers as Reader, with a
red/green edge-reversal check. Review found Reader's horizontal limits mixed rounding and flooring, leaving
the last partial column clipped after a repaint; input, boundary checks and rendering now use the same
ceiling in columns, covered by repeated actual wheel events and repaints. Existing Reader edge, document
switch and split/selection tests remain applicable. The TUIs do not normalize GPUI pixel events; their core
and adapter tests remain in the full local gate.

The pre-change baseline here includes the earlier Reader/LSP work above. Same-machine release comparisons
retain source/executable identities, raw logs, repeat drivers, process samples and fixture assertions under
`target/perf/shared-scroll/` (ignored). `paired-verified.json` uses four alternating pairs per product, one excluded
warmup per executable, isolated state and disposable 20,000-line text fixtures. Diff changes one line in each
hundred. An owned 1920×1200 Xvfb display uses Mesa lavapipe and Catppuccin. Startup ends at the first captured
nonuniform client image; wheel feedback ends at a changed client image after an injected X11 wheel event.
These include capture overhead and do not measure physical display or macOS trackpad latency. Source hashes
are checked after every run. Stress alternates wheel down/up for at least ten seconds per executable.

| Final production comparison (medians) | Before | After |
|---|---:|---:|
| Reader first captured image | 155.58 ms | 158.00 ms |
| Reader wheel feedback | 25.53 ms | 26.11 ms |
| Reader RSS / sampled lifetime peak | 170.64 / 170.76 MiB | 171.09 / 171.43 MiB |
| Diff first captured image | 188.36 ms | 192.41 ms |
| Diff wheel feedback | 27.04 ms | 27.23 ms |
| Diff RSS / sampled lifetime peak | 223.28 / 223.43 MiB | 221.72 / 221.80 MiB |

An earlier four-pair run measured Reader first image 155.96→157.11 ms and wheel feedback 27.39→24.44 ms;
Diff measured 195.44→188.79 ms and 29.91→26.77 ms. Timing direction varies between runs, and these short
samples do not establish exact preservation. Final Reader anonymous RSS was 50.59→50.67 MiB, file-backed
95.42→95.79 MiB and shared 24.63→24.63 MiB; Diff was 64.89→65.06, 100.52→98.92 and 57.75→57.75 MiB.
An earlier Reader RSS increase of about 2.8 MiB did not reproduce at that size in the final repeat.

Final Reader stress lasted 10.036 seconds per executable (200 / 193 down/up cycles), with RSS ranges
170.762–170.953 / 171.277–172.887 MiB. Diff lasted 10.018 / 10.004 seconds (156 cycles each), with RSS
222.590–222.594 / 221.691–221.695 MiB. These are short process-RSS observations, not leak proof or GPU-memory
measurements. An attempted repeat lost its private X11 connection during display construction; its partial
results remain in `paired-final.json` and are excluded. The final collector keeps one owned private display
for the batch, with fresh per-launch config/state/cache and a disposable working directory.

The final local gate, shared unit checks, actual Diff/Reader input regressions and GUI/TUI parity suites pass.
Production executables exclude test/probe features. Native macOS/Windows, physical high-resolution trackpad
events, long sessions and native GPU memory remain unverified. Explorer/list/dialog scrolling beyond the PR
description path was not redesigned.

## Large Rails indexing experiment (2026-10-21)

Discourse **3.5.0**, pinned at
[`05a304006600f36c3e45d19c9c5919f43f5541c9`](https://github.com/discourse/discourse/tree/05a304006600f36c3e45d19c9c5919f43f5541c9),
provided a disposable Rails corpus: 8,294 Ruby files on disk and 277 installed bundle gems. Ruby LSP
**0.26.11**, Ruby **3.4.10**, Linux x86-64, an AMD Ryzen AI MAX+ 395 (32 logical CPUs, 64 GiB RAM), and the
current release-built Reader core were used throughout. Dependencies were installed outside the project
under `target/perf/rails-indexing/bundle`; source and Gemfile.lock remained unchanged.

The useful result survives removal of the Ruby timing hooks. These are medians of three samples per case,
after an excluded warmup, reversing the case order in the middle round:

| Configuration, without Ruby timing hooks | Ruby LSP ready | Local declarations complete | Peak summed process RSS |
|---|---:|---:|---:|
| Default | 13.20 s | 7.56 s | 747.8 MiB |
| Selected project + tooling-gem exclusions | 10.67 s | 7.56 s | 648.0 MiB |
| YJIT enabled, default coverage | 13.45 s | 7.51 s | 752.7 MiB |

Combined exclusions reduced observed readiness by **19.2%** and peak summed RSS by about **100 MiB**.
YJIT showed no startup improvement on this workload. Each sample used a fresh process, provider configuration
and Reader cache with warm OS pages; none measures a cold OS boot. Time starts immediately before the shared
Session's `OpenRoot`/`Open` commands and ends only after observing Ruby LSP indexing followed by Ready.
The driver waits for the local declaration index too, then verifies a real cross-file definition from
`User`'s `Searchable` include to `app/models/concerns/searchable.rb`. All 27 measured samples passed this
check and indexed all 12,077 local declaration candidates. This is shared-core/provider evidence, not GUI
first paint, terminal presentation or a comprehensive navigation test.

### What was excluded

The experiment used these Ruby LSP initialization settings, merged into the existing provider configuration:

```json
{
  "indexing": {
    "excludedPatterns": ["db/**/*.rb", "migrations/**/*.rb", "spec/**/*.rb", "script/**/*.rb"],
    "excludedGems": [
      "parser", "ast", "rubocop", "rubocop-ast", "rubocop-capybara", "rubocop-discourse",
      "rubocop-factory_bot", "rubocop-rails", "rubocop-rspec", "rubocop-rspec_rails", "syntax_tree"
    ]
  }
}
```

These are workload-specific tradeoffs, not recommended blanket exclusions. They reduce navigation into the
excluded migrations, test support, scripts and tooling internals. Application and plugin sources and Rails
gems were retained. Default server coverage was 5,824 project files plus 10,147 dependency/stdlib files;
combined exclusions left 3,610 plus 8,649. File patterns alone do not exclude dependency gems. Lenz's own
declaration index has separate coverage and was not reduced by these server settings. Provider settings
currently apply across Ruby workspaces; see the [configuration scope and glob rules](providers.md#rails-setup-and-large-projects).
No runtime defaults or user settings were changed by the experiment.

### Phase diagnosis and rejected experiments

A preceding instrumented matrix ran six configurations three times each, with a discarded warmup and the
middle round reversed. Benchmark-only Ruby hooks measured discovery, core RBS loading and per-file parsing;
they did not modify the installed server. The hooks add overhead, so use the table above for the final gain:

| Configuration, with Ruby timing hooks | Ruby LSP ready | Local declarations complete | Peak summed RSS |
|---|---:|---:|---:|
| Default | 15.68 s | 8.60 s | 739.3 MiB |
| Hold local index completions until server ready | 14.20 s | 22.04 s | 955.5 MiB |
| Disable automatic hints and semantic tokens | 14.03 s | 8.01 s | 739.6 MiB |
| Project exclusions only | 14.00 s | 8.20 s | 714.8 MiB |
| Tooling-gem exclusions only | 13.30 s | 8.70 s | 671.4 MiB |
| Combined exclusions | 11.08 s | 7.92 s | 636.5 MiB |

Baseline discovery took about 0.58 s and core RBS loading 0.28 s; most indexing time was parsing and recording
declarations, especially dependencies. Parser alone cost about 1.2 s for 76 files in the first measured
sample; RuboCop about 0.95 s for 890 files. By comparison, 1,619 project `db` files cost about 0.18 s.
Removing the greatest number of files is therefore a poor substitute for measuring their cost. Linting was
already disabled; that does not exclude linter/parser gem source from the navigation index.

Holding the first completed local 32-file chunk prevents subsequent crawl chunks from starting until Ready.
It approximates deferring the crawl without changing production scheduling. Its modest server gain came
with much later local symbols and a higher peak RSS, so it was not adopted. Disabling decorations produced
small, order-sensitive differences in overlapping timing ranges; it was not adopted either. YJIT was tested
with `providers.ruby-lsp.env.RUBY_YJIT_ENABLE=1`, using Ruby's supported
[environment switch](https://docs.ruby-lang.org/en/3.4/yjit/yjit_md.html#label-Command-Line+Options), and is also
left opt-in. No persistent Ruby LSP cache was added.

### Evidence and limits

Workload version **1** artifacts remain under `target/perf/rails-indexing/`: `identity.json`,
`reproducibility.json`, `source.diff`, `rails_index_probe.rs`, `run-matrix.py`, `instrument.rb`, `run.py`,
`yjit.py`, installation/build logs, every per-sample configuration, provider-status and phase logs, and
memory/CPU timelines (50 ms sleep plus collection time). `results.json`/`summary.json` hold the instrumented matrix;
`uninstrumented-results.json`/`uninstrumented-summary.json` hold the confirmation and YJIT comparison.
The retained probe executable is identified by SHA-256. Initial pilot/preflight artifacts are excluded;
preflight matching checks caught that a trailing `/**` does not recursively exclude nested Ruby files with
the server's flags, and the measured matrix uses `/**/*.rb` instead.

RSS is sampled Linux VmRSS summed over the owned driver/server tree, with shared pages potentially counted
twice. The peak covers startup, both index completions and the navigation check; it is not settled memory,
GPU memory or leak proof. The observer adds overhead to every case. Samples cover roughly 10–24 seconds
each, with no sustained stress run or frontend responsiveness measurement. Native macOS/Windows, the
reported private Rails project and Rails add-on application boot remain unverified; the add-on was absent.
Only benchmark artifacts and documentation changed in this experiment, so runtime parity gates were not
rerun; the release probe's assertions, fixture integrity, local documentation links and whitespace checks
were verified.

## Editor LSP practices trial (2026-10-21)

The [primary-source research](../research/large-workspace-lsp.md) found applicable practices in the Ruby LSP
VS Code extension, Zed, Neovim and Helix: explicit server indexing settings, correct project boundaries,
on-demand launch and reuse, and project-scoped configuration. Lenz already starts Ruby LSP on document demand
and reuses it by provider/root/launch. A fresh trial tested a narrower exclusion profile and real reuse on
the same pinned Discourse corpus, installed bundle, Ruby 3.4.10 and Ruby LSP 0.26.11 described above.

Three fresh shared-core sessions per case, after a discarded warmup, reversing case order in the middle
round, produced these medians without Ruby profiling hooks:

| Configuration | Ruby LSP ready | Local declarations complete | Peak summed driver/server RSS |
|---|---:|---:|---:|
| Default | 13.33 s | 7.59 s | 724.5 MiB |
| Tooling-gem exclusions only | 11.94 s | 7.90 s | 682.8 MiB |

The narrower profile improved observed readiness by **10.4%**, with about **42 MiB** less peak summed RSS.
It uses only the eleven `excludedGems` names in the earlier experiment's configuration, with no
`excludedPatterns`. Project migrations, scripts, test support, application and plugin code all remain in
the Ruby LSP project file set. Navigation into the excluded tooling gems is the coverage tradeoff. The
official extension's exclusion example supports this approach; it is an explicit setting, not an editor
default. Linting was already disabled. Timing ranges were default 12.93–13.79 s and tooling-only
11.26–13.20 s; this small sample does not establish a universal improvement or a tail-latency claim.

Every measured startup completed all **12,077** local declaration candidates, resolved `User`'s `Searchable`
include into the application concern, and resolved `ActiveRecord::Base` into the installed Rails gem's
`lib/active_record/base.rb`. The startup endpoint remains Session `OpenRoot`/`Open` through observed server
indexing followed by Ready. The extra dependency navigation occurs after this endpoint; memory sampling
includes both navigation checks. Warm OS pages and fresh process/config/cache were used throughout.

An additional **30.082-second** run switched among `User`, `Topic` and `Post` **1,062** times, asking the real
server for each file's symbols. Every answer was nonempty; exactly one server remained associated with the
Discourse root, Ready after every completed request, and process sampling verified one stable Ruby LSP PID
after initial indexing. Median file-symbol completion was **28.86 ms**. The shared-core document-open median
was 0.094 ms, which is an in-memory/model endpoint, not rendered content or input latency. This checks reuse
within one Session, not across independent windows or application launches. Peak summed RSS over this
different, repeated-navigation workload was 837.1 MiB; its short duration does not establish leak freedom.

The useful configuration gap is **per-project initialization settings**. VS Code/Zed/Helix support project
configuration, whereas Lenz's current provider settings apply across projects. These experiments used
isolated configuration directories and did not apply exclusions to the user's projects. A future Lenz
override belongs in the shared provider configuration, with usable support in both adapters. Watcher/search
exclusions should not be copied into a server's index policy without checking its meaning; Ruby LSP owns
its initial index, and clangd-style persistent indexing caches are not a generic client setting.

Workload version **1** artifacts are retained in `target/perf/editor-lsp-practices/`: `identity.json`,
`source.diff`, `editor_lsp_probe.rs` and the hashed release probe executable, `collect.py`, `run-practices.py`,
`results.json`, `summary.json`, `reuse.json`, build logs, every provider configuration, provider-status logs
and process-tree memory/CPU samples (50 ms sleep plus collection time). The prior trial's input hashes and machine identity remain
applicable. Preflight attempts are excluded: the expanded dependency check initially stopped before
asynchronous target navigation completed; the measured driver waits for the actual target document.
No installed server code, runtime defaults or production scheduling changed. Fixture and Gemfile.lock
integrity, raw-result assertions, source identity, documentation links and whitespace checks were verified;
runtime parity gates were not rerun for this research/benchmark/documentation-only follow-up. Native macOS,
other editor binaries, the private Rails project, Rails add-on boot and long sessions remain unverified.

## Project-scoped LSP settings (2026-10-21)

The shared reader core now merges provider defaults, global settings and an override for the resolved
language-server workspace root. Both adapters expose **Exclude Ruby Tooling Gems for This Project** and
**Reset LSP Settings for This Project** through F2 and their command menus. The opt-in profile excludes
only the eleven tooling gems measured above. It preserves custom exclusions and unrelated configuration,
writes outside the checkout, and restarts only the saved provider/workspace after success. See
[configuration and coverage limits](providers.md#project-language-server-settings).

The complete `scripts/ci-local.sh` gate passed: workspace/core checks, actual Diff and Reader GUI tests,
Reader TUI input/render tests, collector tests and separate terminal release builds. A combined adapter
replay invokes both new actions through F2 in dark/light and narrow layouts and compares saved settings
and unchanged source bytes. Protocol tests cover independent roots, matching initialization/configuration,
reset, failure and switching projects before a save completes. Review found a cancelled workspace-symbol
search could retain its spinner; a held-save-completion regression reproduced it before the shared fix.
An initial gate also caught adapter fixtures enabling a server before its asynchronous disable save
finished; the fixtures now wait for command availability.

### Large-project release comparison

The same pinned Discourse checkout, installed bundle, Ruby 3.4.10 and Ruby LSP 0.26.11 from the earlier trials
were used with the saved pre-change and new standalone release core probes. Three sessions per
binary/configuration, with alternating binary order and reversed configuration order in the middle round,
produced these medians after one discarded warmup:

| Binary and configuration | Ruby LSP ready | Local declarations complete | Peak summed driver/server RSS |
|---|---:|---:|---:|
| Before, defaults | 13.86 s | 7.94 s | 749.5 MiB |
| After, defaults | 13.78 s | 7.70 s | 761.3 MiB |
| Before, global tooling exclusions | 12.72 s | 8.13 s | 662.5 MiB |
| After, project tooling exclusions | 12.92 s | 8.28 s | 673.6 MiB |

Within the new binary, the scoped profile reduced readiness **6.2%** and peak summed RSS about
**88 MiB** against its defaults in this batch. All twelve samples completed **12,077** local declaration
candidates and navigated to both the application concern and the installed ActiveRecord gem. This keeps
normal project-file configuration and reduces tooling-gem navigation; it is not an across-project guarantee.
Before readiness ranges were normal 13.84–15.11 s, tooling-only 11.93–13.66 s.
After readiness ranges were normal 13.32–14.28 s, tooling-only 12.78–13.37 s.

A separate **30.043-second** run made **1,020** file switches and nonempty
file-symbol requests using the scoped profile. Every completed request retained one Ready server for the
same root, and owned-process sampling verified a stable Ruby LSP PID after indexing. Median file-symbol
completion was **29.94 ms**. This tests reuse within one session; it does not cache
indexes between application launches.

### Frontend release checks

Four alternating pairs, after a discarded GUI warmup, used an owned Xvfb display, software Vulkan and
fresh XDG directories. A 20,000-line text file isolates frontend startup and scrolling from server load;
both binaries read the same small provider configuration containing a project override. A separate
100,000-line terminal fixture measures PTY output. Production GUI, standalone terminal and core probe
were built separately to avoid dependency feature unification:

| Probe median | Before | After |
|---|---:|---:|
| GUI first captured client image | 156.81 ms | 163.63 ms |
| GUI wheel to changed captured image | 24.12 ms | 26.58 ms |
| GUI sampled RSS | 167.01 MiB | 168.16 MiB |
| TUI first source received | 6.50 ms | 6.62 ms |
| TUI Down to changed PTY output | 0.403 ms | 0.414 ms |
| TUI sampled RSS | 13.92 MiB | 13.89 MiB |

GUI before scroll stress lasted 10.055 s/182 cycles, with RSS 167.57–169.20 MiB.
GUI after scroll stress lasted 10.053 s/185 cycles, with RSS 168.14–168.33 MiB.
TUI before F2 open/close stress lasted 5.045 s/61 cycles, with RSS 14.05–14.11 MiB.
TUI after F2 open/close stress lasted 5.058 s/61 cycles, with RSS 13.99–14.05 MiB.
All fixtures stayed unchanged and terminal state was restored. These short tests are not leak proof;
small differences and overlapping startup ranges do not prove exact preservation.

A second four-pair production batch measured first GUI image 160.30→170.98 ms,
wheel feedback 25.81→25.55 ms and sampled RSS
167.00→165.99 MiB. Terminal source receipt measured
7.67→7.80 ms.
The repeat is retained under `verified-repeat/`. Wheel timing and RSS differences changed direction,
while first GUI image remained about 7–11 ms slower than the saved baseline across the two batches.
A clean reconstruction of the pre-change source (`HEAD` plus the saved source diff and inherited
shared-wheel module) was therefore built separately. Its GUI source matches the current adapter source.
Six alternating pairs, excluding a warmup, measured first image **165.06→162.13 ms**, wheel feedback
**26.15→25.12 ms**, and sampled RSS **167.76→168.25 MiB**. The startup penalty against the old saved
executable did not reproduce against this freshly built source baseline. Both comparisons remain recorded;
exact preservation against the older executable is not established. Source/executable hashes, reconstruction
and six-pair raw samples remain under `rebuilt-baseline-check/`. No deliberate startup tradeoff was accepted.

### Evidence and limits

Workload version **1** separate-build artifacts remain under `target/perf/project-lsp-settings/verified/`: Python
drivers, unchanged Rust probe, per-session configurations, status logs, memory/CPU timelines and result JSON.
The sibling `before/` and `after/` hold executable/source identities and raw source diffs; build/gate and
red/green logs remain in the parent directory. The Rails endpoint observes indexing followed by Ready,
independently of local index completion. Timelines sample owned processes with a 50 ms sleep plus collection
time; summed VmRSS may count shared pages twice. GUI captures include capture overhead and measure first
feedback rather than physical display latency. Terminal values are PTY receipt, not display latency.

Initial root-level and `frontend-repeat/` measurements are **diagnostic only**, excluded from the
tables. Building GUI and core example in one Cargo invocation enabled GUI test dependencies and unified
raster-image features into the probe. Increased executable sizes exposed that mismatch; both executables
were rebuilt separately and all production workloads repeated. Original combined binaries and identities
remain under `combined-build/`; earlier tooling-profile repeat and same-binary scope-isolation trials remain
as diagnostic evidence, not production performance claims.

All fixtures are disposable; no user configuration was changed. Native macOS/Windows, hardware trackpads,
GPU memory, the private Rails app, Rails add-on boot and long sessions remain unverified.

## Persistent index prototypes (2026-10-21)

**Decision: pursue a content-validated persistent declaration index in the shared reader core.**
It made the full local symbol index available much earlier and reduced CPU work while keeping the normal
Ruby LSP startup policy and coverage. This is a direction selected by experiments, **not a shipped cache**.
The isolated integration still needs production storage/lifecycle work and memory/adapter validation.
Reject metadata-only validation and the simple Ruby gem-record replay prototype on correctness grounds.

### Workload and boundaries

The same Discourse 3.5.0 fixture, commit `05a304006600f36c3e45d19c9c5919f43f5541c9`, was used:
12,077 local declaration candidates, 58,844 declaration records, and Ruby LSP 0.26.11's default coverage.
Actual server versions were Ruby 3.4.10 and Prism 1.9.0. The machine was the same Linux AMD Ryzen AI MAX+ 395
with 32 logical CPUs. Gemfile/lockfile and fixture Git status were unchanged.

Every timing case used a fresh release process, fresh provider/session state, warm OS pages, and the same
fixture. Builds and benchmarks ran sequentially. Each comparison has five samples per case with alternating
order. A separate cache-population run precedes the component/gem comparisons; the Session comparison has
an additional discarded uncached warmup. No tool/source exclusions, hints or server defaults changed.

The component experiment uses the existing inventory scan, `index_file`, bounded `DeclIndex`, and ranking.
It compares fresh extraction against a JSON snapshot of records. Both cache variants validate the current
inventory and file metadata; the selected variant also reads and hashes **every source file** before reuse.
Timing includes fresh inventory discovery, snapshot loading, validation and complete index construction.
The first query is deliberately issued after full coverage, so this is not a measurement of the earliest
possible partial result or a rendered popup. Exact candidate targets and source-name ranges are checked for
`Searchable`, `User`, and `Topic`; all 58,844 serialized records have matching fingerprints across cases.

The integrated prototype is an isolated source snapshot: a read-only cache lookup in `index_file`, on the
existing background workers. The rest of the real Session, scheduling and provider remain in use. The same
binary runs with the cache environment unset for the baseline. Cache hits share declaration slices with the
ordinary index. This single-root prototype is not compiled into either production frontend.

### Local declarations and real server startup

| Standalone component median | Fresh extraction | Cached, full content validation |
|---|---:|---:|
| Complete local index | 7.364 s | 0.168 s |
| Complete index plus first ranked/validated query | 7.369 s | 0.172 s |
| Sampled process peak RSS | 262.2 MiB | 37.4 MiB |

The snapshot occupies **12.88 MiB** on disk for 7.85 MiB of accounted index records. Full validation adds
about 31 ms over metadata-only loading (0.168 vs 0.136 s). Metadata-only loading was rejected: replacing
`Alpha` with same-length `Omega`, preserving inode, size and nanosecond mtime, returned stale declarations.
Content validation reparsed that file. Additional checks matched a fresh index after additions, deletions,
renames and a new ignore rule. No cached record from outside the current inventory was admitted.

One population sample took 7.645 s through extraction/hash/record preparation, plus 25 ms to serialize/write.
That single sample does not establish cold-open cost preservation. The cache helps **reopening**; a project's
first uncached opening still needs extraction. No cache benefit is claimed for first-ever dependency discovery.

| Real Session/provider median | No local cache | Local cache |
|---|---:|---:|
| Complete local declaration index | 7.895 s | 0.488 s |
| Ruby LSP indexing → Ready | 14.451 s | 13.550 s |
| Sampled driver/server CPU time | 22.67 CPU-s | 13.95 CPU-s |
| Peak summed driver/server RSS | 732.6 MiB | 759.3 MiB |

Local completion was **16.2× faster**; server readiness improved **6.2%**, and sampled CPU use fell **38.5%**.
Local completion ranges were 7.79–8.10 s uncached and 0.481–0.493 s cached; server ranges were
14.14–14.57 s and 13.40–14.00 s respectively. Every Session indexed all 12,077 candidates and completed
both `User` → `Searchable` and `ActiveRecord::Base` → installed Rails source navigation. Definitions were
requested after Ready, so these are correctness checks, not time-to-first-semantic-answer measurements.

Memory is mixed: the driver's sampled peak fell from roughly 273 to 43 MiB, but the combined peak rose
by **26.7 MiB** in the five-pair startup trial. Per-process timelines locate that increase in Ruby LSP
(roughly 684–706 MiB uncached, 683–719 MiB cached), not retained local records. This is attribution, not a
proven cause. The stress comparison below did not retain the same combined-peak increase. Overall memory
preservation remains unproven; no production performance tradeoff has been accepted.

### Ruby gem cache: faster, rejected as implemented

The other prototype captures the declarations added while indexing Parser's 76 files and 11,144 records,
then replays them from a private local 1.40 MiB cache. Source SHA-256 hashes, exact Ruby/Prism/Ruby LSP versions
and effective indexing configuration participate in validation. Application and other dependency files are
still indexed normally. The hook does not modify installed server files or load a repository-supplied cache.

Five full Session pairs measured Ruby LSP Ready at **15.30 → 14.43 s** (5.7%). Parser's parse/record phase
was about 0.985 s; validating/loading the cache plus replay cost about 0.139 s. Peak combined RSS measured
784.1 → 805.7 MiB. Both cases use the same hooks; these instrumented values must not be compared directly
with the separate local-cache table. All ordinary navigation checks and Parser-record fingerprints matched.

A changed project context exposed the defect: define `Parser::Base` with an application singleton method
before loading the unchanged gem. Normal indexing retains one singleton-class entry; replaying the cached
records produces **two** entries with different owners/source locations. A cache keyed only by dependency
source and configuration cannot safely replay context-bound entry objects. This rules out shipping this
shortcut. A viable upstream design must replay declarations into current owners and preserve index mutations,
include hooks, aliases and project reopenings; that larger implementation was not attempted here.

### Sustained reuse, evidence and remaining work

Uncached stress completed 999 switch/nonempty-file-symbol cycles in
30.032 seconds; cached stress completed 993 in
30.044 seconds. Each retained one Ruby LSP PID and one Ready server.
Median symbol-response times were 29.96 → 30.15 ms.
Driver RSS ranges were 45.92–47.78 MiB and 43.32–45.20 MiB respectively. During active stress, summed RSS
ranged 846.1–951.3 MiB and 884.4–941.6 MiB, largely in the server. These short runs are not leak proof.
Unavailable process samples during exit were excluded, not interpreted as zero memory.

Workload **1** artifacts are retained under `target/perf/index-cache-prototypes/`: a reproduction README,
source/fixture/executable identities and hashes, the isolated source snapshot, release binaries, exact
prototype sources, Python collectors/checks, all per-sample configurations/status output, 25 ms-sleep
process-tree RSS/CPU timelines, raw results and summaries. Sampling overhead extends the interval; RSS
peaks can miss shorter transients, and sampled CPU excludes work from processes that disappear between samples.
The original tree's runtime sources and production executable files were preserved.

Correctness checks exercise collector success/failure, local stale/inventory handling and the gem-context
counterexample. Initial checker failures are retained: a lazy empty mixin list needed normalization before
semantic comparison, and a reused mutation fixture needed a fresh path. These were harness corrections.
The metadata and gem-context failures remain **rejected prototype behavior**, not waived correctness tests.

Before implementing the chosen local-cache design: version against extractor/schema changes, use a stronger
content digest, bound loading/storage/writing and ownership, preserve cancellation and current request
identity, fall back safely on corruption, and measure cold creation plus GUI/TUI startup/interaction/memory.
The prototype only validates one fixed-build/root format and uses standard-library 64-bit content hashes.
No LSP startup delay, query-priority scheduling or general gem cache was added. Native macOS/Windows,
physical scrolling/display latency, GPU memory, the private Rails app, Rails add-on boot and long sessions
remain unverified. The full runtime gate and adapter runs were not repeated for this isolated experiment
and documentation change; no production behavior changed.

### Branch-switch follow-up (2026-10-21)

A disposable four-file Git fixture exercised real `git switch` A → B → A with the measured local-cache
component binary. B modified one file, added one, removed one and renamed one. Reopening unchanged A reused
4/4 files. B reused 1/4 and parsed 3/4; its complete declaration fingerprint matched a fresh index. Returning
to A also reused only 1/4 and parsed 3/4, even though SHA-256 verified all current source bytes matched the
original A. Correctness held, but the prototype requires metadata equality before considering the content
hash, so checkout-generated inode/mtime changes waste reusable records. This is a tiny correctness/reuse
probe, not another large-project timing claim or a live-window branch-checkout test.

The proposed improvement is a bounded store keyed by actual source contents, language/parser mode and
extractor/schema identity, plus a separate current path-to-record inventory. A timestamp change would
trigger validation; matching content could still reuse declarations. Keeping recent content versions permits
switching back to A without parsing A again; renames can reuse records when parsing context is unchanged.
This has not been implemented or benchmarked. Git can help prioritize changed paths, but staged blob IDs
are not proof of current disk bytes; checkout attributes can transform those bytes
([Git attributes](https://git-scm.com/docs/gitattributes)).

A separate release shared-Session probe, with the experimental cache **disabled**, exposed an existing
recovery gap. It suppresses automatic watcher completions to simulate lost notifications, then injects
an explicit overflow notification. After rewriting Alpha → Omega and waiting for the inventory rescan,
Find Declaration still returns Alpha (one hit) and not Omega (zero). An exact-file notification fixes the
index (zero Alpha, one Omega). A subsequent rewrite Omega → Sigma followed by a directory-only structural
notification similarly leaves Omega in the index. Inventory rescans retain records for surviving paths;
they do not revalidate their contents. This establishes stale symbol listings, not a proven incorrect jump:
opening a result has separate validation. The precise-file path works; full/coarse invalidation needs repair
before making live-checkout guarantees for persistence. No production fix was applied during this exploration.

Artifacts are under `target/perf/index-cache-followup/`: the actual-branch script, fixture and result hashes,
release watcher probe source/executable identity, raw output and assertion logs. The watcher probe used
synthetic coarse/lost events and started no LSP. Native macOS notifications, a large live Rails checkout,
rapid successive checkouts, LSP recovery and bounded multi-version-cache behavior remain unverified.

## Checkout and watcher recovery (2026-10-21)

The shared reader core now invalidates surviving files after directory events and watcher overflow,
refreshes an open declaration search, rejects obsolete indexing/ranking work, and reconciles additions,
deletions and renames with client-watched language servers. Exact file events remain targeted. Both
adapters use this behavior; GUI and TUI regressions exercise actual keys, displayed results, narrow layouts
and opening the updated declaration. This fixes the live-update gap found above; no persistent cache is shipped.

Watch registration is distinct from a source change. Existing completed records are checked against their
live-file stamps off the UI thread, unfinished reads are retried, and inventory reconciliation catches files
created or removed before registration. Unchanged records retain their shared ownership. These existing
stamp checks are not content validation for persistent storage. Actual directory changes and event loss
rebuild the affected local declarations, using the existing worker queue and cancellation rules.

### Real Rails checkout

A disposable clone of the same pinned Discourse 3.5.0 fixture runs Ruby LSP 0.26.11. Two fixture commits
change a constant, add a file, delete another and rename a third under `lib/lenz_watch_demo`. An untracked
caller outside that subtree references the old and new constants for exact definition checks. The test
performs actual A → B → A checkouts twice in one live Session. It suppresses OS watcher completions and
injects a directory event for B and overflow for A, so precise notifications cannot mask recovery defects.

| Checkout notification | Local declarations complete | Ruby symbol and exact-definition checks complete |
|---|---:|---:|
| Directory, two transitions to B | 0.109 / 0.104 s | 0.606 / 0.667 s |
| Overflow, two transitions back to A | 7.537 / 7.584 s | 8.324 / 8.368 s |

Times start before dispatching the synthetic notification, after checkout has returned. Synchronous dispatch
took 11.49–13.31 ms; full rebuild work runs in the background. The metric called `ruby_symbols_seconds` in
the raw result includes the later exact-definition assertions, not just a symbol query. All transitions
restore 12,082 local candidates, reject removed names/paths, preserve an unaffected sibling and resolve only
the current constant. The 31.173-second run retains one running Ruby LSP PID after its separate version probe.
This is a shared-core/provider completion measurement, not frontend display latency or native notification proof.

This test found a second real defect: Ruby LSP 0.26.11 ignores watched-file changes for an open document and
defers indexing updated text until a document request. Closing that document first can leave its declarations
missing after checkout. Changed open Ruby documents now request document symbols to refresh that index;
closing a dirty document reports its disk state again after `didClose`. Internal refresh replies do not use
a navigation request lane, and ordinary file opening/switching adds no refresh request. Scripted protocol tests
cover ordering and navigation isolation; the real checkout passes exact old/new-definition checks.

### Startup, interaction and memory

The final default-coverage comparison uses four alternating pairs of fresh release Sessions, identical global
provider settings, warm OS pages and the same installed Ruby/bundle. Builds and measurements ran separately.
Every sample indexed all 12,077 original candidates and navigated from `User` to `Searchable` and from
`ActiveRecord::Base` into the installed Rails source. Separate message-count trials observed zero startup
`workspace/didChangeWatchedFiles` notifications in both binaries.

| Median | Before | After |
|---|---:|---:|
| Complete local declaration index | 7.262 s | 7.377 s |
| Ruby LSP indexing → Ready | 12.830 s | 12.610 s |
| Peak summed core/server RSS | 740.1 MiB | 740.9 MiB |
| Reader GUI first captured image | 156.04 ms | 154.83 ms |
| Reader GUI wheel → changed captured image | 26.61 ms | 24.17 ms |
| Reader GUI settled RSS | 167.40 MiB | 167.32 MiB |
| Reader TUI first source bytes | 6.04 ms | 6.20 ms |
| Reader TUI arrow → output bytes | 0.410 ms | 0.410 ms |
| Reader TUI settled RSS | 13.94 MiB | 13.88 MiB |

The local-index median is 115 ms higher; ranges overlap (7.175–7.367 s before, 7.280–7.422 s after), so this
does not establish exact timing preservation or a local-index speedup. Ruby startup ranges are 12.249–12.966 s
and 12.416–12.888 s. An earlier four-pair run with equivalent default settings saved globally before and
per-project after measured combined peak RSS about 724→760 MiB. The identical-configuration repeat above
did not retain that median shift; individual combined peaks span 726–802 MiB, with most variation in Ruby.
Both sets remain in the record. No intentional performance tradeoff has been accepted.

The GUI comparison has four alternating pairs after one discarded warmup pair, using a private Xvfb display,
lavapipe and a 20,000-line file. Image timing includes capture overhead. Its 10.041 / 10.049-second wheel
stress completed 163 / 159 cycles; RSS ranged 167.46–168.90 / 166.91–168.36 MiB and returned to the starting
stress value in both cases. The four-pair TUI comparison uses a 100,000-line file and an owned PTY. Its
5.043 / 5.041-second status-dialog stress completed 61 cycles each; RSS ranged 14.14–14.19 / 13.98–14.04 MiB.
Fixture bytes and terminal restoration are asserted. These frontend workloads do not run a concurrent Rails
server, and PTY receipt does not measure emulator display time.

The real-provider reuse comparison completed 1,062 / 1,041 file-switch/nonempty-symbol cycles in
30.082 / 30.036 seconds, retaining one Ruby LSP PID and one Ready server in each process. Median symbol
responses were 28.49 / 28.83 ms. Driver RSS ranged 45.16–47.13 / 45.23–47.12 MiB; summed process RSS during
the final stress interval ranged 768.2–923.3 / 757.1–957.5 MiB. The higher server-dominated stress maximum is
retained as a limit of this single pair, not hidden by the startup medians or interpreted as a proven leak.

### Rejected approaches and verification record

The first candidate treated registration like a source rewrite. Local startup regressed 8.204→9.430 seconds
and combined peak RSS 726.3→893.8 MiB, so that approach was rejected. Distinguishing registration removed the
unnecessary rebuild. A further red regression found that unchanged rescans requeued an already running
chunk; enqueue now distinguishes existing work from a newly invalidated path. Bulk removal also compacts
the pending queue once instead of rescanning it for every deleted file.

Initial Cargo artifacts accidentally came from the earlier cache prototype despite the copied baseline source.
Unexpected build warnings exposed that mismatch before accepted timing. Reader packages were cleaned and
both source snapshots rebuilt separately for core, GUI and TUI to avoid feature unification. Earlier frontend
measurements overlapping corrective builds are excluded. An intermediate GUI RSS difference of about 2 MiB
did not recur in the final standalone comparison above.

`scripts/ci-local.sh` passes on the final runtime: workspace/core, both GUI adapters, TUI event/render checks,
Clippy, collector checks and standalone releases. Reader core has 331 passing unit tests and 89 passing
semantic integration tests; intentionally ignored tests and one optional Python collector check remain skipped.
The red/green tests cover coarse changes, overflow, registration gaps, deletions, prefix boundaries, two rapid
changes and held stale workers. The initial real-LSP harness incorrectly treated fuzzy workspace-symbol
matches as exact; exact definition assertions replaced that assumption and exposed the actual open-file defect.

Workload **1** artifacts are retained under `target/perf/watch-invalidation/`: before/after source manifests,
an isolated diff against the starting dirty tree, release executable hashes, fixture commits and caller hash,
exact collectors/configurations, raw samples, process RSS/CPU timelines, red/green logs, the diagnosis record
and final gate log. `common-results.json` is the final Rails startup comparison; `gui.json`, `tui.json` and
`live-result.json` retain final frontend/checkout samples. Rejected and intermediate trials are retained separately.
This experiment adds no production benchmark instrumentation or dependency.

Native macOS/Windows notifications, physical scrolling, GPU memory, the private Rails app, Rails add-on boot
and long sessions remain unverified. Short RSS runs are not leak proof. Full event loss still needs a background
rebuild; bundle/runtime changes may require **Restart Language Server**. The next cache step remains a bounded,
content-validated store with recent versions and a separate current inventory, measured independently of this fix.

## Configurable index reuse and rebuild (2026-10-21)

The shared reader core now implements the selected content-validated declaration cache. **Reuse Saved
Declaration Index** defaults on and can be disabled in GUI/TUI F2, the command menu, or `persistent_index`
in Reader settings. The Ruby tooling-gem profile also defaults on, with explicit provider settings taking
precedence. Its project action is a reversible toggle that preserves unrelated settings. **Rebuild Project
Index** clears saved/current declarations, refreshes open documents and restarts the browsing project's
language servers. It works with cache reuse off; source files and settings are preserved. Server-owned
persistent workspace/build directories are not erased. See [controls](providers.md#saved-declarations-and-rebuilding).

Cache keys use SHA-256 of current source bytes and language; a build-time identity covers extraction,
decoding, cache format and locked dependencies. Paths come exclusively from the current inventory. Renames
and branch returns can reuse content despite changed timestamps. A checksum detects corrupted snapshots;
incompatible, corrupt or unavailable caches fall back to parsing. The cache retains recent content versions
within a 32 MiB accounted memory budget per active project, sharing declaration slices with the live index.
Completed snapshots are capped at 64 MiB per project; saving prunes storage to eight project directories.
Hashing, loading, writes and removal use existing background workers. Rebuild changes the request identity
and a persisted cache epoch, so older work or another process's late snapshot cannot restore the cleared index.

### Release Rails comparison

The pinned Discourse 3.5.0 checkout and installed Ruby 3.4.10/Ruby LSP 0.26.11 bundle are unchanged from the
preceding experiment. Four measured rounds alternate case order; each uses a fresh process and configuration,
warm OS pages and default project-file coverage. Warm cases copy only the saved local declarations from a
separate population run; language-server processes and state start fresh. Core, GUI and TUI builds are separate,
and builds/tests do not overlap accepted measurements. Every run indexes all 12,077 candidates and verifies
application-concern and installed ActiveRecord definition targets.

| Median, four samples per case | Local declarations complete | Ruby LSP Ready | Sampled CPU | Peak summed core/server RSS |
|---|---:|---:|---:|---:|
| Previous implementation, full coverage | 8.122 s | 14.046 s | 22.84 CPU-s | 726.6 MiB |
| New implementation, both optimizations off | 8.074 s | 14.258 s | 23.05 CPU-s | 741.1 MiB |
| Both on, no saved declarations | 8.054 s | 12.070 s | 20.85 CPU-s | 683.6 MiB |
| Both on, saved declarations | 0.523 s | 11.611 s | 12.41 CPU-s | 682.0 MiB |
| Saved declarations, tooling exclusions off | 0.522 s | 13.029 s | 13.83 CPU-s | 750.8 MiB |

With both on, reopening gives **15.5× earlier complete local coverage**, **17.3% earlier Ruby readiness** and
**45.7% lower sampled CPU** than the previous implementation. The first uncached opening still extracts local
declarations; tooling exclusions improve Ruby readiness by 14.1% in this comparison. Warm local completion
spans 0.519–0.532 seconds. These are shared-core/provider completion times, not a rendered symbol popup or
the earliest partial semantic answer. The snapshot is 12.20 MiB and contains 12,026 distinct content entries;
identical files share records while each current path remains in the live index.

Memory is configuration-dependent. The warm driver's sampled peak falls from 273.5 to 55.0 MiB, but cache
reuse alone still raises the combined startup peak. Four additional alternating rounds with full Ruby coverage
measured previous / both-off / cache-only combined peaks at **727.7 / 735.5 / 762.9 MiB**, and Ruby Ready at
**13.343 / 13.177 / 13.017 seconds**. Local completion was **7.591 / 7.577 / 0.516 seconds**. The cache-only
increase was therefore retained; it is not memory-preserving in these samples. Most combined variation is
in Ruby; warm driver residency near exit was also about 6–7 MiB higher in the first matrix. Near-exit samples
are not an idle-memory measurement. Both defaults enabled had lower combined peaks and stress residency.
Retain this tradeoff when choosing full tooling-gem navigation; no general memory-preservation claim is made.

### Live checkout, rebuild and sustained reuse

The same disposable A/B Discourse commits perform A → B → A twice with one live server, suppressing precise
OS completions and injecting directory or overflow events. Exact definition checks verify both present and
removed constants; all transitions retain 12,082 local candidates, including the controlled fixture files.

| Notification | Local declarations complete | Ruby symbol and exact-definition checks complete |
|---|---:|---:|
| Directory, transitions to B | 0.145 / 0.109 s | 0.501 / 0.622 s |
| Overflow, transitions back to A | 0.516 / 0.526 s | 5.557 / 5.831 s |

The earlier uncached overflow probe required about 7.6 seconds for local coverage. Cache reuse now validates
all current source contents and reuses matching records; Ruby still processes the full changed-file reports.
An explicit **Rebuild Project Index** then discards the saved records, starts a new server, and completes both
indexes in **11.335 seconds**, preserving fixture source bytes and expected symbols. This single live run
lasted 36.422 seconds; it measures recovery after synthetic notifications, not native OS delivery reliability.

The separate before/after reuse runs complete **1,056 / 1,038** file-switch/nonempty-symbol cycles in
**30.012 / 30.020 seconds**, with one Ready server and one Ruby LSP PID throughout each active interval.
Median symbol responses are **28.88 / 28.92 ms**. Driver RSS ranges **44.04–45.96 / 44.45–46.28 MiB**;
summed RSS during stress ranges **730.4–937.9 / 669.7–844.2 MiB**. The after run uses both defaults and starts
without a saved local cache. These short samples do not prove leak freedom.

### Frontend checks and investigation repeats

The first four GUI pairs measured first captured image **160.75→170.62 ms**, wheel feedback **24.91→25.40 ms**
and settled RSS **165.20→165.31 MiB**. Six further alternating pairs did not retain the startup difference:
**162.54→158.95 ms** first image, **25.09→24.88 ms** wheel feedback and **167.04→167.20 MiB** RSS.
Both rounds use owned Xvfb/lavapipe, one discarded warmup pair and a 20,000-line file. Image timing includes
capture overhead. Ten-second wheel stress ranges were **165.25–165.44 / 165.42–165.61 MiB** initially and
**167.02–167.08 / 167.38–167.38 MiB** in the repeat, with actual durations retained in the raw samples.

TUI first source-byte receipt measured **6.84→7.43 ms** initially and **6.11→6.61 ms** in six further pairs.
To distinguish startup work from the longer interaction workload, a separate startup-only check used sixteen
alternating pairs: **6.16→6.05 ms**, with child CPU **7.43→7.06 ms**. Twelve additional pairs pinned to one CPU
measured **7.45→7.46 ms**, with child CPU **7.04→7.05 ms**; these include the `taskset` launch wrapper and are
not directly comparable to unpinned absolute times. The earlier startup gap was not retained by those checks;
exact preservation is not established. Arrow feedback measured **0.427→0.433 ms**, then **0.410→0.411 ms**.
All terminal cases use a 100,000-line source and an owned PTY; receipt is not emulator display latency.
Two five-second status-dialog stress rounds completed 61 cycles per case, with RSS ranges under 0.06 MiB
within each run. Source bytes and terminal restoration are asserted. Frontend timing cases do not run a
concurrent Rails server; native trackpads and GPU memory remain separate checks.

The full `scripts/ci-local.sh` gate passes on the measured runtime: 337 reader-core unit tests, 91 semantic
integration tests, 129 Reader GUI tests and 70 TUI/GUI parity checks, plus workspace/Diff checks, Clippy,
collector checks and standalone releases. Intentional ignored tests and one optional collector check remain
skipped. Actual F2 input covers both appearances and narrow layouts, persists opt-outs, toggles exclusions
off/on without resetting unrelated settings and invokes rebuild. Core checks cover corruption, extractor
changes, metadata-preserving edits, rename/branch content reuse, storage bounds, cancellation, late writers,
held old workers, project switching and refreshing a changed open document after lost events.

A final review found a pending symbol search could retain its spinner across rebuild. The red/green regression
now finishes searches started before or during clearing with a retry message. Initial performance collection
was interrupted for that fix; its binaries/raw partial runs are retained but excluded from the tables. Earlier
test-harness fixes opened an explicit browsing root and counted all declaration candidates, including Gemfile.

Workload **1** artifacts are in `target/perf/index-settings/`: starting/final source manifests and isolated diff,
separate release binaries and hashes, commands/configurations, the checksum-protected seed cache, raw startup
and investigation samples, process RSS/CPU timelines, real checkout/rebuild output, test failures and final
gate log. `results.json` is the five-case startup matrix; `repeat-results.json` retains the memory investigation;
`repeat/` retains the frontend repeats; `tui-startup-investigation.json` retains startup-only controls.
Sampling can miss brief peaks and CPU from processes that exit between samples. Native macOS/Windows,
physical display/scrolling, GPU memory, the private Rails project, Rails add-on boot and long sessions remain
unverified. No Ruby-owned declaration replay cache, automatic dependency installation or source-file exclusion
profile was added.

## Cross-language index validation (2026-10-21)

The existing implementation is unchanged. Validation now covers the other nine supported declaration
grammars: Rust, Python, Java, Kotlin, JavaScript, TypeScript, TSX, HTML and CSS. Ruby tooling-gem exclusions
apply only to Ruby. Languages with syntax highlighting but no declaration extractor/provider are outside
this matrix.

### Local declaration indexing

Each disposable corpus contains 1,200 generated source files plus one or two entry/definition files,
14,401–15,604 declarations and no syntax errors. Source contents are unique; these are controlled corpora,
not nine production applications. A normal, uninstrumented release Session opens the root and waits for
complete inventory and local declaration coverage. No document or language server opens in this timing
matrix. Three rounds alternate off/cold/warm order, with a separate population run per language. Each run
has a fresh process and private configuration; warm runs copy only saved declarations. OS pages remain warm.

| Grammar | Cache disabled | First cache creation | Saved declarations | Saved vs disabled |
|---|---:|---:|---:|---:|
| Rust | 172 ms | 183 ms | 22 ms | 7.7× |
| Python | 174 ms | 187 ms | 21 ms | 8.1× |
| Java | 175 ms | 186 ms | 23 ms | 7.7× |
| Kotlin | 347 ms | 360 ms | 22 ms | 15.7× |
| JavaScript | 169 ms | 179 ms | 22 ms | 7.6× |
| TypeScript | 222 ms | 227 ms | 22 ms | 9.9× |
| TSX | 219 ms | 232 ms | 22 ms | 10.1× |
| HTML | 164 ms | 179 ms | 21 ms | 7.6× |
| CSS | 167 ms | 174 ms | 22 ms | 7.7× |

Values are medians of three samples. Reuse improves every corpus, while initial cache creation adds
5–16 ms (2–10%) here. Snapshots occupy 2.81–3.05 MiB per project. The process-tree collector samples every
20 ms, so these short runs cannot establish precise peak or settled-memory differences. Local completion
is not a rendered result or full semantic readiness; the long-session and frontend measurements from the
preceding implementation remain separate evidence.

### Real language servers and recovery

A second run per grammar uses the same corpus, saved declarations and an installed real language server.
Every scenario checks a nonempty local result, exact semantic definition target and selected name,
nonempty file symbols, A → B → A source rewrites with injected watcher overflow, absence of the old local
name, **Rebuild Project Index**, and persisted cache opt-out. After rebuild the exact definition still works
and one project server remains. HTML checks an embedded script function and CSS a custom property, within
their supported scope. All original fixture files retain identical bytes; no additional files appeared in
the source trees.

| Provider / corpus | First exact definition verified | Rebuild through exact definition | Result |
|---|---:|---:|---|
| rust-analyzer `9074e9b4c6` / Rust | 1.966 s | 1.462 s | Pass |
| Pyright 1.1.414 / Python | 0.280 s | 0.331 s | Pass |
| JDT LS 1.62.0-SNAPSHOT / Java | 2.764 s | 2.345 s | Pass |
| Kotlin LSP 263.6379.0-EAP / Kotlin | 10.324 s | 5.434 s | Pass |
| typescript-go 7.0.2 / JavaScript | 0.071 s | 0.172 s | Pass |
| typescript-go 7.0.2 / TypeScript | 0.069 s | 0.182 s | Pass |
| typescript-go 7.0.2 / TSX | 0.078 s | 0.198 s | Pass |
| VS Code 1.142.0 HTML / HTML | 0.433 s | 0.442 s | Pass |
| VS Code 1.142.0 CSS / CSS | 0.079 s | 0.193 s | Pass |
| typescript-language-server 6.0.1 + TypeScript 6.0.3 / TypeScript | 0.675 s | 0.648 s | Pass |

These are single correctness runs, not comparative language-server benchmarks. First definition starts at
project open; rebuild starts at command dispatch and includes complete local coverage and the checked
semantic answer. Java and Kotlin were still reporting background indexing when the initial exact definition
worked. Thus neither column claims complete server indexing, and no semantic-startup speedup is inferred.
Rebuild preserves server-managed build/workspace caches, as designed. The Rust, Java and VS Code versions
are newer than the provider catalog's broad compatibility tests, so their existing untested-version notices
remain; this focused check does not replace that broader coverage.

### Regression coverage, evidence and limits

The normal core suite now includes
`saved_declarations_match_fresh_extraction_across_languages_and_content_changes`. It checks all nine
grammars against fresh extraction, comparing names, kinds, ranges, nesting and Unicode/CRLF offsets across
save/reopen, opt-out, equal-length edits with preserved mtimes, and restoring source followed by rebuild.
It also verifies syntax-error counts. An initial test incorrectly assumed the compact Kotlin CRLF fixture
would parse without warnings; fresh extraction itself reports a warning. The corrected oracle compares
fresh and cached warnings, while all nine large fixtures parse without warnings. The disposable probe's
initial build used a private setting accessor; it now uses the existing public action catalog. No production
API or runtime change was needed.

All ten real-server scenarios pass. The full `scripts/ci-local.sh` gate passes with the new regression;
GUI/TUI parity checks remain part of that gate. This turn adds test and documentation coverage only.
Python's PATH shim had no configured runtime, so the test names the already-installed Pyright executable;
no user runtime settings or provider installations were changed.

Workload **1** evidence is retained in `target/perf/index-languages/`: `prepare.py`, fixture manifests and
specifications, production release `probe` with retained source/build logs, `measure.py`, per-run settings,
raw `results.json` and RSS/CPU timelines, `summary.json`, `lifecycle.json`, source/executable identities,
initial harness failures and the final gate log. Builds/tests were complete before measurements; the final
gate ran after all measurement processes exited. Lifecycles lasted 0.49–19.63 seconds, not a sustained stress
test. This does not verify native macOS/Windows, scrolling, GPU memory, long sessions, dependency-heavy
production projects or full semantic indexing speed in other languages. Existing shared adapter checks are
reused; no new language-specific GUI input workload was run.

## Further indexing experiments, 2026-10-21

Three isolated release prototypes were tested against the current implementation on the same Linux machine.
The production runtime and GUI/TUI executables were not changed. These are measurements of shared-core
work and a private helper, not shipped startup or display-latency improvements. OS pages were warm;
provider installations, bundle and fixture revisions were held fixed. Source copies, hashes, binaries,
private configurations, raw samples, failed initial builds and collectors are retained in
`target/perf/index-next/` (workload version 1).

### Content comparison before coarse change notifications

Three rounds alternate the current implementation, full-project hashing, and directory-scoped hashing
(with a full scan after overflow). Medians below contain six checkout events per event type.

| Through exact semantic checks | Current | Full hashing | Scoped hashing |
|---|---:|---:|---:|
| Directory event after checkout | 0.548 s | 0.843 s | 0.636 s |
| Watcher overflow after checkout | 5.503 s | 0.823 s | 0.809 s |
| No-change overflow | 5.462 s | 0.739 s | 0.731 s |
| Missing fingerprint baseline | 5.587 s | 5.402 s | 5.323 s |
| Sampled peak summed RSS, complete scenario | 939.3 MiB | 839.5 MiB | 868.7 MiB |
| Sampled CPU, complete scenario | 64.53 CPU-s | 51.32 CPU-s | 50.40 CPU-s |

Overflow checks range from 5.123–5.810 seconds currently, versus 0.809–0.840 with full hashing and
0.800–0.828 with scoped hashing. Local overflow coverage improves from 0.522 to 0.348 / 0.344 seconds.
However, normal directory coverage regresses from 0.122 to 0.366 / 0.166 seconds. Full hashing costs about
267 ms per event; scoped discovery/comparison still costs 71 ms for three files. Initial fingerprint
capture costs another 203–204 ms. Neither candidate preserves ordinary-directory performance.

Complete scenarios, including initial startup, both checkout cycles, no-op/missing-baseline checks and
rebuild, last 46.436–48.318 seconds currently, 33.470–33.815 with full hashing, and 32.757–33.606 with scoped
hashing. Peak summed RSS ranges are 921.8–944.6, 837.3–851.5 and 837.3–882.2 MiB respectively. Explicit
rebuild medians are 11.474 / 11.413 / 11.080 seconds; these small differences do not establish faster rebuilds.
The CPU/RSS totals describe this entire recovery scenario, not just startup or one overflow.

The driver uses the real Session and Ruby LSP 0.26.11 on an owned Discourse checkout, with 12,082 local
candidates and 16,405 discovered files. Each run performs two A → B → A checkout cycles. It checks exact
local/server names, the exact definition target and selected name, failure to navigate to the removed
constant, and a single server. Timers start after checkout and include hashing, core recovery and the
semantic checks. They do not measure native filesystem event delivery. The raw `ruby_symbols_seconds`
field includes the definition checks too. Five explicit changed paths replace the coarse event in the
candidate; baseline `sent: 0` means the probe did not enumerate paths, not that no LSP notification occurred.

The no-change overflow and missing-fingerprint cases are also exercised, followed by explicit rebuild.
Missing baseline information retains conservative full recovery. Fingerprint self-checks cover same-size,
preserved-mtime changes, rename and missing snapshots. Hashing is synchronous in this disposable driver:
production integration would require the existing background workers, cancellation and request identities.
Discovery/read errors invalidate the snapshot. This is not a proof against arbitrary concurrent mutations.

### Borrowed UTF-8 validation on saved declaration hits

The candidate reads and hashes source bytes as before, but validates cached UTF-8 declarations against
borrowed text with a streaming newline iterator. It avoids creating a decoded Document and allocated line
map on a valid hit. BOM/legacy encodings and invalid records retain the original decoding/validation path.
Byte, line and cancellation limits remain. Cache format is unchanged; each executable seeds its own cache
because source changes alter the extractor identity.

| Discourse local coverage | Current | Prototype |
|---|---:|---:|
| Saved declarations | 250 ms | 237 ms |
| First cache creation | 7.337 s | 7.365 s |
| Reuse off | 7.270 s | 7.288 s |
| Sampled peak RSS, saved declarations | 44.6 MiB | 38.2 MiB |

These are three alternating samples per mode, without a language server. Warm ranges are 249–253 versus
237–240 ms. The roughly 13 ms / 5% gain is small; first cache creation is 29 ms slower, while opt-out ranges overlap. Nine other generated language
corpora stay around 22–23 ms with mixed sub-millisecond shifts, so a general speedup is not established.
Ninety-two runs include separate seeds. RSS is sampled every 25 ms and is only a coarse short-run peak,
not settled memory or leak evidence. This optimization is a lower priority than avoiding redundant LSP work.

The prototype passes 340 core tests, with 16 existing ignored tests, plus the fingerprint self-check.
A complete Discourse pass compares fresh extraction with cold and warm cache results for all 12,077
candidates. Existing cross-language tests and new encoding/offset/line-boundary checks pass. This does not
replace a frontend performance gate if the implementation is subsequently adopted.

### Retaining a server across fresh client processes

A private Unix-socket helper keeps a single headless Session. Every attach/detach command comes from a
new client process. The comparison recreates the Session/server on each attach; the candidate retains
it, closes its tabs on detach, hashes source contents on reconnect and validates exact definitions.
Watcher notifications are suppressed to exercise changes missed while disconnected.

| Checked operation | Measured time |
|---|---:|
| Fresh server on reopen, median of three | 12.329 s |
| Retained server on reopen, median of three | 0.956 s |
| Checkout while disconnected, B / A | 1.029 / 0.994 s |
| Edit injected after the first reconnect scan | 1.459 s |
| Explicit rebuild | 12.071 s |

Reopen ranges are 12.196–12.345 versus 0.944–0.957 seconds. Process identities confirm that retained
reopens use the same Ruby process and fresh reopens replace it. Initial attach remains 12.6–12.8 seconds.
A second content scan catches the deterministic reconnect race. Gemfile.lock change/restoration and a
provider launch-environment change each restart the server and pass the definition checks; they take
12.3–12.5 seconds. Actual runtime replacement was not tested.

The cost is resident memory after closing the client: after the extended scenario, helper plus Ruby use
**699.1 MiB** through a **29.906-second** sampled idle interval, consuming **0.09 CPU-seconds**. The configured
35-second expiry drops the Session and Ruby; after a 36-second wait the helper alone still uses **66.8 MiB**.
Explicit shutdown ends the helper, and the collector cleans its owned process group. Complete fresh/retained
scenarios last 49.930 / 118.215 seconds, with sampled summed peaks of 713.6 / 719.5 MiB.

This establishes a possible reconnect benefit, with a substantial memory tradeoff. Any product version
should be opt-in and expire automatically. The prototype retains a whole Session, not just LSP transport;
it supports serial clients only and does not establish independent simultaneous-window snapshots, full
server/toolchain identity validation, crash recovery or daemon security. No production setting was added.

### Decision and remaining validation

Prioritize avoiding unchanged-file notifications during overflow recovery: the measured semantic benefit
is about **6.8×**. Do not adopt the extra directory scan. A production experiment should reuse existing
inventory/index reads to maintain fingerprints, do validation in background work, and fall back to full
recovery whenever its baseline is incomplete or races a change. Keeping fingerprints current after normal
changes is essential; skipping that maintenance could miss a later checkout back to earlier content.

Defer the borrowed-text cache path: its 13 ms gain is small, smaller-language results are inconclusive,
and first cache creation has a small measured cost. Keep server retention as a separate opt-in design
candidate: its roughly **12.9×** reconnect benefit comes with around 700 MiB of idle residency here.
Neither tradeoff is enabled in the app.

Production source hashes remain unchanged from the pre-experiment manifest. Only README/report/notebook
text is updated in the normal tree. Local links and whitespace are checked; the full application gate and
GUI/TUI rendering/input workloads were not rerun for these isolated prototypes. Native macOS/Windows,
physical scrolling, GPU memory, multi-project resource limits and long sessions remain unverified. Summed
process RSS can count shared pages more than once. The retained-server experiment covers Ruby only;
the local cache experiment covers the ten declaration grammars.

## Content-aware overflow recovery, 2026-10-21

The shared reader core now retains the content digest already computed for each successfully indexed file.
After watcher overflow, existing background index reads verify which contents changed before notifying Ruby
LSP. This adds no extra directory walk, file read or content hash, and leaves ordinary file/directory notifications
unchanged. Both adapters use this implementation. **Reuse Saved Declaration Index: Off** also disables this
filter; **Rebuild Project Index** still clears local saved/current records and restarts the project's servers.

Unknown, unreadable and oversized files retain notifications. A byte-budget stop releases pending reports;
initializing servers and overlapping overflow recovery use full notifications. A precise edit arriving during
recovery cancels that file's comparison. Root/rebuild identities and the indexer's requeue checks reject stale
work. Registration-triggered follow-up scans preserve the baseline until their final inventory. Retained
digests count toward the existing live-index byte budget; the temporary comparison map is bounded by that
index. Other providers currently watch their own files, so this change does not filter their native watchers.

### Method and correctness

Workload **1** uses paired release executables on the same Linux machine, Ruby 3.4.10 and Ruby LSP 0.26.11.
The static Discourse revision is `05a304006600f36c3e45d19c9c5919f43f5541c9` (12,077 local candidates).
The owned checkout fixture has 12,082 candidates and 16,405 discovered files. Three alternating pairs each
perform two A → B → A cycles, with a directory event for B and an overflow for A. Exact local/server names,
definition target and selected name, failure to resolve the removed constant, and a single server are asserted.
Each scenario also checks no-change overflow, explicit rebuild, and overflow after opting out of saved reuse.
Events are injected after real Git checkout while native events are suppressed; timing excludes Git and does
not measure native watcher delivery. `ruby_symbols_seconds` includes the definition checks too.

Startup uses three alternating pairs per cache mode, plus a separate seed for each executable. OS pages are
warm; “cold” means no saved Lenz declarations. Each startup verifies a project definition and an ActiveRecord
dependency definition. The 25 ms process-tree sampler retains RSS and CPU timelines. Summed RSS can count
shared pages more than once; sampled peaks are not settled memory or leak evidence. Builds and tests finish
before timing. The two source copies, exact probes/collectors, binaries, hashes, private configs, fixture
identities, raw samples and logs are retained under `target/perf/overflow-content/`.

Regression tests exercise scan-first and index-first completion, registration during recovery, exact edits,
directory edits, branch reversals, overlapping overflow, equal-size edits with preserved mtimes, index limits,
read failure and opt-out. They include exact paths accompanying an overflow. Existing GUI/TUI checks exercise
declaration refresh through actual keys and presentation. No new adapter control is needed.

### Recovery and startup measurements

The initial three alternating checkout pairs contain six events of each type per executable. Medians:

| Operation or complete-scenario metric | Before | After |
|---|---:|---:|
| Overflow, through exact semantic checks | 5.560 s | 1.137 s |
| Directory event, through exact semantic checks | 0.576 s | 0.501 s |
| Overflow, local coverage | 0.519 s | 0.505 s |
| Directory event, local coverage | 0.122 s | 0.126 s |
| No-change overflow, local and server symbols checked | 5.658 s | 1.244 s |
| Explicit rebuild | 11.461 s | 11.476 s |
| Overflow after opting out | 8.075 s | 8.928 s |
| Complete scenario duration | 57.973 s | 44.797 s |
| Complete scenario sampled CPU | 83.67 CPU-s | 69.06 CPU-s |
| Complete scenario sampled peak summed RSS | 983.48 MiB | 1,049.51 MiB |

Overflow dispatch itself remains **9.34→9.31 ms**; directory dispatch **11.91→11.78 ms**. The gain comes from
avoiding redundant server work after recovery, not faster initial parsing. Peak summed RSS before the
explicit server restart improves **920.50→713.26 MiB**. The higher full-scenario peak occurs later, during
rebuild/opt-out. That phase and initial startup differences prompted additional alternating samples.

Three additional checkout pairs retain the benefit: overflow **5.761→1.053 s**, directory events
**0.595→0.487 s**, no-change overflow **5.691→0.780 s**, total duration **60.862→47.865 s**, and sampled CPU
**87.72→75.06 CPU-s**. Full-scenario peak summed RSS reverses to **1,066.57→1,043.61 MiB**, with pre-restart
peaks **907.22→812.66 MiB**. However, rebuild medians are **11.622→12.269 s** and post-opt-out recovery
**9.105→10.226 s**. Those unfavorable timings are retained rather than folded into the favorable overflow
headline. The host load and an additional protocol/scheduling investigation are described below.

| Startup, initial three pairs | Before | After |
|---|---:|---:|
| Cold local coverage | 7.659 s | 7.637 s |
| Cold Ruby LSP Ready | 11.318 s | 11.598 s |
| Warm local coverage | 0.522 s | 0.529 s |
| Warm Ruby LSP Ready | 11.043 s | 11.189 s |
| Cold sampled peak summed RSS | 684.94 MiB | 686.65 MiB |
| Warm sampled peak summed RSS | 683.20 MiB | 685.75 MiB |

Four additional alternating startup pairs per mode give cold local **7.625→7.676 s**, cold Ruby Ready
**11.417→11.558 s**, warm local **0.522→0.526 s**, and warm Ruby Ready **11.034→10.932 s**. Cold Ready ranges
overlap (11.222–11.600 / 11.312–11.762 s); the warm Ready difference reverses. Repeated combined peak RSS is
684.52→679.34 MiB cold and 682.33→688.46 MiB warm, with overlapping ranges. Reader-only warm sampled peaks
are 53.94→55.95 MiB. First-readable-document medians remain about 1.4 ms; this is core availability, not paint.

Three standalone opt-out pairs give local coverage **7.638→7.560 s**, Ruby Ready **11.360→11.575 s**, and
combined peak RSS **667.16→678.32 MiB**. Small cold-server timing increases remain visible; these measurements
do not establish a cold-start improvement or exact memory preservation. A host snapshot during the repeat
shows other active development workloads and load near 12 on 32 logical CPUs. Our builds/tests were finished,
but the host was not exclusive; this limits attribution of small differences. Unrelated processes were left
alone. Both favorable and unfavorable samples are retained.

A separate diagnostic run relays and captures the real JSON-RPC traffic. After rebuild, both executables
send identical initialization options (apart from process ID), identical client-method counts and exactly
one opt-out notification containing the same **16,405 path/type pairs**. Thus the trace finds no extra
post-opt-out LSP requests. Time from that notification to the symbol request is 8.733 / 10.376 seconds;
the symbol response itself takes 1.231 / 1.252 seconds. The difference is predominantly before the client
finishes local recovery. These traced runs include relay overhead and are excluded from benchmark medians.
`check-traces.py` asserts protocol parity, and `protocol-check.json` records it.

Two final alternating pairs constrain only the owned benchmark processes to CPUs 8–11. The host remains
shared. Overflow medians are **6.930→1.404 s**, ordinary directory recovery **0.680→0.592 s**, and full-scenario
sampled peak RSS **1,072.11→1,069.47 MiB**. Opt-out recovery is 8.783 / 10.950 seconds before and
10.157 / 10.339 after: the second pair reverses the earlier difference, and the baseline range contains both
candidate results. Rebuild is 14.615 / 13.004 before and 14.540 / 15.099 after. These checks do not establish
an invariant opt-out/rebuild slowdown, but also do not prove exact preservation on an exclusive machine.
The robust result is less redundant overflow work, with ordinary-directory timings maintained in these runs.

Initial complete scenarios last 57.159–58.931 / 44.628–45.365 seconds; repeats last
57.892–67.682 / 45.445–52.668, and fixed-affinity scenarios 66.632–70.472 / 50.633–55.800. The shortest
correctness-checked overflow is 5.206 / 0.767 seconds in the initial set; all raw ranges remain in
`summary.json`. These are short repeated recovery workloads, not a long-session soak.

### Frontend and sustained checks

Four alternating GUI pairs after a warmup use the production Reader on an owned Xvfb display with lavapipe
and a 20,000-line file. Median first captured content is **156.05→157.52 ms**, wheel-to-changed-image
**25.17→20.55 ms**, and sampled RSS **167.23→167.84 MiB**. Capture overhead is included; these are not physical
trackpad/display measurements. The 10.038 / 10.019-second scroll loops complete 157 / 188 cycles. RSS ranges
are 166.59–166.59 / 167.54–169.14 MiB. The loops preserve source bytes.

Four production TUI pairs on an owned PTY and a 100,000-line file measure first content **6.57→6.29 ms**,
arrow feedback **0.401→0.399 ms**, and sampled RSS **13.84→14.04 MiB**. The separate TUI builds exclude GUI
dependencies. Both 5.038-second status-dialog open/close loops complete 61 cycles; RSS ranges are
14.04–14.07 / 14.13–14.18 MiB. File bytes and terminal restoration are asserted. PTY receipt is not terminal
emulator display latency. Neither frontend timing workload starts an LSP.

With real Ruby LSP, repeated file-switch/symbol requests run **30.059 / 30.086 seconds**, completing
**1,059 / 1,086** successful cycles while retaining one Ready server. Median switches take 0.092 / 0.092 ms,
file symbols 28.88 / 27.80 ms. Combined sampled peak RSS is 847.95 / 827.68 MiB. Median reader RSS in the
first/last five seconds is 49.32 / 50.01 MiB before and 50.87 / 51.58 MiB after. Retaining per-file digests adds
a small reader-memory cost within the existing accounted index bound; these short runs do not prove leak
freedom or long-session stability.

### Verification limits

`scripts/ci-local.sh` passes, including workspace/core, actual diff GUI adapter, Reader GUI/TUI and collector
tests. One final-gate attempt timed out in the existing GUI outline/declaration-navigation test. It passed
alone and in the complete rerun without a code change; the timeout's cause is unconfirmed and both logs are
retained. Earlier harness failures are retained too: one Ruby fixture expectation omitted Gemfile from the
candidate count, and a deterministic registration-during-overflow test exposed duplicate notifications from
a follow-up scan. The latter was fixed before measurement, and both completion orders now pass.

Native macOS/Windows, physical scrolling, GPU memory, Rails add-on boot, long sessions and real-server
performance in other languages remain unverified by this experiment. Shared extraction/cache tests still
cover all declaration grammars. The whole diff benchmark suite was not rerun for this Reader-only change;
its adapter tests remain in the local gate. Retained-server and borrowed-UTF-8 prototypes remain deferred.

## Language Servers menu and details popup (2026-10-21)

The shared menu now has nine management actions; diagnostic paragraphs are generated only when opening
**Server Details…** or copying the report. Both adapters reuse their frozen details popup, with Copy and
Close controls outside the scrolling body. The full local gate passes, followed by final affected-target
Clippy and terminal/GUI parity tests after a pointer-handler guard adjustment. Real mouse/key tests cover
menu entry, copying the complete report, dismissal, source preservation, indexing percentages, both
appearances and tiny layouts. Owned-display screenshots cover 800×500 and 380×260 in dark and light themes.

Production before/after binaries were measured on the same Linux host, using an owned Xvfb/lavapipe display
and a separate PTY. Four alternating GUI pairs after a warmup use a 20,000-line file; four terminal pairs use
100,000 lines. Median GUI first captured paint is **156.86→156.56 ms**, wheel feedback **24.75→25.05 ms**,
and sampled RSS **168.10→168.48 MiB**. F2/Escape feedback initially changes **20.78→24.53 ms**;
four additional pairs reverse this to **25.27→24.08 ms**, so a consistent menu slowdown is not established.
The repeat gives first paint **151.85→154.20 ms**, wheel feedback **23.31→21.69 ms**, and RSS
**168.02→168.13 MiB**. Capture and frame scheduling are included in GUI timings.

Terminal first content is **6.09→6.38 ms**, arrow-to-output feedback **0.43→0.41 ms**, and RSS
**13.98→13.98 MiB**. These measure PTY byte receipt, not emulator display latency. Repeated F2/Escape
stress lasts **10.05 / 10.05 seconds** in the GUI (163 / 157 cycles) and **5.04 / 5.04 seconds** in the
terminal (61 / 61 cycles), before / after. Candidate GUI RSS starts/ends at **168.51 / 168.53 MiB**
with a **169.95 MiB** sampled maximum; terminal starts/ends at **14.09 / 14.15 MiB**. Fixture contents stay
unchanged, input produces visible feedback, and the terminal restores its modes on exit.

Raw samples, collectors, logs, screenshots, source manifests and executable identities are retained in
`target/perf/server-menu/`. Short stress and RSS sampling do not establish leak freedom or GPU memory use.
Native macOS/Windows, physical trackpad input and long-session behavior remain unverified. The LSP indexing
benchmarks were not rerun for this menu-only change.

## Markdown file-header control (2026-10-21)

Preview/Source moved from the window toolbar to each Markdown pane's header. Both adapters obtain its
label and availability from the core, reserve space beside the tabs, and focus the clicked pane before
running the existing toggle. The local gate and an additional rendered-label assertion pass. Tests cover
both split orientations, inactive panes, long filenames, modal interception, source preservation, dark/light
appearances and tiny layouts. Owned-display screenshots cover source, preview and narrow split views.

Paired production measurements use the same Linux host and configuration, now with Markdown source files:
20,000 lines in the GUI and 100,000 in the terminal. Four alternating GUI pairs after warmup give first
captured paint **151.32→144.55 ms**, wheel-to-captured-change **21.24→25.93 ms**, sampled RSS
**171.05→170.80 MiB** and process peak RSS **198.27→197.85 MiB**. Four further pairs reverse the wheel
difference to **27.15→25.04 ms**, with first paint **141.69→143.60 ms** and sampled RSS
**171.10→170.92 MiB**. A consistent slowdown is not established. The 10.01-second scrolling runs complete
184 / 183 cycles, before / after; candidate RSS starts/ends at **170.88 / 171.05 MiB**.

Terminal first content is **6.97→6.59 ms**, arrow-to-output feedback **0.42→0.44 ms**, and initial sampled
RSS **173.56→173.51 MiB**. During 5.05 / 5.06 seconds of repeated F2/Escape, both complete 61 cycles;
RSS settles to **15.61 / 15.83 MiB**, before / after. The higher initial samples include background syntax
work. Fixture bytes remain unchanged and terminal modes restore on exit. GUI timings include capture and
frame scheduling; terminal timings end at PTY byte receipt. These workloads test source-view responsiveness,
not completed preview generation. Raw data, sources, identities and screenshots are in
`target/perf/markdown-control/`. Native macOS/Windows, physical display latency, GPU memory and long sessions
remain unverified.


## Dependency libraries in the explorer — 2026-10-21

Following a dependency definition adds a **Libraries** section, reveals the file and allows browsing sibling
files in both adapters, including when no project folder is open. Shared core commands own discovery, expansion,
activation, cancellation and root selection. Directory listings run through the existing worker pool; they
never extend the project inventory or declaration index. Cached rows are borrowed and painted by viewport.
Collapsing a directory cancels queued reads and releases its listings; expanding it reads current disk contents.
Changing projects rejects results from the previous generation. Listings are bounded at 2,000 entries per
folder and 20,000 overall; library roots and retrieved-source metadata are each capped at 128.

`target/perf/library-tree/ci-delivery.log` records the passing full local gate. Coverage includes package-root
recognition (Ruby, Rust, Python and npm), lazy listing, limits, cancellation, delayed results after re-expansion,
project replacement, ignored dependencies, retaining the original server, and virtual-source reopening and
split-pane snapshot reuse. Final targeted checks (`edge-cases.log`, `library-delivery.log`, `lint-delivery.log`)
also cover standalone navigation within the server's own workspace and non-Unicode filenames that could alias
displayable names. Actual GPUI and Ratatui input tests compare tree rows and resulting documents,
including standalone browsing, folder expansion, sibling opening, modal clicks, both appearances and tiny
layouts. Source bytes remain unchanged. Native macOS/Windows and live non-Ruby servers remain unverified;
Rust, TypeScript and Java protocol flows use the fake server.

A live Ruby LSP 0.26.11 session followed `Rake::Task` into the installed `rake-13.3.0` gem in both release
frontends. GUI screenshots cover Darcula, GitHub Light and narrow windows. In an earlier run the terminal's
first Ready status preceded a usable definition, requiring 4 attempts; the final run succeeded on its first
attempt. The collector retries while indexing, so its elapsed time is not definition latency.
This verifies library navigation, not a reduction in Ruby indexing time. Early collector failures are retained:
waiting for an offscreen heading, light-theme OCR failing on a visibly correct heading, and one private-display
connection failure. The corrected collector checks the returned file, brings the tree heading into view and
retains screenshots for visual inspection.

Measurements use the same Linux host and release configuration as the preceding reader experiments. GUI
trials use an owned Xvfb display, Mesa software rendering and 20,000 Markdown source lines; terminal trials
use an owned 120×36 PTY and 100,000 lines. Four alternating pairs use separate reader config/cache directories;
the GUI also has an excluded warmup pair. GUI timings include image capture; terminal timings end at received
bytes. Neither is physical display latency. These runs share the development host with unrelated work.

| Measurement (median unless stated) | Before | After |
| --- | ---: | ---: |
| GUI first captured nonuniform client frame | 153.90 ms | 167.08 ms |
| GUI wheel feedback | 27.80 ms | 29.96 ms |
| GUI sampled RSS | 168.78 MiB | 168.88 MiB |
| GUI maximum observed peak RSS | 192.32 MiB | 194.18 MiB |
| TUI first content bytes | 7.46 ms | 7.04 ms |
| TUI arrow feedback | 0.488 ms | 0.479 ms |

An intermediate GUI build repeatedly held about 3.8 MiB more executable pages, with similar heap use. Grouping
infrequent library mutations with `#[cold]` brought RSS back to the baseline range: earlier four-pair runs
measured 170.58→170.92 MiB and 170.87→170.94 MiB. Startup differences changed sign between those runs
(141.09→145.82 ms, then 158.29→155.31 ms). The final ordinary run above was 13.18 ms slower to capture a frame.
A control pinning only the owned driver, display and child processes to four allowed CPUs measured
161.47→166.63 ms startup, 28.36→27.57 ms wheel feedback and 168.79→169.21 MiB RSS. CPUs were not exclusive.
The first nonuniform frame may contain chrome before document content. Capture/scheduling variability and the
small sample count leave the startup difference inconclusive; these runs do not establish exact equality on
an exclusive machine or native macOS. No library discovery or listing is performed at ordinary startup.

Final GUI scroll stress ran 10.044 / 10.015 seconds, completing 139 / 136 down/up cycles; final RSS was
168.64 / 168.68 MiB. Terminal menu stress ran 5.012 / 5.069 seconds and completed 60 / 61 cycles.
Its initial memory sample overlaps background syntax work, so it is not settled document memory. Extracting
infrequent tree activation from the shared command dispatcher into a cold function reduced an intermediate
terminal executable-page increase: `extracted-tui.json` ended at 15.69 / 15.76 MiB. The final delivery run
ended at 15.73 / 16.71 MiB, prompting four alternating repeat pairs with executable/heap attribution.
Each repeat completed 61 cycles in 5.046–5.062 seconds. Median final RSS was 15.82 / 16.24 MiB;
anonymous mappings were 6.094 / 6.098 MiB, and executable mappings accounted for nearly all the remaining
increase. Thus the extraction reduced, but did not eliminate, the observed executable residency increase.
This is a measured memory limit of the final build, not evidence of a growing retained document cache.
The repeat's first content was 6.45 / 6.99 ms and arrow feedback 0.427 / 0.446 ms. RSS excludes GPU allocations
and server memory; these short workloads are not long-session leak proof.

The new shared-core workload reveals a 1,000-file library beside an unopened 10,000-file subtree, then repeatedly
collapses and expands the library. It asserts the subtree is never enumerated, cached listings are released on
collapse, the row count is exact and source bytes are unchanged. Final first reveal: **0.811 ms**;
**21,907 cycles in 10.000 seconds**, 0.428 ms median and 0.611 ms p95 per cycle.
Sampled RSS was 7.57 MiB initially, 8.12 MiB at sampled peak and 8.37 MiB finally, including
the retained timing samples. This measures in-process discovery/listing with a warm filesystem cache, excluding
worker queue delay, language-server resolution and frontend painting.

Raw samples, source snapshots/diffs, executable hashes, configuration, smaps attribution, failed experiments,
release binaries, screenshots and collector logs are retained under `target/perf/library-tree/`. The final
ordinary workloads are `delivery-gui.json` and `delivery-tui.json`; `delivery-gui-pinned.json` records the CPU
affinity control, `delivery-repeat-tui.json` and `delivery-repeat-smaps-*.txt` record terminal repeats and
attribution, and `core-perf-delivery.log` includes every timed library cycle. Long-duration stress,
native hardware display latency, GPU memory and real Java archive listing
remain unverified. Virtual sources expose only visited files and depend on the original server session; the
reader does not enumerate archive contents or load every installed dependency at startup.
