# 0.0.8 against 0.0.7

Paired Linux release builds measured on 2026-10-30. Baseline: tag `v0.0.7`. Candidate: 0.0.8 before its version
commit (macOS editing, gpui v1.24.2 → v1.26.2, pulldown-cmark 0.13.6, sha2 0.11, xml-rs 1.0, resvg 0.49,
lzma-rust2 0.24 and the lockfile-compatible updates). Both were built in the same local toolchain with the
release profile (`lenz-code` with `perf-probe`) and alternated twice (old, new, old, new) with the 0.0.7 fixtures.
Raw samples and identities are in the ignored `perf-results/release-0.0.8/` (`run-collectors.sh`, `summary.txt`).

| Workload | 0.0.7 (pair 1 / 2) | 0.0.8 (pair 1 / 2) |
|---|---:|---:|
| Code GUI open 2,000-line Rust file, median / p95 of 20 | 159.6 / 160.0 · 162.9 / 162.7 ms | 158.6 / 160.8 · 163.1 / 162.8 ms |
| Code GUI ten kept tabs, settled / peak RSS | 184.9 / 186.0 · 208.9 / 210.0 MiB | 184.7 / 182.9 · 208.3 / 207.8 MiB |
| Code GUI edit / undo / redo / save, median of 30 | 2.23 / 4.31 / 10.69 / 11.25 ms | 2.23 / 4.30 / 10.69 / 11.30 ms |
| Code GUI edit run peak RSS | 253.0 / 251.4 MiB | 254.3 / 254.3 MiB |
| Code TUI 100k lines, first feedback | 3.9 / 4.0 ms | 4.7 / 4.6 ms |
| Code TUI 100k lines, first content | 22.9 / 23.6 ms | 23.3 / 23.0 ms |
| Code TUI input to first byte, median of 30 | 0.80 / 1.02 ms | 0.81 / 0.76 ms |
| Code TUI 10 s stress input, median / p95 | 0.87 / 1.05 · 1.25 / 1.50 ms | 0.83 / 0.80 · 1.19 / 1.09 ms |
| Code TUI settled / peak RSS | 51.1 / 42.5 · 59.1 / 51.4 MiB | 50.0 / 50.4 · 58.8 / 59.3 MiB |
| Diff TUI 100k lines, first paint / first content | 4.3 / 2.9 · 36.9 / 32.7 ms | 4.3 / 4.5 · 34.8 / 34.5 ms |
| Diff TUI input / resize to first byte, median | 0.55 / 0.46 · 0.28 / 0.20 ms | 0.87 / 0.73 · 0.27 / 0.46 ms |
| Diff TUI settled / peak RSS | 52.7 / 52.8 · 58.5 / 58.8 MiB | 52.8 / 52.6 · 58.2 / 57.9 MiB |
| Diff GUI first paint, median of 10 (two files) | 250.0 / 251.5 ms | 253.0 / 252.0 ms |
| Diff GUI RSS, median of 10 | 217.8 / 218.9 MiB | 217.0 / 219.0 MiB |

No workload rose by a millisecond or more in both pairs. Sub-millisecond rises in both pairs: Code TUI first
feedback (+0.7 ms) and Diff TUI input to first byte (+0.3 ms); the Code GUI edit run's peak RSS is 1–3 MiB higher
with the new gpui. They are recorded, not pursued. Commands: `scripts/code-launch-bench.py --trials 20`,
`scripts/code-tui-bench.py --lines 100000 --rounds 30 --stress-seconds 10`, `scripts/code-editor-gui-bench.py
--rounds 30`, `scripts/tui-bench.py --lines 100000 --rounds 90` and `scripts/startup-bench.py --runs 10`, each on a
private Xvfb display with Mesa software rendering and isolated configuration.

`tui-bench.py`, `markdown-bench.py` and `crash-reporting-check.py` now take the status line's `F2 details` as the
Diff terminal's first paint: since 0.0.7 its header names the comparison instead of `Lenz`, so they waited forever.
Both builds above were measured with the new marker.

## Not verified

Native macOS (editing there is checked by hand on the draft release), physical display latency and GPU memory.
