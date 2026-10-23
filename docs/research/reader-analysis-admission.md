# Reader analysis admission

The implementation is measured against the accepted snapshot foundation
`1d874aee50c620c10201d0695de0e97fe140a83a`. The combined candidate is
`dbdc325e75dbf9066336299175de5a850a59a41c`, including the completed GUI/TUI
snapshot adapters from `b17741b`. These comparisons cover the combined core;
they do not isolate the cost of ticket 10 from the adapter prerequisites.

Raw evidence, executable archives, source hashes, configuration and the exact
workload script are under `target/perf/editor-implementation/10`. Cargo uses
only the implementation worktree's private target. Release configuration is
unchanged, including line tables and overflow checks. The archived fake server
is selected from its expected `release/examples` path; `CARGO=/usr/bin/true`
prevents fixture setup from compiling during timing.

## Production ownership and admission

The private Session-owned Analysis service uses the existing three Jobs
workers. There are at most 64 combined source/result owners, one deduplicated
waiting identity per owner, six submitted Analysis tasks in total and at most three
Background submissions. This reserves queue capacity for Visible work without
creating another pool. A queued background materializer can be promoted when
visible demand arrives.

Each source owner keeps one running version group and one replaceable pending
snapshot. A matching loaded Document registers synchronously. A differing
snapshot materializes once off the input thread, reusing its line index and
checking cancellation between source chunks. Independent syntax, Find per
TabId, Markdown, current-buffer index, semantic readiness/decode/documentation,
selection expansion and root-kind consumers share that Document. Old pending
payloads release immediately. Completed inactive owners are pruned rather than
becoming a second frontend cache.

Semantic target resolution, retrieved-source decoding and both hierarchy
result branches use result owners keyed by provider session and semantic lane.
They reserve capacity before cloning the provider's sent map. Each slot keeps
one submitted operation and one replaceable pending operation. Its stage,
request, optional source identity, project, result set and hierarchy node are
fenced again at publication. Source-free workspace symbols need no synthetic
buffer, and independent servers retain independent answers. Successful queued
completions stay valid after their CPU lease finishes, until acknowledged.

RAII leases release CPU credits on cancellation, panic and delivery failure.
Current abandoned result operations visibly fail and finish their spinner;
capacity pressure reports a retryable error. Session teardown cancels both
split predicates, derived renders, analysis and layout/projection schedulers
before closing the result channel and joining workers.

The bound on retained sent-map **generations** is separate from their size:
at most six submitted task payloads and 64 pending owner payloads can capture
maps. Individual generations share Documents with provider caches rather than
copying their text. Live provider maps, current source ownership and bounded
published Target payloads are additional owners. This is not a constant-byte
bound for an arbitrary number of open documents, or an end-to-end two-version
text claim.

## Provider writes and request ordering

Each server has desired latest text, exact text admitted to its FIFO and exact
last-written text used to resolve replies. It permits one unacknowledged text
frame per path, at most three frames overall, and aggregate outstanding UTF-8
payload at most `3 * LARGE_BYTES` (15 MiB). Closed/reopened/renamed paths retain
credit until the session/token acknowledgement arrives.

The writer acknowledges only after complete write and flush, before advancing
to its next message. Exact admitted text may be followed immediately by its
request in the same FIFO; no extra acknowledgement turn is required. All
semantic lanes, hierarchy/items, workspace symbols, virtual retrieval and Ruby
refresh respect deferred text barriers. Pressure defers demand rather than
reporting provider failure. Failure reaps the owned process group and preserves
bounded stderr/exit evidence even when a helper keeps output open.

The 15 MiB figure bounds outstanding historical transport payload. It excludes
live desired/last-written Documents, result-map generations, one writer's
encoding scratch, bounded non-text protocol messages and provider memory.
Legacy-decoding expansion is measured as UTF-8 bytes rather than inferred from
encoded file size. Existing loaded-Document large-file eligibility is preserved.

## Correctness evidence

The actual previous target producer, replayed in an otherwise current source
snapshot, retained 129 Document-map closures for 129 replaced replies. The
candidate passes the same gated Session trace with bounded submissions. The
source selection trace likewise demonstrated 129 obsolete queued owners before
migration. Raw red/green logs remain in `10/results`.

Focused checks cover shared materialization, split Find independence, visible
promotion and saturated background work, pending weak release, admission-enabled
clean-cache eviction, cancellation after work starts, panic/channel failure,
source metadata and version changes, stale resolved navigation, zero-source
independent result owners, saturation before payload construction and queued
completion validity after CPU release. Provider regressions cover restart,
request lanes, hierarchy, virtual sources, Ruby refresh, blocked writes,
close/reopen/rename and decoded expansion.

After merging the GUI/TUI prerequisites: 14 selected Session/admission checks,
6 native-column/reveal/projection Session checks and 100 provider integration
checks passed; five external/timing checks were ignored. Strict reader-core
Clippy over all targets passed. These are focused migration checks. The root
agent owns the single combined workspace/GUI/TUI/local gate.

## Measurements

Existing benchmark assertions stayed active. Two process runs per variant used
AB/BA order; each opening metric has 200 trials, cached tab/history have
4,000/2,000 trials, and fake definitions have 2,000. Values below are pooled
statistical medians, with the unchanged raw per-run samples retained.

| Core action | Baseline | Candidate | Difference |
| --- | ---: | ---: | ---: |
| 56 KB text shown | 0.0995 ms | 0.1105 ms | +0.0110 ms |
| 56 KB syntax ready | 10.479 ms | 11.201 ms | +0.722 ms |
| Roughly 1 MiB text shown | 0.9172 ms | 0.9521 ms | +0.0349 ms |
| Roughly 1 MiB syntax ready | 223.857 ms | 233.731 ms | +9.874 ms |
| Cached tab switch | 0.00519 ms | 0.00485 ms | −0.00034 ms |
| Cached Back/Forward | 0.00618 ms | 0.00580 ms | −0.00038 ms |
| Fake-provider definition shown | 3.186 ms | 3.220 ms | +0.033 ms |

The syntax cost repeats in both runs: baseline/candidate medians are
10.542/11.106 and 10.463/11.277 ms for the smaller file, and
224.291/233.557 and 223.223/233.966 ms for the larger one. Definition p95
improves from 4.236 to 3.286 ms. During the 20,000-message provider progress
flood, dispatch medians are 0.0006 ms baseline and 0.0008–0.0009 ms candidate;
p99 ranges are 0.0054–0.0079 and 0.0044–0.0068 ms respectively. One candidate
maximum is 0.6945 ms; retain that outlier alongside the percentile results.

Opening peak RSS falls from 112.52–112.82 to 109.43–110.37 MiB. Fake-provider
workload peaks increase from 17.15–17.32 to 18.31–18.43 MiB. These supplemental
Linux `wait4` peak figures are the launched process/descendant high-water mark,
not aggregate process-tree memory or GPU memory. The versioned resource wrapper
has a focused Python status/wall/resource check; test-defined timing samples
are unchanged. An initial missing GNU-time attempt exited before launching any
benchmark and supplies no measurement.

Both fixed-working-set stress runs pass over the same 40-file corpus: baseline
180.125 s, candidate 180.197 s. Their ten settled RSS-window medians are 19.078
and 20.869 MiB. First/final-three-window medians are 20.031/20.121 and
20.836/20.996 MiB respectively; process peaks are 33.219 and 38.246 MiB.
Baseline completes 1,155,619 iterations/23,112 external rewrites; candidate
completes 1,250,150/25,003. This single pair is a stability observation, not leak
proof or a repeated throughput estimate. Passing its growth ceiling does not
establish preservation of the baseline.

The 1,040,009-byte UTF-16-origin conversion fixture uses 300 interleaved samples:
original API median 333.478 µs, cancellable indexed conversion 94.699 µs, and
cancellation after the first chunk 0.371 µs (p95 0.411 µs). Literal line/metadata
assertions remain active. Gated real parser teardown and pending-result
replacement also pass in the retained release executable; their whole-test wall
times, 0.017 and 0.001 s, are not individual cancellation-latency estimates.

**The user accepted the documented batch costs on 2026-10-24** and requested
continued implementation, with full validation and stress measurements once per
batch of several tickets. Acceptance covers these measured costs, including the
TUI report's startup-memory and completed-GoTo changes; it does not approve
future regressions. No additional stress or optimization loop is scheduled for
this batch. Core opening is not native window first
paint, and fake-server latency is not real-provider performance.

## Remaining boundaries

Editing/history and project-search overlay are later tickets. Existing target
navigation still resolves provider positions against exact last-written
Documents and reads other target files from disk; target-draft navigation policy
belongs to ticket 17. Foundation ticket 11 must remove remaining compatibility
Tab.doc reads/getters and reconcile encoded versus normalized large-file policy
explicitly. Native macOS, external real-provider runs and physical input-to-photon latency remain unverified.
