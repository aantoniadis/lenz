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
