# 0.0.7 against 0.0.6

Paired Linux release builds measured on 2026-10-29. Baseline: tag `v0.0.6`
(`lenz-reader`, `lenz-reader-tui`). Candidate: the 0.0.7 source before its last
TUI wheel, number-column and split-boundary changes (`lenz-code`, `lenz-code-tui`),
which touch none of the measured paths; the results panel probes ran after the
wheel change. The two builds alternated twice
(old, new, old, new) on the same host with the same fixtures and collectors.
Raw samples and identities are in the ignored `perf-results/release-0.0.7/`.

## Collectors

| Workload | 0.0.6 (pair 1 / 2) | 0.0.7 (pair 1 / 2) |
|---|---:|---:|
| GUI open 2,000-line Rust file, warm cache, p95 of 20 | 144.3 / 146.2 ms | 164.8 / 164.8 ms |
| GUI same file, fresh cache, p95 of 20 | 146.3 / 144.9 ms | 165.1 / 162.6 ms |
| GUI ten kept tabs, settled / peak RSS | 181.9 / 180.4 · 205.9 / 206.5 MiB | 186.1 / 185.4 · 210.5 / 210.9 MiB |
| TUI 100k lines, first feedback | 5.0 / 4.7 ms | 4.1 / 4.2 ms |
| TUI 100k lines, first content | 17.9 / 17.0 ms | 20.8 / 20.6 ms |
| TUI input to first byte, median of 30 | 0.78 / 0.76 ms | 0.61 / 0.65 ms |
| TUI 10 s stress, median / p95 input | 0.66 / 1.27 · 0.72 / 1.37 ms | 0.66 / 0.87 · 0.71 / 1.01 ms |
| TUI settled / peak RSS | 35.9 / 39.9 · 36.7 / 40.4 MiB | 50.3 / 58.4 · 50.4 / 58.0 MiB |
| GUI edit (30 rounds) | failed: no paint feedback, 0.0.6 cannot edit | edit 2.27 · undo 4.31 · redo 10.63 · save 11.3 ms median; 252 MiB peak |

Commands: `scripts/code-launch-bench.py --trials 20` on the perf-probe build,
`scripts/code-tui-bench.py --lines 100000 --rounds 30 --stress-seconds 10` and
`scripts/code-editor-gui-bench.py --rounds 30`, each on a private Xvfb display with
Mesa software rendering and isolated configuration.

GUI startup is about 18 ms slower and the TUI 100k-line run holds about 14 MiB
more. A TUI binary built before the results-panel and terminal fixes
already measured 22.6 ms first content and 50.2 / 58.4 MiB, so both costs come
from the integrated-editor work since 0.0.6, not from the final fixes. The TUI
memory increase was recorded and accepted earlier (README, editor batch 11–18).

## Project search results panel

The ignored `results_panel_perf` probes in both adapters search 1,800 files for
9,000 matches, then step through 400 rows with Down and a full redraw.

| Build | TUI search done · Down+frame median · RSS | GUI search done · Down+frame median · RSS |
|---|---:|---:|
| 0.0.6 | 6.5 ms · 0.12 ms · 17.8 MiB | 7.7 ms · 0.16 ms · 23.7 MiB |
| Before the results-panel changes | ~21 ms · — · 24.5 MiB | — |
| 0.0.7, three runs | 17.0–19.0 ms · 0.12–0.15 ms · 24.4–25.0 MiB | 25.3–25.9 ms · 0.26–0.30 ms · 30.0–32.1 MiB |

A first profile pointed at the SHA-256 that records each matching file's identity
for the stale-result checks added with editing, but replacing it with XXH3-128
measured the same (TUI 17.0–18.3 ms, GUI 23.8–26.2 ms over three runs each), so
that hash is not the cost; the change was reverted and the increase stays
unattributed. It predates the results-panel changes. The rows rebuild during streaming is now
limited to once per 50 ms, which removed about 3 ms. Further reduction is not
attempted for this release. The GUI Down+frame cost rose 0.16 → ~0.28 ms; 0.0.7
draws the grouped tree and the side preview in that frame, and the cost was not
attributed further.

## Changes after these runs

Release QA fixes followed the measurements: the GUI completion popup with no rows, the terminal close loop, Discard
at quit, new-file permissions and focus, the terminal list highlight, sticky rows, Markdown list markers, explorer
marks and the status column. Only the status column runs per frame: it counts the characters before the caret on
its line (UTF-8 lead bytes, without decoding), which the 100,000-line and editing workloads keep short. They were
not re-measured.

A second round followed: the reopened search's selected query, word-start matches in Search Everywhere's All list,
grapheme caret steps (a 256-byte window per key), retired status messages, results in the explorer's order with
long lines shown around their match, and terminal layout changes (pane divider, results height, find box, dialog
fit). Of these only the long-line clip runs per frame, on visible result rows of at most 240 characters; they were
not re-measured. The terminal input source changed to crossterm's level-triggered tty source, which removes a stall
under bursts of more than 1,024 bytes; three paired collector runs show the same input feedback (0.78–0.95 versus
0.78–1.01 ms median) and startup, and about 0.8 MiB more settled RSS (50.8–51.8 versus 50.0–50.2 MiB).

## Lenz Diff

Lenz Diff was measured against 0.0.6 with the full suite (`scripts/perf-suite.py --profile full`), alternating 0.0.6
and 0.0.7 twice from their own checkouts on the same machine; all four runs passed. Of 391 compared metrics, five
rose by 10% or more in both pairs:

| Metric | 0.0.6 | 0.0.7 |
|---|---:|---:|
| TUI input, first output byte | 0.54–0.58 ms | 0.72–0.95 ms |
| TUI resize, first output byte | 0.38–0.43 ms | 0.73–0.80 ms |
| TUI history filter | 0.17–0.19 ms | 0.37–0.39 ms |
| TUI search cancel | 1.24 ms | 1.39–1.41 ms |
| GUI merge undo, first damage | 6.4 ms | 29.7 ms, then 7.5 ms |

The merge figure is one outlier in the first pair. To attribute the terminal ones, 0.0.7 was also built without the
`use-dev-tty` input source and the standalone collectors run twice on all three builds: input first byte
0.49 / 0.73 ms (0.0.6), 0.43 / 0.49 ms (0.0.7, old source) and 0.50 / 0.71 ms (0.0.7), resize and startup mixed the
same way, settled RSS 53.3–54.1 MiB for all. Only the history filter stayed higher with the new source (about
0.37 versus 0.19–0.22 ms; 0.0.6 also measured 0.37 once). These sub-millisecond differences are inconclusive and
were not pursued. The input source removes the same burst stall in the Diff terminal: a 3,000-byte burst into its
find box never completes in 0.0.6 (three runs) and completes in 0.27–0.30 s in 0.0.7.

The Diff defect fixes that followed add one idle thread to each terminal app (it restores the terminal on SIGTERM or
SIGHUP). The same collector, run twice on 0.0.7 before and after the fixes, alternating: first content 32.3–32.5 ms
before and 32.4 ms after (a 56.8 ms first launch of the freshly copied executable was a cold start), input first byte
0.57–0.62 / 0.56–0.59 ms, settled RSS 53.1–53.2 / 52.3–52.8 MiB. Lenz Code's terminal app gets the same thread and
was not measured separately.

## Not verified

Native macOS, physical display latency, GPU memory, and the language-server
replays for TypeScript, Java (jdtls) and Kotlin.
