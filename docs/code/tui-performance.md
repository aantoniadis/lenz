# Reader TUI workspace integration — 2026-10-18

The terminal reader now uses this workspace's `reader-core` and `lenz-content`.
The command-family trial exposes it through `lenz --tui [file-or-folder]` and
`lenz-code-tui`. Release packaging remains a separate migration.

## Correctness and integration

`scripts/ci-local.sh` passed: strict Clippy, separate Diff/Reader/TUI panic-site
budgets, workspace/core tests, serial content memory checks, actual Diff and
Reader GUI adapter tests, terminal event/render tests, the combined GUI/TUI
semantic replay, Python collectors, independent terminal release builds and
the existing stripped Diff executable size check.

The reader TUI suite has 26 passing tests with `gui-test`, including real
constructors, empty documents, dark/light and narrow rendering, modal capture,
UTF-8 selection/copy, Markdown/images, stale results, cancellation and independent
views. The shared replay checks selection, find, splits, reload and modal outcomes
through both adapters' inputs, together with unchanged source bytes. This covers
those scenarios, rather than claiming every possible interaction is equivalent.

The staging script built all four applications. An owned PTY verified that the
actual `lenz --tui` launcher replaces itself with the reader TUI, shows the source,
opens/dismisses Help, preserves source bytes and restores termios and the alternate
screen on exit. Both terminal help routes select their respective applications.
The standalone reader TUI's normal dependency tree contains no GPUI or GPU crates.

One initial **pre-change** reader test run timed out in
`a_typescript_7_project_is_served_by_typescript_go_from_its_own_tsc` while waiting
for its fake provider. The subsequent combined reader suite and full local gate
passed. Its intermittent cause was not diagnosed by this port.

## Method and retained evidence

Local raw evidence is under `target/reader-tui-port/`: full gate/build logs,
`launcher-smoke.json`, raw terminal output, process/timing samples, fixture hashes,
binary hashes and source identities. The integration starts from `3cca626` plus
the pre-existing worktree changes saved in `preexisting.patch`; those changes were
preserved. No commit or installed configuration was changed.

GUI comparisons use immutable release `perf-probe` copies built before and after
the port. `paired-gui-fresh.py` alternates their order, uses a fresh owned Xvfb
display for each launch at 1600×1000 with Mesa lavapipe, isolated configuration,
disabled providers, warmed OS/shader caches and one shared reader cache. Each case
has one warmup pair and five measured pairs. Startup ends at the content-frame
marker, before physical presentation. The small fixture is a three-line Rust
program; the large fixture is 100,000 plain-text lines.

`gui-scroll.py` adds one warmup pair and three measured pairs of scrolling the
large fixture. Each run records 63 intervals spanning about 3.024 seconds. The
software renderer's 48 ms intervals are observed values, not an assumed refresh
rate. Process CPU is cumulative from launch through the last sample. RSS is
sampled from Linux `/proc`, excludes GPU allocations and child processes, and is
not proof of leak freedom.

| GUI measurement, median | Before | After |
|---|---:|---:|
| Small source, content marker | 151.77 ms | 147.44 ms |
| Large text, content marker | 148.29 ms | 148.63 ms |
| Small source, RSS at marker | 157.68 MiB | 156.21 MiB |
| Large text, RSS at marker | 155.63 MiB | 157.54 MiB |
| Scroll interval | 48.001 ms | 47.999 ms |
| CPU through scroll sample | 1.87 s | 1.79 s |
| Scroll process high-water RSS | 173.21 MiB | 173.38 MiB |

These repeated samples show no consistent GUI timing or memory regression for
the measured workloads. The final RSS sample varied with renderer teardown;
it is not presented as settled idle memory.

The terminal comparison uses the existing standalone reader TUI as its baseline,
with its own source identity retained in `standalone-source-identity.json`.
That baseline includes the prior reader TUI work and its older shared-content
dependency; it is not a clean checkout of this monorepo's base commit. The
candidate uses the staged `lenz-code-tui` alias and local shared-content crates.
Both run the same disposable 100,000-line fixture in an owned 120×36 PTY, isolated
HOME/XDG/provider state and Darcula appearance. Timing ends at PTY bytes received
by the collector, not a terminal emulator's displayed pixels.

The collector checks palette filtering/disabled reasons, modal isolation, OSC 52
copy, goto/find, independent views, intentional external modification/reload,
idle output/CPU, sizes down to 1×1, sustained page navigation, exact file bytes and
normal terminal cleanup. Its directory-named-`*.rs` identity failure was reproduced,
fixed by hashing only files and covered with a red/green Python regression test;
the failed attempt and logs are retained. Metric semantics remain schema 1.

`tui-startup.json` contains one warmup pair and seven alternating measured pairs.
The full collector separately records 100 arrow-key samples and one 60-second
page-navigation run per executable. Both stress runs completed 740 cycles in
60.04 seconds. They emitted zero bytes and accumulated zero measured CPU ticks
during their approximately 0.50-second idle samples.

| Terminal measurement | Standalone baseline | Workspace candidate |
|---|---:|---:|
| First frame, median of seven launches | 2.87 ms | 3.20 ms |
| First source content, median of seven launches | 6.55 ms | 6.76 ms |
| Arrow-key feedback, median / p95 | 0.485 / 0.643 ms | 0.490 / 0.738 ms |
| Sustained page feedback, median / p95 | 0.500 / 0.721 ms | 0.499 / 0.739 ms |
| Largest resize feedback sample | 0.50 ms | 0.49 ms |
| Process high-water RSS | 41.34 MiB | 38.21 MiB |
| RSS after stress and 0.25-second quiet interval | 37.46 MiB | 35.89 MiB |

Both runs passed all correctness assertions and restored termios. RSS was
unchanged between the final resize and the end of stress in each executable.
This short-run plateau is not a long-session leak test. The candidate's early
frame was about 0.32 ms later; source-content sample ranges overlap. Because the
standalone baseline uses an older shared-content version, these numbers describe
the integrated product comparison and do not isolate the cost of moving crates.

Reproduction from the workspace root:

```sh
scripts/try-launcher.sh
python3 -B scripts/code-tui-bench.py \
  --binary target/launcher-trial/lenz-code-tui \
  --out target/perf/code-tui-check \
  --lines 100000 --rounds 100 --stress-seconds 60
```

The normal GUI release executable was rebuilt after preserving the probe copies;
benchmark instrumentation is not left in the staged GUI application. After the
collector fix, the complete Python collector suite passed again (63 tests,
one platform-dependent skip). Documentation links and `git diff --check` passed.

## Limits

Native macOS, hardware GPU rendering, physical terminal appearance/display
latency, installed language-server combinations, cold OS caches, the full Diff
performance matrix and long sessions remain unverified here. The terminal guide
documents clipboard, shortcut, image and pane-layout limits. No release/install
migration was attempted. Historical reader measurements remain in
[the earlier performance record](performance.md); these integration measurements
use different fixtures and should not replace or be directly compared with them.
