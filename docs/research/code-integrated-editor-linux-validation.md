# Integrated editor: Linux release validation

The product was subsequently renamed **Lenz Code**. This report preserves the original build names,
markers and executable paths of its captured revisions; see the [current guide](../code/README.md)
for present-day build and launch commands.

Linux/shared acceptance for tickets 25–67 was exercised after ticket 58 joined the integration branch. Native macOS tickets 03 and 24 remain open, so ticket 68 and the full release remain incomplete. Wayland desktop editing stays gated. The performance comparison below records the measured costs. The user's single end-of-epic cost decision remains pending; correctness alone does not accept new costs.

Measurement execution is complete. The unchanged absolute gate failed view construction and one regex-search attempt; one GUI Undo paint-feedback timeout remains unexplained. Their original failures and subsequent comparisons are retained below.

## Sources and method

The accepted foundation baseline is `fce466ff805d490d4d0e411ed00e0c0f3c9c0c12`. The final Reader runtime is `ff217fc8f7b1dafbffd1f4abe2ac8456393ea60b`. Measurement build `d6fd35b8d2d99c88ffc515eeb188d4f9fb4d4380` adds only a real-provider test assertion/comment correction; its production runtime tree is identical. The complete canonical Diff/core/TUI campaign used `b072d0ea8ac141b260aa37eda5e5266018d663db`; the later change affects only Reader marker code and the separate Reader PTY collector, its test and guide, leaving the canonical workloads' production subtrees unchanged. Source and executable identities distinguish these cohorts. The intermediate unoptimized Reader measurements are retained separately.

Both builds used their checked-in release profiles, the same Linux environment and generated fixture bytes. The foundation cohort ran first and the candidate later; focused follow-ups use alternating executable pairs. Scheduler/cache variation remains in the distributions. The ordinary Reader GUI and standalone TUI were built separately from instrumentation. X11 runs use owned private Xvfb displays and Mesa software rendering. No competing owned builds or collectors ran during measurements.

Raw evidence is retained under ignored `perf-results/editor68/`: source, executable and fixture hashes; collector identities; raw samples and commands; failed runs; native syscall traces and server statements. These local files are not published as fabricated download links. The original 105-file core corpus and standalone Cargo GUI corpus (106 files) have separate manifests. GUI startup uses ten measured launches per cache state; fresh means a fresh Reader cache, while OS/shader caches remain warm. Fixed 60-second provider observation is not a full-index readiness measurement.

The candidate canonical campaign was continued after a tool interruption stopped its execution sessions. The first 111 completed case records were retained; the interrupted third small-GUI repetition was archived and excluded, then rerun with the remaining cases. Source, executable and fixture hashes were checked before continuation, and the public suite's collector, commands, environment and workload settings were reused. The continuation and original attempt are identified separately in the raw record. The final local correctness gate was repeated after the optimization and returned zero in a fresh uninterrupted full invocation. Later tool interruptions stopped final Reader stress phases before completion. Their incomplete logs were retained and excluded; only each incomplete phase was restarted for its full duration. Completed canonical and Reader phases were preserved. The final editing continuation used an owned background runner, with the same immutable executable and unchanged workload; its collector checkout adds documentation only.

## Final correctness and native Linux acceptance

A fresh `scripts/ci-local.sh` invocation returned zero on the final optimized runtime `ff217fc`: strict workspace Clippy in default and adapter-feature configurations, all three panic-site ratchets, default workspace/integration/doc tests, six serial memory checks, **261 Diff GUI**, **199 Reader GUI** and **201 Reader GUI/TUI parity** tests, and **75 Python collector tests (74 passes and one expected platform skip)**. Standalone terminal release builds passed. Stripped Diff size is **68,422,440 bytes**, below the unchanged 80,000,000-byte limit. Earlier failed gates and their repairs remain archived; this claim refers to the final complete invocation, not a stitched pass.

Twelve native PostgreSQL/DuckDB boundary and adapter cases passed, including an apostrophe in the explicitly selected DuckDB filename. Syscall and server-statement observations remain with the results. The PostgreSQL fixture still contained `42|unchanged`; DuckDB file hashes matched before and after. Destructive/quote-escape documents did not create export/sentinel files. Explicit metadata access stayed read-only; opening a document did not authorize running its SQL. The owned PostgreSQL server was stopped after the checks.

Owned Xvfb/XTest acceptance used the default GUI input policy (no writing opt-in): native WM_DELETE veto and repetition, modal precedence/Cancel, two consecutive preedits committing `éé` with independent Undo, exact Ctrl-S bytes, later dirty text and selective Discard, then normal exit. Native composition cancellation was not exercised: the platform can commit a dead-key prefix on cancellation. The ordinary standalone TUI passed literal q/Unicode, exact bracketed CRLF paste/Copy/Save, dirty-quit veto/Cancel/Discard and restored terminal settings. These are native Linux delivery checks, not physical input/display latency or macOS evidence.

## Review and repairs

- Diagnostic merging now tracks weak source identities rather than allocator-reusable addresses. Older versioned pushes cannot replace a newer publication waiting for acknowledgement. Closed diagnostic sources retire cached merged results. This addresses the template-problems loss observed after Undo.
- Repository lookups and semantic root resolution reject replies from an obsolete request/root epoch, including workspace changes and pending provider restarts. A panicked Git status task releases its single-flight flag.
- Recovery coalesces pending discard and newer Save intents atomically, in the correct order, with a bounded wake channel. A queued old discard can no longer erase a newer pending draft.
- Watch registration covers a newly opened file before asynchronous inventory work completes. Deterministic held-job/held-notification tests reproduced both the initial inventory and already-known-folder gaps. Actual TUI followed-rename replay passed 128 runs with concurrency eight after repair.
- Inactive buffers release marker baselines and HEAD blobs, maintaining the inactive-document cache boundary. Deterministic ownership checks and forced large-buffer GUI/TUI eviction replays pass.
- Home/End now moves the caret to line bounds in both adapters; document-bound motions and Shift selection use the shared motion path.
- The final semantic owner synchronously stops remaining owned server children and waits for those it takes; stop handoffs signal under the ownership lock, including already-stopping servers and launches racing teardown. Restarts retain their existing grace period. Real JDTLS teardown no longer requires observer cleanup of its language server; Gradle's separately owned daemon still requires fixture cleanup and is identified in the logs.

Regressions were reproduced before the production repairs. Unreproduced old flakes are not retrospectively attributed to those fixes. Test-only repairs add actual request-admission, save-worker, recovery-startup and render barriers; they do not raise timeouts. The Linux memory test uses a touched large allocation instead of requiring glibc to return small freed allocation pages. The macOS fixture is unchanged.

The benchmark collector now discovers detached descendants via `/proc` parent/birth identities, and pauses its owned process group before potentially slow timeout discovery. A deterministic delayed-discovery failure and cooperative-SIGINT cleanup were checked. Successful-workload metric boundaries are unchanged; no baseline case timed out.

## Measured marker regression and repair

The initial schema-4 candidate completed 180 seconds but settled at **83.25 MiB RSS**, versus **45.70 MiB** for the foundation, with **91.23 versus 51.00 MiB** peaks. Inspection found that unchanged saved snapshots were still fully materialized and line-diffed. The final implementation recognizes the exact buffer/version/source-metadata identity and keeps the existing baseline without that work; cancellation still wins. An equal-text fast path also avoids the line-diff workspace for Undo or an identical HEAD blob. No allocator trimming or history cap was added.

The no-extra-conversion regression failed before repair (one conversion instead of zero), then passed with controls for equal keys/different metadata and shared documents in different buffers. All 15 marker regressions passed. Three controlled before/after PTY workflow pairs with a one-second final stress interval measured pre-fix settled RSS **58.45 / 80.91 / 83.22 MiB** and peaks **90.91–92.77 MiB**; after repair, settled RSS was **47.84–47.93 MiB** and peaks **57.88–58.42 MiB**. The pre-fix residency band remains visible. These short probes justified the fix; final sustained and GUI measurements are reported separately below. Completed pre-fix results and interrupted later stress were retained, and the affected Reader campaign was repeated on the final runtime.

## Reader PTY readiness correction

The first candidate Reader PTY attempt failed its strict idle assertion on 139 bytes. Captured terminal output identified a single `vs saved` baseline publication after Reload, with background analysis still consuming CPU. It was not continuous idle repainting. A real delayed-output PTY fixture reproduced the old readiness error, then passed after repair; the complete collector suite passed **75 tests with one platform skip**.

Collector schema 4 records post-Reload settling seconds and bytes separately. One full second of consecutive output quiescence, within the unchanged five-second bound, precedes the unchanged strict half-second zero-output idle interval. Both retained baseline and candidate executables were repeated with this identical collector; the original schema-3 baseline and failed candidate attempt remain archived. This readiness change applies to standalone Reader PTY metrics, not the canonical Diff campaign. Source claims and executable hashes distinguish retained executables from the collector checkout. The settling cost stays visible rather than being relabelled as idle.

## Available providers

The available cohort passes 62 checks: 41 core provider scenarios, TypeScript 7 automatic selection, 13 actual GUI/TUI provider replays, two real ERB/Haml compiler checks and five semantic lifecycle/churn workloads. Ruby's changed-draft completion replay passed five additional serial runs. Its initial failure remains retained: Ruby LSP 0.26.11 queues didChange on its worker while parsing incoming requests on its reader; the fixture now waits for diagnostics describing the edited snapshot before completion. Exact acceptance and Undo assertions remain.

Tested tools include rust-analyzer 2026-08-31 (0.3.3033, verified source alias), gopls 0.24.0/Go 1.27.3, TypeScript 5.8.3 and 7.0.2, typescript-language-server 6.0.1, Pyright 1.1.414, Ruby LSP 0.26.11/RuboCop 1.92.1/Rails 8.1.4, JDTLS 1.61.0/Java 21, vscode language servers 4.10.0, YAML server 2.0.0, bash-language-server 5.8.1, ShellCheck 0.9.0 and shfmt 3.8.0. Real ERB and Haml formatting/compiler routes use their installed project gems. Ruby installed-gem origin was tested in a conventional private RubyGems tree; an arbitrary nonstandard GEM_HOME correctly remains a generic external origin. Java library definitions may select a decompiled declaration when JDK source archives are absent.

The final fresh provider run passed 61 cases and exposed a stale expectation in the Ruff replay: the test demanded a cancellation notice saying the tool had already started, while safe pre-launch cancellation was also possible. The updated assertion accepts exactly those two guarded outcomes while keeping exact draft/disk and successful-format/Undo checks. Strict provider-target Clippy and five consecutive real replays passed on the fixture-only follow-up. The initial failure remains archived; no production behavior or timeout changed.

Fresh Kotlin execution was unavailable: its official download endpoint was blocked, and the official GitHub release had no installable asset. The four Kotlin opt-ins are excluded, not counted as passes. Prior ticket 45 evidence and its declared limits remain valid. Optional large external corpora and provider timing probes were not silently counted. Resource sampling of tests/children is coarse: discovery occurs at most once per second plus scan cost, so short children can be missed. A reported unobserved child peak is not proof of zero resource use; setup, imports, network and observer overhead enter test wall time.

The user directed integration merge `422cb9e5337f88750f78f6c22186988da4b0b645` on 2026-10-27; it preserves the validated code tree and ticket 58. Remaining follow-ups subsequently completed as recorded below. This does not establish native release acceptance.

## Comparison with the accepted foundation

The canonical campaign completed **133 case records and 391 comparable metrics** per build, with three measured repetitions, untimed warmups, 180 seconds of mixed stress, generated 100k-line/4K fixtures, standalone TUI and owned GUI scenarios. No metric disappeared. The three optional real-Tokio endpoints were skipped in both canonical invocations; a separately identified paired supplement is reported below. [The complete metric summary](code-integrated-editor-linux-metrics.csv) gives sample counts, median, nearest-rank p95 and median percentage change for all 391 endpoints. It is a summary; local raw samples and failed attempts remain retained.

Selected canonical startup and settled-memory endpoints:

| Endpoint | Foundation median / p95 | Candidate median / p95 |
|---|---:|---:|
| Small GUI first frame, ms | 141 / 177 | 144 / 154 |
| Generated 100k GUI first frame, ms | 268 / 305 | 236 / 272 |
| Merge GUI first frame, ms | 461 / 590 | 475 / 530 |
| 4K image GUI first frame, ms | 392 / 423 | 406 / 419 |
| Generated 100k GUI RSS, MiB | 210.5 / 210.9 | 209.6 / 212.3 |
| Text browsing settled RSS, MiB | 279.7 / 284.4 | 278.9 / 282.6 |
| Image browsing settled RSS, MiB | 660.6 / 660.6 | 658.8 / 661.0 |
| Merge settled RSS, MiB | 298.9 / 301.6 | 298.4 / 299.2 |

Each row has three process samples. These selected values do not establish preservation of every interaction. The original campaign measured several GUI file-switch/merge feedback and TUI merge/independent-view medians more than 15% higher, motivating the focused alternating follow-ups below. An image-browsing phase also rose from about 235 to 275 MiB, while its overall peak/settled range stayed about 660 MiB. Retain that phase difference without describing it as a global image-memory increase.

Canonical mixed stress completed **689 versus 665 cycles** in **180.27 versus 180.10 seconds** (about 3.5% fewer candidate cycles). Median mixed-cycle time was **259.5 versus 268.0 ms**, and Undo/Redo **10.76 versus 11.19 ms**. Sampled RSS settled at **315.69 versus 317.55 MiB**. The workload checks complete results on every cycle; it is a model/worker workload, not production rendering or file-write throughput. Its modest throughput reduction remains in the cost review.

## Reader startup, input and residency

Reader GUI startup used ten measured launches after three warmups per cache state:

| Endpoint | Foundation median / p95 | Candidate median / p95 |
|---|---:|---:|
| Warm application cache, ms | 187.1 / 201.8 | 190.5 / 217.7 |
| Fresh application cache, ms | 188.3 / 202.1 | 185.2 / 213.6 |
| Ten-tab settled / peak Reader RSS, MiB | 140.37 / 142.16 | 147.42 / 149.20 |
| Maximum sampled child RSS during the ten-tab observation, MiB | 630.89 | 654.62 |

The ten-tab observation lasts 60 seconds and includes the provider. It does not wait for complete indexing. Startup ends at the first content-frame marker; it excludes presentation and completed analysis. The additional approximately 7 MiB of Reader ten-tab residency stays visible in the cost decision.

Standalone Reader PTY schema 4 used the same generated 100k-line workflow and collector on both retained executables:

| Endpoint | Foundation | Candidate |
|---|---:|---:|
| First feedback, ms (one launch) | 8.31 | 10.39 |
| First visible content, ms (one launch) | 21.34 | 18.88 |
| Post-Reload preparation, seconds / output bytes | 1.001 / 0 | 1.293 / 25 |
| Measured half-second idle output / CPU seconds | 0 / 0 | 0 / 0 |
| Ordinary input median / p95, ms (30) | 0.857 / 1.533 | 0.886 / 1.212 |
| Sustained input median / p95, ms | 0.990 / 1.577 | 1.038 / 1.903 |
| Resize feedback median / p95, ms (5) | 0.538 / 1.053 | 0.489 / 0.938 |
| Actual sustained duration, seconds | 180.060 | 180.068 |
| Sustained input cycles | 2,195 | 2,193 |
| Settled / peak RSS, MiB | 45.70 / 51.00 | 40.00 / 50.10 |

Feedback ends at collector bytes, not terminal presentation. Stress cycles include quiescence pauses, so their count is not pure engine throughput. Single-launch startup direction is checked with the paired follow-up, rather than generalized from this row.

Reader core acceptance includes cached operations (1,000 samples each): history-step median/p95 **0.360/0.484 → 0.435/0.556 ms**, tab switching **0.359/0.495 → 0.448/0.723 ms**; cached opening (100 samples) **0.408/0.861 → 0.518/1.265 ms**. Finder improved from **0.0122/0.0441 to 0.00955/0.0232 ms**. Complete ordinary search stayed **10.27/12.67 → 10.14/12.66 ms**, absent search **3.11/4.25 → 2.99/3.80 ms**, and frequent search **8.45/12.07 → 8.84/13.63 ms**. These in-process endpoints exclude frontend painting and physical latency; cached-operation increases remain measured costs.

## Ordinary GUI editing and matched shared history

The ordinary production GUI fixture edits a 100k-line CRLF/Unicode document, saves exact expected bytes, undoes, redoes, restores and saves again. Three initial rounds are excluded from timing, followed by 30 measured rounds. Each operation preserves its exact-byte oracle.

| Endpoint | Foundation median / p95, ms | Candidate median / p95, ms |
|---|---:|---:|
| Insert feedback | 7.17 / 14.79 | 8.51 / 25.73 |
| Undo feedback | 10.36 / 17.91 | 5.20 / 23.19 |
| Redo feedback | 8.35 / 17.83 | 6.21 / 21.99 |
| Save to exact observed bytes | 28.94 / 42.46 | 15.13 / 25.83 |
| Peak RSS, MiB | 165.15 | 212.52 |
| Settled RSS, MiB | 156.83 | 212.52 |

First-XDamage feedback can include background repaint and does not prove completion of the requested analysis. Save includes the 2 ms polling interval and the exact-byte check; it is not a durability measurement. The **55.69 MiB higher settled GUI editing RSS** in this run remains reported alongside the alternating repeats below. The explicit cost decision is pending. Allocation ownership was not isolated, and RSS alone does not prove a leak.

The shared foundation editing example is byte-identical across source cohorts. Each run keeps the entire growing history and its independent oracle, edits actual 34,890-byte Rust, 3,800,000-byte text and 900,001-byte Unicode fixtures, and finishes with complete Undo/Redo. Actual editing lasted **180.001 versus 180.003 seconds** and completed **12,727 versus 13,176 cycles** (+3.5%). Full-history verification then took **78.86 versus 87.08 seconds**; the candidate had more history to verify, so those totals are not a matched-workload regression comparison.

Matching the first 12,727 cycles verifies identical current byte counts before comparing timing:

| Workload | Foundation edit median / p95, ms | Candidate edit median / p95, ms |
|---|---:|---:|
| Rust | 1.229 / 2.657 | 1.186 / 2.363 |
| Large text | 1.720 / 3.313 | 1.682 / 3.015 |
| Unicode | 1.478 / 2.875 | 1.440 / 2.625 |

Five equal matched-cycle windows show the cost of the deliberately growing history. First-window medians were **0.323/0.644/0.487 → 0.349/0.667/0.504 ms** for Rust/text/Unicode; final-window medians **2.216/2.717/2.362 → 2.032/2.535/2.144 ms**. The run does not cap history or pretend this growth is a fixed working set. Matched Save medians were **3.153/15.51/4.156 → 2.825/16.44/4.117 ms**, including write confirmation; exact output and full history checks passed.

RSS immediately before full Undo was **78.92 versus 77.87 MiB**, after full Redo **89.02 versus 96.50 MiB**, with lifetime peaks **94.73 versus 111.89 MiB**. These process totals include oracle/history storage and different completed-cycle totals. Higher peak and final retained residency remain reported rather than explained away by lower pre-Undo RSS.

## Native metadata and provider resources

Explicit metadata refresh on tiny disposable catalogs, including subprocess startup and complete catalog parsing, excluded three warmups and retained 30 samples: PostgreSQL **16.54 ms median / 18.94 ms p95**, DuckDB **81.93 / 103.5 ms**. Catalog fixture assertions passed and DuckDB bytes stayed identical. No edited SQL or catalog function was executed. These are local tiny-catalog costs, not remote latency or large-database scalability evidence.

A separate prerequisite API probe records 100 PostgreSQL samples per operation: completion **0.0288/0.1088 ms**, diagnostic **0.00131/0.00388 ms**, format **0.0554/0.2186 ms** and signature **0.00145/0.00313 ms** (median/p95). Its actual 60-second loop completed **573,862 cycles**. DuckDB's isolated probe recorded native syntax **42.31/47.01 ms** (eight), native help **59.93/62.58 ms** (eight) and local completion **0.0465/0.0660 ms** (20). These isolated prerequisite probes ran on a development host and are not Session, worker or UI response times. The PostgreSQL record has its own path so a later DuckDB record cannot overwrite it.

Real provider fixture resource observations are candidate-only, include setup/imports/fixture work, and can miss short children. For scale, a plain Pyright case sampled approximately **267 MiB** of children, Ruby editing **212 MiB**, Rails editing **386 MiB**, Rust decoration assistance **597 MiB**, and JDTLS Gradle editing **2,084 MiB**. A two-adapter JDTLS replay sampled **2,526 MiB** across its children. Those are distinct fixture process sets, not additive per-tab costs or completed-index requirements. No zero value from an unobserved short child is treated as real zero. Original commands, wall times, sampled parent/child RSS and cleanup observations remain retained.

## Sustained reading

Both core reading runs completed their full 15 minutes: foundation **900.13 seconds, 982,833 iterations and 19,656 external rewrites**; final candidate **900.20 seconds, 987,042 iterations and 19,740 rewrites**. The original 105-file corpus gains one intentional externally rewritten fixture during stress. After 300 seconds of warmup, ten 60-second RSS windows are retained. The first/final three-window medians were **29.65/29.20 MiB** for the foundation and **32.14/33.22 MiB** for the candidate. Candidate tail growth was about **1.08 MiB**, within the unchanged 16 MiB guard; the roughly **4 MiB higher final residency** remains a cost rather than a baseline-preservation claim. This is a finite process-memory observation, not proof of leak absence.

## Sustained integrated editing

The final candidate completed **900.110 seconds and 30,980 iterations**. This candidate-only workload has no foundation comparison: a 100k-line CRLF/Unicode draft, marker settling and inspection, opening/closing saved comparisons, exact Undo/Redo, and offline SQL completion acceptance/Undo every twentieth iteration. Only two intended tabs remain; final draft and disk bytes match their originals. It does not time external language-server completion or native display latency. Revert correctness has separate coverage; this loop does not isolate Revert latency.

One-second RSS sampling retained 793 timeline points. Middle/final-third medians were **85.36/84.37 MiB**, maximum sampled RSS **125.80 MiB**, and the workload's tail-growth statistic **-1.00 MiB**. The unchanged 16 MiB growth guard passed. This finite fixed-working-set observation is not proof of indefinite leak freedom; RSS includes allocator retention.

The timings below retain at most the first 1,000 samples per command, rather than representing every minute of the run. Inspection and comparison include their stated readiness checks; edit/Undo/Redo measure dispatch.

| Endpoint | Median / p95, ms | Samples |
|---|---:|---:|
| compare-saved/100k | 146.671 / 179.562 | 1000 |
| edit/100k | 0.113 / 0.241 | 1000 |
| inspect-change/100k | 0.020 / 0.037 | 1000 |
| markers/100k | 134.461 / 180.766 | 1000 |
| redo/100k | 0.108 / 0.206 | 1000 |
| sql/offline-completion | 0.268 / 3.138 | 1000 |
| undo/100k | 0.103 / 0.210 | 1000 |

## Template materialization and external formatting

Template decoding/materialization excludes three warmups and retains thirty samples per fixture. Projection bytes and mapping spans are counts, not total process RSS. Exact source/generated mapping round trips and release of projection ownership pass; fixtures are generic templates without Rails context or a language server.

| Template / lines | Source / generated bytes | Mapping spans | Median / p95, ms |
|---|---:|---:|---:|
| html.erb / 100 | 2,800 / 5,600 | 201 | 0.018 / 0.023 |
| html.erb / 10,000 | 280,000 / 560,000 | 20,001 | 1.755 / 3.794 |
| haml / 100 | 1,800 / 1,800 | 100 | 0.015 / 0.016 |
| haml / 10,000 | 180,000 / 180,000 | 10,000 | 1.366 / 1.811 |

Three repeated real-provider fixtures per template retain exact formatting/Undo/disk assertions. Linux syscall tracing measures configured formatter exec-to-exit wall time, including tracing and process/Ruby startup; it excludes Session admission and UI feedback. The ERB CRLF-linter error path is reported separately from accepted corrections. Exit codes alone are not used to classify correction success.

| Formatter path | Median / p95, ms | Requests |
|---|---:|---:|
| erb/expected CRLF linter error | 2335.900 / 2504.987 | 3 |
| erb/accepted correction | 2429.269 / 2600.852 | 6 |
| haml/accepted correction | 3416.360 / 3575.174 | 3 |

## Optional real-source supplement

The canonical optional-Tokio skip remains intact. A separate supplement uses official Tokio 1.52.1/1.53.1 sources in an isolated Cargo home, immutable canonical executables, one warmup per cohort and three alternating measured pairs. No compilation occurs inside this supplement. Values below summarize the per-process medians across the three repetitions; small samples do not establish statistical confidence.

| Endpoint | Foundation median / p95 | Candidate median / p95 |
|---|---:|---:|
| tokio 185 files Default/Char (ms) | 104.261 / 112.809 | 99.340 / 104.348 |
| tokio 185 files Default/Line (ms) | 67.299 / 70.859 | 61.670 / 62.204 |
| tokio 185 files Default/Word (ms) | 68.075 / 76.145 | 69.158 / 69.205 |

## Focused alternating comparisons

The predetermined follow-ups use one untimed warmup per executable/target, then five alternating AB/BA pairs for search, independent TUI views, TUI merge, GUI file browsing, GUI merge and Reader PTY startup. Ordinary 30-round production GUI editing uses three alternating pairs. Retained baseline and candidate executables are immutable and separately identified; Reader startup repeats use one second of subsequent stress and do not replace the 180-second memory/input comparison.

One candidate GUI repetition failed to observe XDamage for its final restore Undo within the unchanged five-second deadline. That incomplete attempt is retained separately and excluded from aggregates. Only the failed/incomplete repetitions were continued with the same original collector, executable, assertions and timeouts; no original failure is relabelled as passing. Its cause was not isolated, so neither a product fix nor a collector-readiness diagnosis is claimed. Trace any recurrence; the completed repetitions do not explain the original timeout.

Selected results show the median of per-process medians, followed by the observed process range and repetition count. This is not a confidence interval. The original campaign remains reported alongside the follow-up, including outliers not independently repeated here.

| Endpoint | Foundation median (range; n) | Candidate median (range; n) |
|---|---:|---:|
| code-gui-edit/edit_ms (ms) | 7.714 (5.936–11.932; n=3) | 8.489 (7.447–9.977; n=3) |
| code-gui-edit/peak_rss_mib (MiB) | 173.141 (171.004–176.711; n=3) | 207.715 (181.203–209.750; n=3) |
| code-gui-edit/redo_ms (ms) | 3.413 (3.385–10.918; n=3) | 5.524 (4.500–7.252; n=3) |
| code-gui-edit/save_ms (ms) | 26.242 (25.704–29.066; n=3) | 12.095 (11.873–24.842; n=3) |
| code-gui-edit/settled_rss_mib (MiB) | 162.688 (156.836–167.664; n=3) | 207.715 (175.348–209.750; n=3) |
| code-gui-edit/undo_ms (ms) | 6.390 (4.772–7.092; n=3) | 6.628 (4.730–7.150; n=3) |
| reader-pty/first_content_ms (ms) | 22.410 (17.923–40.203; n=5) | 22.899 (18.594–27.945; n=5) |
| reader-pty/first_feedback_ms (ms) | 6.281 (5.175–11.016; n=5) | 6.434 (5.516–10.915; n=5) |
| search/capped (ms) | 4.220 (4.023–4.869; n=5) | 4.420 (4.008–4.749; n=5) |
| search/find-regex (ms) | 20.618 (19.368–21.835; n=5) | 20.054 (19.331–22.861; n=5) |
| search/find/absent (ms) | 7.498 (6.903–9.123; n=5) | 7.278 (6.798–8.348; n=5) |
| search/find/e (ms) | 16.930 (15.667–18.311; n=5) | 15.181 (14.135–17.617; n=5) |
| search/find/needle (ms) | 9.629 (9.244–10.238; n=5) | 8.840 (8.499–10.507; n=5) |
| tui/merge/conflict-list-open (ms) | 25.823 (22.965–32.740; n=5) | 25.730 (24.945–26.795; n=5) |
| tui/merge/resize (ms) | 0.509 (0.485–0.784; n=5) | 0.555 (0.486–2.486; n=5) |
| tui/tabs/close-ready (ms) | 7.054 (5.638–7.593; n=5) | 6.580 (5.964–7.462; n=5) |
| ui-files/backward-0/first-damage (ms) | 29.590 (27.810–32.330; n=5) | 32.240 (27.140–34.180; n=5) |
| ui-files/forward-0/first-damage (ms) | 29.200 (24.010–33.960; n=5) | 25.230 (23.840–36.080; n=5) |
| ui-files/load-file/first-damage (ms) | 45.900 (28.600–52.050; n=5) | 37.550 (24.060–58.710; n=5) |
| ui-files/revisit-file/first-damage (ms) | 49.760 (39.980–67.560; n=5) | 48.590 (34.520–70.640; n=5) |
| ui-files/settled/rss (MiB) | 275.700 (264.900–278.400; n=5) | 278.400 (272.700–284.000; n=5) |
| ui-merge/settled/rss (MiB) | 298.500 (297.600–301.800; n=5) | 297.200 (295.400–298.100; n=5) |
| ui-merge/undo-keep-base/first-damage (ms) | 39.470 (28.420–42.280; n=5) | 38.530 (33.170–53.830; n=5) |

Repeated ordinary GUI editing settled RSS medians were **162.69 versus 207.71 MiB**, a **+45.03 MiB** candidate difference. Preserve both the repeated ranges and the original 55.69 MiB difference; allocation ownership has not been isolated, and these finite runs do not establish leak freedom or an allocation cause. No measured cost has been silently accepted.

## Evidence verification and acceptance status

Final checking revalidated retained executable/fixture manifests, matched final Reader hashes to their original measurements, and recorded final source/Reader fingerprints (**30 records**). Original corpus files retain their manifest hashes. Failed/intermediate/interrupted attempts remain archived.

### Absolute-budget failures retained

The unchanged `scripts/perf-check.sh` gate **failed** at 100k-line view construction: candidate **4.61 ms** exceeds the **2.4 ms** limit. The retained foundation executable also failed this exact enabled check at **4.83 ms**. The canonical same-machine medians were **4.85→4.56 ms**; the reference ceiling is not met even by the baseline. No limit was raised and no repeat is relabelled as a passing full gate.

The original script stopped at that failure. Its previously unrun merge, syntax, image and history checks passed when executed individually with their original limits. Regex search failed at **25.213 ms** against **25 ms**. The original candidate-only search failure is preserved rather than overwritten by later repetitions.

The matching baseline search replay **passed**, with regex median **21.045 ms**. Original canonical regex medians were **20.739→21.244 ms**; the five alternating search pairs above show the repeated distributions. These observations do not turn the failed absolute gate into a pass. The fresh full correctness CI invocation remains a separate passing result.

The end-of-epic cost decision remains **pending**. It covers the complete canonical summary, cached Reader increases, observed GUI/reading/history residency, first-feedback distributions, mixed-stress throughput, and candidate-only template/provider/SQL costs and their stated measurement gaps. Correctness and passing ceilings do not accept a deliberate tradeoff. Native macOS/Wayland and physical latency remain unverified.


## Repeating the measurements

Install the normal build/display prerequisites and the exact provider versions required by the selected
opt-ins; [provider setup and limits](../code/providers.md) remain authoritative. Build each source in
its own worktree/target with unchanged Cargo profiles. Run the full gate first, retain executable
hashes, then collect sequentially without competing builds. Use new output directories and disposable
fixtures. A retained executable's `--source` claim must name its actual build source.

```sh
scripts/ci-local.sh
python3 scripts/perf-suite.py --profile full --runs 3 --stress-seconds 180 \
  --output perf-results/editor-candidate --compare perf-results/editor-baseline
cargo build --locked --release -p code-tui
python3 scripts/code-tui-bench.py --binary target/release/lenz-reader-tui \
  --out perf-results/editor-pty --lines 100000 --rounds 30 --stress-seconds 180
cargo build --locked --release -p code-gui
python3 scripts/code-editor-gui-bench.py --binary target/release/lenz-reader \
  --source BUILD_SOURCE_SHA --out perf-results/editor-gui --rounds 30
READER_CORPUS=DISPOSABLE_CORPUS READER_STRESS_MINUTES=15 \
  cargo test --locked --release -p code-core --lib session::tests::stress_perf \
  -- --exact --ignored --nocapture
EDITOR_STRESS_SECONDS=900 cargo test --locked --release -p code-core --lib \
  session::tests::editor_perf::integrated_editing_costs_and_retention \
  -- --exact --ignored --nocapture
cargo test --locked --release -p code-core --lib \
  session::tests::editor_perf::template_materialization_costs -- --exact --ignored --nocapture
```

The catalog-cost opt-in additionally requires explicitly configured disposable PostgreSQL/DuckDB
fixtures. Native close/composition checks require owned windows. Fresh official-provider acquisition
and native platforms are capabilities to verify, not grounds to count a skip as a pass. The optional
Tokio supplement used already acquired official versioned source trees in an isolated Cargo home;
the canonical suite's recorded skip remains unchanged. Raw attempts and source/fixture/executable
manifests are retained locally under `perf-results/editor68/`.

## Remaining acceptance

Native macOS quit veto, dirty closure, composition and ordinary production input remain unverified; tickets 03, 24 and 68 stay open. No platform gate is removed. Physical display/keyboard latency, hardware GPU memory, Wayland and fresh Kotlin are unverified. Provider inference and template contexts retain the capability limits in the provider guide. Rust check-on-Save and a broad Python Fix All remain separate policy/features, as previously decided.
