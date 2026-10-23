# Integrated editor readiness evidence

Date: 2026-10-23. Execution of the [reviewed readiness plan](../superpowers/plans/2026-10-23-editor-readiness.md), governed by the [accepted design](../superpowers/specs/2026-10-23-integrated-editor-design.md).

## Status

Baseline completed. Buffer/save and provider investigations are pending. No production editor behavior has been implemented or validated.

## Baseline

Executed on Linux from clean runtime source `300b8c8a9d3c14d520a7c1dcf2735e881e382bfa`, Rust 1.99.1. The full `scripts/ci-local.sh` gate passed, including strict Clippy, workspace/core tests, actual GUI adapter tests, TUI event/render checks, Python collectors (66 tests; one platform skip), separate terminal release builds and the executable-size gate. This establishes the existing Reader/Diff baseline, not editor acceptance.

Saved separate release GUI probe, ordinary GUI and standalone TUI executables. GUI collection used an owned private 1920×1200 Xvfb display, Mesa lavapipe, Darcula, isolated HOME/XDG state and a PATH containing only Xvfb and ps. No language-server child RSS was observed. Each cache mode has 20 measured launches after three warmups; ten kept tabs have 120 memory samples. Fixture bytes remained unchanged.

| Existing Reader workload | Result |
| --- | ---: |
| GUI warm readable marker, median / p95 | 119.58 / 123.58 ms |
| GUI fresh application cache, median / p95 | 121.36 / 123.64 ms |
| GUI ten kept tabs, settled RSS | 171.14 MiB |
| TUI first filename feedback / source content | 3.02 / 7.04 ms |
| TUI arrow-key feedback, median of 30 | 0.411 ms |
| TUI sustained page navigation | 741 cycles / 60.076 s |
| TUI settled / peak RSS | 36.85 / 40.93 MiB |

The TUI schema-2 collector passed modal, navigation, reload, independent-view and resize checks down to 1×1, preserved expected fixture bytes and restored termios. Its startup timings are one sample, not a repeated distribution. GUI timing ends at construction of a content frame before physical presentation and starts after process creation. PTY latency ends at received bytes. RSS excludes GPU allocations; short-run memory is not leak proof. These are baseline measurements, not a before/after performance claim.

Local raw evidence, immutable executable copies and exact commands remain under `target/perf/editor-readiness/` (ignored): [source identity](../../target/perf/editor-readiness/before/source.json), [configuration](../../target/perf/editor-readiness/before/configuration.json), [binary hashes](../../target/perf/editor-readiness/before/binaries.json), [fixture hashes](../../target/perf/editor-readiness/fixtures.json), [CI log](../../target/perf/editor-readiness/before/ci-local.log), [GUI raw samples](../../target/perf/editor-readiness/before/gui/launch-bench.json), [TUI raw samples](../../target/perf/editor-readiness/before/tui/samples.json), and [reproduction driver](../../target/perf/editor-readiness/capture_baseline.py). The driver retains elapsed collection time and commands per collector. These links require this local evidence directory; they are not hosted evidence.

## Verification limits

Native macOS, hardware rendering, physical display/input latency, cold OS caches and the full Diff performance matrix were not exercised. No source or collector behavior changed for this baseline.
