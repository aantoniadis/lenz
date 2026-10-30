# Reader editor batch 11/12/13/16/18: performance

Measured on Linux on 2026-10-24. **The user accepted the documented additional costs on 2026-10-24.**
The [batch report](code-editor-batch-11-18.md) records correctness separately. Passing its checks or a
memory ceiling does not establish preservation. This explicit decision applies only to this batch.

## Identity and method

Baseline `bb16273815e958c2153c732f186874a0c0e95184`; candidate runtime
`bb2f55f44144cd28b6c7a080635a656c43cacd5f`. Both were clean, separately built in owned private targets,
with unchanged `release` profile, line tables/overflow checks, Rust 1.99.1/LLVM 23.1.3 and no release overrides.
Production TUI excludes GPUI; ordinary GUI excludes instrumentation, with separate `perf-probe` executables.
Providers/configuration are isolated and disabled except the identified fake-provider test.

One coordinated campaign used the same shared Linux host, owned 1600×1000 Xvfb/Mesa display and 120×36 PTY.
Other owned builds/collectors stopped during timing. Core uses two alternating process pairs with existing
warmups; startup uses eight alternating pairs and three excluded GUI warmup pairs. The headless stress corpus
has 40 files. Fixtures are disposable; timing excludes compilation. Native macOS/Windows were not run.

Raw samples, exact commands/configuration, full source/collector/fixture/executable hashes and archived binaries:
`target/perf/editor-implementation/merge-11-18/performance/`. Key files are `summary.json`,
`baseline/identity.json`, `candidate/identity.json`, `machine-and-collectors.json` and `measurements/`.
Its original failed `status.json`, phased `continuation-01-status.json` and separate baseline-repeat status
remain intact. No passed stress was repeated after harmless harness corrections.

## Measurements

Medians unless specified. Core first text is model availability; GUI marker is frame construction before
presentation/syntax readiness; PTY endpoints are received content, not emulator display latency.

| Comparable endpoint | Baseline | Candidate |
|---|---:|---:|
| Core first text, 50,400 bytes; 200 samples/build | 0.1043 ms | 0.1391 ms |
| Core first text, 945,000 bytes; 200 samples/build | 0.9543 ms | 1.3592 ms |
| Syntax ready, small / large | 10.4039 / 223.4016 ms | 10.4744 / 223.3700 ms |
| Cached tab switch; 4,000 samples/build | 5.020 µs | 6.362 µs |
| Cached Back/Forward; 2,000 samples/build | 6.292 µs | 7.214 µs |
| Fake-definition target shown; 2,000 samples/build | 3.2180 ms | 2.1627 ms |
| GUI internal first-content marker; eight pairs | 139.60 ms | 139.55 ms |
| GUI ten-tab settled / peak RSS; 60.03 s/build | 176.47 / 176.47 MiB | 176.23 / 176.23 MiB |
| GUI actual-scroll median/p95, unwrapped | 48.000 / 48.093 ms | 48.000 / 48.088 ms |
| GUI actual-scroll median/p95, wrapped | 48.000 / 48.079 ms | 47.999 / 48.073 ms |
| TUI first filename / source content; eight pairs | 2.875 / 7.900 ms | 2.782 / 12.075 ms |
| TUI Go To first feedback / completed content | 0.507 / 1.551 ms | 0.509 / 1.633 ms |
| TUI startup RSS; eight-pair median | 15.85 MiB | 16.42 MiB |
| TUI arrow / sustained page feedback | 0.473 / 0.501 ms | 0.455 / 0.503 ms |
| TUI post-workflow/stress settled / peak RSS | 37.73 / 45.87 MiB | 46.42 / 49.11 MiB |
| Core stress iterations, passing runs | 1,239,150 | 992,200 |
| Core stress first / final settled RSS | 20.38 / 20.39 MiB | 22.04 / 22.07 MiB |
| Core stress process peak RSS | 34.61 MiB | 36.31 MiB |
| Core stress user / system CPU | 141.80 / 168.26 s | 230.31 / 160.23 s |

The measured costs include **+35/+405 µs first text**, **+4.175 ms TUI content**, **+0.082 ms completed Go To**,
**+0.57 MiB startup RSS**, an **upper +8.69 MiB TUI workflow band**, and **about 20% fewer mixed-stress iterations**.
Passing core stress adds 1.69 MiB settled/1.70 MiB peak RSS. Fake-provider improvement is client/fixture behavior,
not a real-provider claim. GUI cadence is similar under this software display; headroom is unmeasured.

Both PTY runs passed exact copy/source, modal, independent-view, reload, 1×1 resize, idle-output/CPU and terminal
restoration checks. They lasted 60.002/60.008 s with 740 page inputs each; collector quiet periods pace those inputs.
GUI memory has 120 half-second samples, settled from the final 60. Actual-scroll modes each recorded 626 intervals:
30.120/30.118 s baseline and 30.117/30.109 s candidate (unwrapped/wrapped). Each mode/build has 30 distinct source
crops and unchanged files. Unwrapped final/peak RSS is 193.89/193.91→193.13/193.16 MiB; wrapped 177.56/177.56→
176.80/176.82 MiB. RSS rose during these short runs; no long-session plateau is established.

## Retained failures, attribution and limits

- The original baseline core stress ran 180.170 s,1,252,260 iterations/25,045 rewrites, with settled 18.86→21.24 MiB
  and 34.28 MiB peak. Its final `[21.2383,21.2344,21.2383]` samples all exceed the earlier maximum 20.8906:
  the strict trend guard fails although the 16 MiB ceiling passes. Only this failed workload was repeated.
  The repeat passes unchanged assertions at 180.192 s/1,239,150 iterations/24,783 rewrites. Candidate passes at
  180.090 s/992,200 iterations/19,844 rewrites. Both baseline distributions remain; no threshold was relaxed.
- The retained single-logical-line “scroll” input reaches the probe's `Scroll(0)` branch. Its 30 s/mode intervals
  are **long-line repaint**, including that limit on the historical 08 collector. The separate actual-scroll
  **workload v 2** above uses 2,000 distinct lines and a changing-source-crop assertion. Crop capture is correctness,
  not latency. Both native Go To/OCR/clipboard oracles also pass with 2,000 family emoji, trailing spaces, a 600×300
  window, exact `END` copy and unchanged file bytes.
- First-text cost is in load/admission: [`decode_read`](../../crates/code-core/src/session.rs) now constructs
  exact Save provenance before `Loaded`; [`ExactRead::snapshot`](../../crates/content/src/write.rs) hashes raw
  encoded bytes and counts decoded CRLF/LF, with additional target/descriptor checks. Initial TUI filename
  feedback stays near 2.9/2.8 ms while content arrives later. These verified differences support phase attribution;
  no ablation assigns every microsecond to them. Snapshot ownership and captured-query continuations add
  navigation/search bookkeeping; stress searches every 20 iterations. Exclusive throughput attribution is unproven.
- Two short ABBA diagnostics used the existing PTY workflow, one arrow and **zero stress**, after builds resumed;
  latency values are discarded. Baseline finals span 37.00–37.78 MiB. Candidate includes 37.59 MiB and 46.10–46.41 MiB
  bands; the original 60 s upper 46.42 MiB remains. Do not average those into a preservation claim. Corrected phase
  rows put anonymous RSS near 7.3 MiB before Find in both builds, then 7.7/16.9 MiB after Find and 27.1–27.9/35.7 MiB
  after reload; file-backed differences are 0.5–0.9 MiB. [`materialize_while`](../../crates/code-core/src/text.rs)
  allocates String→Arc and line buffers when snapshot-only ownership requires a worker Document. Current admission
  invalidates old Analysis scope. Kernel anonymous RSS is not a live-owner census: neither allocator caching,
  retained backing, nor one allocation is proved to explain all 8.69 MiB. See the deferred candidate below.
- Auxiliary failures are retained: one missing String→Arc conversion blocked harness compilation; the first
  phase parser skipped empty kernel fields. Valid first-attempt finalRSS remains, corrected phase rows have
  an empty-field self-check. No production code or passed stress was changed/repeated for either correction.

## New actions and deferred work

Candidate-only shared Session commands on a 256 KiBfile:200 independent inserts median/p9515.65/23.84 µs;
one Undo/Redo 25.36/24.36 µs; one Save through confirmed baseline/backing compaction 0.807 ms. History grows through
this sequence. Save includes 100 µspolling on a disposable temporary filesystem, with one sample; this is not
frontend typing/IME or general storage-throughput evidence. EmbeddedRSS 9.35→10.25 MiB. Exact versions/disk,
intermediate and retired-baseline weak release, current storage below 64 KiB, history after file switching,
Undo/Redo dirty state and unchanged disk all pass.

A concrete deferred candidate is to reuse unchanged snapshots' full Arc-backed text/index during admitted
materialization. [`Chunk::shared`](../../crates/code-core/src/text.rs) already retains original full text/index
Arcs behind contiguous chunk ranges; a large file is several ranges, not one full-range chunk. Any reuse must
verify complete contiguous coverage of the same backing, preserving exact versions and edited/chunked fallbacks.
This was inspected, **not implemented or measured**, and does not make the observed costs unavoidable.
The [backlog](../editor-follow-ups.md) retains allocator/live-owner attribution and this candidate.

Ten reused PTY collector unit tests, wrapper syntax checks, real crop/copy assertions and the phase-parser
self-check pass. Native macOS/Windows, physical display/input-to-photon, hardware GPU memory, real providers,
production typing/IME,15-minute/long sessions and allocator-versus-live-backing attribution remain unverified.
