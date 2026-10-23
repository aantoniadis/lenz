# Integrated editor readiness evidence

Date: 2026-10-23. Execution of the [reviewed readiness plan](../superpowers/plans/2026-10-23-editor-readiness.md), governed by the [accepted design](../superpowers/specs/2026-10-23-integrated-editor-design.md).

## Status

Baseline and buffer/save experiments completed. Template and SQL investigations are pending. No production editor behavior has been implemented or validated.

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

## Buffer ownership and storage

Executed the test-only [storage probe](../../crates/reader-core/tests/editor_storage_probe.rs). Eight ordinary tests pass: shared views and independent sessions, undo after Save and actual inactive-cache eviction, stale worker rejection, UTF-8 multi-range undo/redo, edits across chunk boundaries, pre-start and in-work parser cancellation, and replacement/release of superseded pending snapshots. The owner and pending slot are experimental models; these tests do not establish production Session or adapter behavior.

Selected direction for foundation: indexed persistent UTF-8 chunks, initially the measured `Arc<Vec<Arc<Chunk>>>` structure with approximately 4 KiB chunks. Its cheap immutable snapshots and local text replacement fit session-owned buffers. Stable buffer identity remains separate from paths; views own selections. Keep compact inverse transactions and preserve history through Save and reloadable-cache pressure. Release a buffer only when no view/recovery ownership requires it, after the agreed dirty-close decision. This selects an implementation direction, not an unlimited-size or product-performance claim.

Workload version 2 ran each store in a separate release test process for at least 180 seconds. Three owners remained alive across repeated fixed traces: 2,000-line Rust, 100,000-line text and a 900,001-byte Unicode line. Each trace covers beginning/middle/end insertions, multiline paste, distant alternating and multi-range edits, held snapshots, viewport reads, Save materialization and undo/redo, with exact text assertions. Every eighth cycle also exercises real `Document` decoding/line access, shared `shown_line` conversion, Rust `parse_with` for the small Rust file and actual LSP JSON/framing. The probe has one synchronous analysis in flight; a separate model tests one replaceable pending snapshot.

| Workload v2 measurement | Gap candidate | Indexed chunks |
| --- | ---: | ---: |
| Complete three-fixture traces / actual duration | 1,557 / 180.071 s | 13,345 / 180.015 s |
| 100k-line snapshot median | 0.077957 ms | 0.000040 ms |
| 100k-line store viewport median | 48.199 ms | 0.070 ms |
| Small Rust: 16 distant edits, median | 0.002 ms | 0.061 ms |
| Long Unicode store viewport median | 1.601 ms | 0.275 ms |
| Process peak / last-quarter median RSS | 57.82 / 47.29 MiB | 68.45 / 56.46 MiB |

The gap candidate scans lines; chunks have newline indexes. The viewport and aggregate-throughput difference does not isolate storage or prove that an indexed gap buffer would lose. Chunks were slower for the small-file distant-edit case and had higher process RSS. The process includes the oracle, saved strings, snapshots and analysis allocations, and faster candidates complete more traces. These are not product memory comparisons or accepted performance tradeoffs. Native frontend startup, typing and memory must still preserve the captured baseline before accepting production changes.

For the 100k-line chunk case, median explicit flattening, Document construction and LSP handoff were approximately 0.75, 1.22 and 1.11 ms respectively. Better insertion storage does not eliminate these costs. Keep contiguous conversion off input/render paths, reuse one conversion of an admitted version, coalesce newer pending versions and reject obsolete completion by buffer/version/request identity. The prototype counts explicit store-content copies, root-pointer copies, snapshots and full-range materializations separately; it does not instrument every allocation/copy inside decoding, JSON or the allocator. The root vector is copied on edits and lookup scans chunks: this is not a logarithmic rope.

The existing display helper cuts a line after 10,000 characters. The corrected long-line test visits the actual long source and measures this real display conversion, but it does not prove caret/edit access beyond that displayed prefix. Foundation must resolve bounded line-window access and source-coordinate mapping before advertising long-line editing. Existing `Document::line() -> &str` and `.text() -> &Arc<str>` cannot be retained as implicit synchronous full-document conversions for chunked drafts.

An advisor's read-only review found two invalid original fixtures: a 128-byte cache rejected every pressure entry, and the Unicode viewport visited an inserted short prefix. Strengthened assertions reproduced both deficiencies. The corrected cache holds entries and verifies eviction; the long-line case asserts that the visited source exceeds half the original long line. Original `storage/` measurements and red logs remain retained, but are superseded by workload v2. Mid-parse cancellation, latest-only pending ownership and shared display conversion were added before the v2 rerun.

Evidence: [v2 source/binary identity](../../target/perf/editor-readiness/storage-v2/identity.json), [summary](../../target/perf/editor-readiness/storage-v2/summary.json), [gap raw samples](../../target/perf/editor-readiness/storage-v2/storage-gap.jsonl), [chunk raw samples](../../target/perf/editor-readiness/storage-v2/storage-chunks.jsonl), [pinned probe source](../../target/perf/editor-readiness/storage-v2/sources/editor_storage_probe.rs), and [ordinary test output](../../target/perf/editor-readiness/storage-green.log). Reproduce per store with `EDITOR_STORE=gap` or `chunks`, `EDITOR_STRESS_SECONDS=180`, a fresh `EDITOR_EVIDENCE_DIR`, and `cargo test --release --locked -p reader-core --test editor_storage_probe editor_storage_measurements -- --ignored --nocapture --test-threads=1`.

## Save boundary

Executed six readiness tests plus the existing writer and EditSession suites, including the retargeted-symlink regression. Test-only thread-local fault points exercise the real file writer against disposable files. They compile out of production. Controlled outcomes were:

- Failure before replacement leaves original bytes and no temporary file.
- In-place partial write with successful rollback restores original bytes and hard-link identity.
- Failure of both write and rollback leaves partial bytes (`aVWdef` in the six-byte fixture), visible through both hard links.
- An error reported after replacement leaves complete new bytes. The real Diff EditSession retains its draft, undo and confirmed old baseline; another Save refuses the changed disk.
- Passing unchanged mixed LF/CRLF text through the existing Diff writer normalizes lone LF when CRLF dominates.

Writer extraction contract: move only reusable primitives into `lenz-content`; keep existing Diff semantics and per-file partial-success handling. Reader needs an explicit exact-line-ending write route, preserving untouched endings, alongside the existing dominant-ending behavior. Caller-side canonical-target checks must survive extraction. Treat unclassified I/O failure as uncertain: retain draft/history and the last confirmed baseline, recheck disk, and never silently retry with a newly accepted baseline. Do not promise atomicity or crash durability for every path.

These tests establish the controlled failure boundaries, not recovery from every filesystem/hardware failure. Evidence: [writer outcomes](../../target/perf/editor-readiness/writer-green.log), [writer regression suite](../../target/perf/editor-readiness/writer-regression.log), [EditSession regression suite](../../target/perf/editor-readiness/edit-session-regression.log), and the pinned writer/EditSession sources beside the storage probe.

## Verification limits

Native macOS, hardware rendering, physical display/input latency, cold OS caches and the full Diff performance matrix were not exercised. No source or collector behavior changed for this baseline.
