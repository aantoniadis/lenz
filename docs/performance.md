# Performance

Lenz aims to keep both frontends fast to open, responsive while you work, and small in memory. Changes to performance-sensitive paths are measured against a saved baseline on the same machine; a regression is fixed or recorded under [accepted costs](#accepted-costs).

This page summarizes the current results. The [benchmark guide](benchmarking.md) explains how to run the suite, and the [optimization notebook](optimization-notebook.md) records the experiments behind them.

## GUI and TUI on Linux

Release builds on a Linux desktop. The GUI runs on a private Xvfb display with software presentation, the TUI in a 120×36 pseudo-terminal. Values are medians of three warm launches.

### Opening

| Workload | GUI first paint | TUI first content | GUI RSS | TUI RSS |
|---|---:|---:|---:|---:|
| Small text pair | 95 ms | 6.7 ms | 185 MiB | 9.1 MiB |
| Generated 100k-line pair | 158 ms | 27 ms | 237 MiB | 53 MiB |
| Unrelated 100k-line pair | 232 ms | 27 ms | 247 MiB | 54 MiB |
| Two 4096×4096 PNGs | 177 ms | 64 ms | 704 MiB | 138 MiB |
| 100k-line three-way merge | 266 ms | 169 ms | 288 MiB | 96 MiB |
| Repository log, 500 commits | 114 ms | 14 ms | 186 MiB | 9.9 MiB |
| 1,000 small changed files | 102 ms | 11 ms | 186 MiB | 10 MiB |

Repeated file switching (1,100–4,400 switches) showed no memory growth in either frontend.

### Working

| Operation | GUI | TUI |
|---|---:|---:|
| Page Down in a 100k-line diff | 15.7 ms | 0.58 ms |
| Page Down in a 100k-line merge | 15.9 ms | 0.89 ms |
| Move through the repository log | 16.0 ms | 0.25 ms |
| Open About | 13.0 ms | 2.0 ms |

### Reading the numbers

- **What is timed.** GUI first paint is the first paint callback; it follows load and line diff. TUI first content is when text from the opened comparison reaches the PTY; the TUI draws a loading frame earlier, at 2–5 ms. Interaction times are first feedback after the key: the GUI's first X damage, the TUI's first PTY byte. None of these is a completed action or a photon on a physical display.
- **Work continues after first content.** Syntax highlighting completes in the background: in the TUI, output from the 100k-line pair settles after about 0.5 s and from the merge after about 0.9 s; small inputs settle within a few milliseconds.
- **GUI feedback spread.** Many GUI navigation samples cluster at 15–17 ms, while merge accepts and file loads measure 4–8 ms; the cause of the clustering is not isolated.
- **Memory.** RSS is process residency three seconds after the first frame, not GPU memory. The GUI's ~185 MiB floor is the windowing and rendering stack on this software display; the TUI's is ~9 MiB. On large inputs both hold the same documents.
- **One machine.** These are results from one machine on a software-rendered display; hardware-rendered windows will differ.

### Shared core

Both frontends call the same engine and models. Component medians without rendering:

| Component | Median |
|---|---:|
| Generated 100k-line word diff | 40 ms |
| Unrelated 100k-line word diff | 182 ms |
| 100k-line three-way merge | 72 ms |
| Find in 100k lines | 4–8 ms |
| 4K image blend | 32 ms |
| Layout of one million commits | 315 ms |

## macOS

Measured on 2026-10-11 with the current build on macOS 27.0, Apple M2 (8 logical CPUs, 16 GiB RAM), native windows, five alternating pairs per case against the previous build. Memory is the physical footprint (`proc_pid_rusage`, what Activity Monitor shows) three seconds after first paint or first frame; the `ps` resident size the earlier Mac reports used counts freed pages libmalloc keeps reusable and read about twice these figures (see the [benchmark guide](benchmarking.md#linux-and-macos)).

| Workload | GUI first paint | GUI memory | TUI first content | TUI memory |
| --- | ---: | ---: | ---: | ---: |
| Small text comparison | 231 ms | 90 MiB | 52 ms | 3.6 MiB |
| Generated 100k-line comparison | 294 ms | 143 MiB | 64 ms | 56 MiB |
| Unrelated 100k-line comparison | 348 ms | 152 MiB | 67 ms | 67 MiB |
| Two 4096×4096 PNGs | 319 ms | 459 MiB | 93 ms | 133 MiB |
| 100k-line three-way merge | 401 ms | 194 MiB | 198 ms | 92 MiB |

The 100k-line text cases dropped from 209, 223 and 411 MiB (GUI) and 122, 134 and 321 MiB (TUI) when the per-parse syntax arena started returning tree-sitter's memory whole instead of leaving it to libmalloc; timings did not move beyond noise. Since then, blocks of 1 MiB and up are mapped directly and unmapped on free (with at most 16 MiB kept for same-size reuse), instead of staying in libmalloc's large cache until memory pressure: the unrelated pair fell from 95 to 67 MiB (TUI) and 180 to 152 MiB (GUI), and four closed 4K image views from 518 to 6 MiB, for about 5.6 ms (+16%) on a 100k-line merge accept/undo/redo sequence, 7 ms (+4.6%) on opening that merge, and about 1–1.7 ms on closing a 4K view. The images row is the open comparison itself. The TUI's first content on this machine includes about 20 ms per git process (three run inside a repository); the GUI's first paint is window creation plus Metal setup. Peaks are unchanged: a 100k-line side still needs about 90 MiB of parse tree while it is being highlighted, and the three-way merge parses three sides at once.

The [optimization notebook](optimization-notebook.md#2026-10-11-macos-memory-what-ps-counted-what-libmalloc-keeps-and-the-syntax-arena) has the breakdown, the rejected allocator experiments and the raw-sample locations. Of the earlier Mac run's two findings, the native merge startup case now passes a common base, and cancelling `git mergetool` no longer stages unresolved conflict markers with `trustExitCode=true`: AppKit had been ending every quit with exit code 0 (fixed and checked on 2026-10-11, see the notebook).

From the earlier `f361ada` run (three warm launches, before the memory changes): in the TUI, Page Down in the 100k-line diff gives first feedback in 2.1 ms (merge 3.0 ms) and moving through the log in 0.27 ms; a 100k-line frame renders in 0.25 ms. Shared-core medians: generated 100k-line word diff 38.6 ms, unrelated 153 ms, 100k-line merge 70.7 ms, find in 100k lines 4.4–7.7 ms, 4K blend 26.2 ms, layout of one million commits 339 ms. The 10-second mixed stress run peaked at 308 MiB with no growth over its second half; the 15-minute stress profile was not run on macOS.

Known macOS costs in 0.0.1:

- **Large text documents still use somewhat more memory than on Linux** (TUI unrelated 100k pair 67 vs 53 MiB), mostly small-block fragmentation in libmalloc. The earlier "about 5×" was largely the `ps` resident size counting freed pages; the footprint figures above replace it.
- **Each git process costs about 20 ms** (accepted for 0.0.1), against about 1.5 ms on Linux. TUI comparisons start git before their first frame (25 ms on macOS); workflows that run several git commands are 5–17× slower than on Linux (opening the repository view: 211 vs 24 ms).
- Automated GUI input and frame timing are Linux-only (XTest/XDamage); GUI interaction on macOS relies on the manual [release checklist](release-checklist.md).

The Linux and macOS results come from different hardware and rendering setups and are not a like-for-like platform comparison.

## Release-preparation run (2026-10-10)

A full-profile suite run passed all 132 cases (391 metrics) with correctness, timeout and memory-growth checks. Its timings are **not comparable** with the tables above: background load used about a third of a CPU core throughout, and unchanged code slowed together (4K blend 25.5 → 34.5 ms, startup CPU time +25–55 % on every fixture). The tables therefore keep their earlier measurements.

Two deltas were larger than that background drag. Both were rechecked on 2026-10-11 with the machine idle, pairing the release build with the `7a29bfa` build:

- GUI find first damage on the 100k-line pair (15 → 59 ms) came from the workload, not the code. Version 3 of the diff scenario pressed `s` three times, which once returned to Shared but, with the fourth Scroll Mode, ended in Independent. With a full cycle both builds measure 17–18 ms (find-e) and 14 ms (find-scroll). Workload version 4 fixes the scenario. In Independent mode the same steps take about 46 ms on Xvfb, almost all of it in the llvmpipe software rasterizer rather than in Lenz; physical-GPU cost is unmeasured.
- TUI tab close readiness (1.1 → 5.2 ms) was background load: both builds measure 0.8–1.0 ms idle.

A second full-profile run on 2026-10-11 (workload version 4, machine idle, `f68953a`) passed with the GUI adapter tests, every registered workflow and the native user flows. Against the 2026-10-08 run, warm GUI first paint was within 0–7 ms on every fixture except the oversized prompt (107 → 119 ms); 4K image memory fell about 18 %. Flagged deltas in 4K onion/opacity/difference first damage and TUI repository refresh did not reproduce in paired runs against `7a29bfa`: both builds alternate between about 60 and 95 ms on those image steps.

## Accepted costs

A measured regression is kept only when it buys something worth the cost and the maintainer agrees. The current ones:

| Change | Accepted cost |
|---|---|
| Parallel syntax parsing | Peak memory while parsing: TUI 100k-line merge 271→642 MiB, pair 155→266 MiB; settled residency +2–22 MiB. Soak runs show no monotonic growth |
| Security and reliability hardening | About 1.4 MiB GUI / 1 MiB TUI startup residency, mostly executable pages |
| Markdown Preview | About 2.6 MiB GUI and 1.74–1.76 MiB TUI startup residency, mostly executable and file pages |
| Horizontal text scrolling | GUI startup median 189→195 ms and 240.6→242.1 MiB RSS, mostly file-backed residency |
| PR details | Terminal popup opening 1.550→1.906 ms; PR settled RSS 9.914→10.117 MiB |
| Shared themes | Resize feedback about 0.11 ms slower and focus feedback 0.1–0.25 ms slower |
| TUI About logo sent as an image | About 0.3 ms slower to open and 0.6 MiB more process RSS |

## Reproduce

```sh
# Full suite: components, terminal PTY probes, GUI startup and interactions on a private Xvfb.
python3 scripts/perf-suite.py --profile full --output target/perf/full

# TUI startup and key feedback over the same fixtures.
python3 scripts/tui-startup-bench.py --app target/release/lenz-tui \
  --fixtures target/perf/full/fixtures > target/perf/full/tui-startup.jsonl

# On macOS, add native windows to the GUI startup matrix.
python3 scripts/perf-suite.py --profile full --ui native --output target/perf/mac-full
```

See the [benchmark guide](benchmarking.md) for comparison against a baseline, regression gates and measurement boundaries, and the [release checklist](release-checklist.md) for the physical-display walkthrough.
