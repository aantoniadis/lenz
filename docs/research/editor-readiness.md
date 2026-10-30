# Integrated editor readiness evidence

Date: 2026-10-23. Execution of the [reviewed readiness plan](../superpowers/plans/2026-10-23-editor-readiness.md), governed by the [accepted design](../superpowers/specs/2026-10-23-integrated-editor-design.md).

## Status

Baseline and buffer/save experiments completed. Template and SQL probes completed with release blockers. The [foundation implementation plan](../superpowers/plans/2026-10-23-editor-foundation.md) is complete for review. No production editor behavior has been implemented or validated.

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

Executed the test-only [storage probe](../../crates/code-core/tests/editor_storage_probe.rs). Eight ordinary tests pass: shared views and independent sessions, undo after Save and actual inactive-cache eviction, stale worker rejection, UTF-8 multi-range undo/redo, edits across chunk boundaries, pre-start and in-work parser cancellation, and replacement/release of superseded pending snapshots. The owner and pending slot are experimental models; these tests do not establish production Session or adapter behavior.

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

Evidence: [v2 source/binary identity](../../target/perf/editor-readiness/storage-v2/identity.json), [summary](../../target/perf/editor-readiness/storage-v2/summary.json), [gap raw samples](../../target/perf/editor-readiness/storage-v2/storage-gap.jsonl), [chunk raw samples](../../target/perf/editor-readiness/storage-v2/storage-chunks.jsonl), [pinned probe source](../../target/perf/editor-readiness/storage-v2/sources/editor_storage_probe.rs), and [ordinary test output](../../target/perf/editor-readiness/storage-green.log). Reproduce per store with `EDITOR_STORE=gap` or `chunks`, `EDITOR_STRESS_SECONDS=180`, a fresh `EDITOR_EVIDENCE_DIR`, and `cargo test --release --locked -p code-core --test editor_storage_probe editor_storage_measurements -- --ignored --nocapture --test-threads=1`.

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

## ERB and Haml

Ran the real installed Ruby LSP 0.26.11 and VS Code HTML extension 10.0.0 server, using Ruby 3.4.10 and Node 26.10.0. Each case owns its project, HOME, XDG and Bundler state. `bundle lock --local` uses installed gems only; explicit BUNDLE_GEMFILE prevents Ruby LSP's automatic bundle setup. Rails, ruby-lsp-rails and a Haml grammar/provider are absent. No production provider was changed.

Three ordinary tests verify fixture-region mappings across ERB/Haml markup, indentation, interpolation, multiline Ruby, CRLF and non-BMP text; exact completion ranges and surrounding bytes; rejection of scaffolding, malformed positions, foreign files and stale versions; and whole-transaction refusal. These regions are supplied explicitly. They are not an ERB/Haml parser and do not establish automatic language-region recognition.

| Capability | ERB | Haml |
| --- | --- | --- |
| Project helper member completion | Native Ruby LSP returns `greet`, but its replacement range is wrong after host-language emoji; unsafe | Explicit Ruby projection returns `greet` with the exact original range |
| Mapped Ruby syntax diagnostics | Native diagnostics return null; explicit projection locates the original `]` byte | Explicit projection locates the original `]` byte |
| Signature help | One signature with UTF-16, empty with UTF-8 in the exercised changed draft; unresolved | Same encoding-dependent result; unresolved |
| HTML tags/attributes | HTML server returns the expected `div` and `class`; native ERB requests host delegation | Same HTML candidates available, but HTML edit/snippet syntax is not Haml syntax; conversion unproven |
| Rails helpers, automatic region parsing, full template formatting | Unverified; required before full release | Unverified; required before full release |

The range defect is concrete: native ERB's final UTF-8 run proposes bytes 39..42 while `gre` occupies 42..45. UTF-16 yielded a wrong ERB range and an unmappable Haml Ruby range. Do not apply these native results as if they were correct source edits. An explicit Ruby projection with UTF-8 negotiation passes exact `gre` replacement and maps syntax diagnostics to bytes 48..49 (ERB) and 51..52 (Haml). Generated scaffolding and foreign-file edits must reject the whole edit transaction. This is a viable mapping primitive, not a validated complete template implementation. Signature help and real grammar/context recognition still block the template slice and full release.

Changed drafts were sent via negotiated incremental updates and source files remained unchanged. The initial run incorrectly sent full-text changes to this incremental-only implementation; that run is retained but superseded. UTF-16, UTF-8 and strengthened failing range checks are retained separately. Stale-range rejection uses the deterministic mapping contract, including a real completion payload; it is not proof of a production asynchronous lifecycle. Markup candidates were exercised in standalone HTML projections, not a complete template host adapter.

In the final two single runs, the exercised requests plus HTML startup took about 0.59 seconds after Ruby initialization; end-of-run RSS was 77.7–78.1 MiB for Ruby and 121.6–131.0 MiB for HTML. These are point samples, not peak/settled distributions or a product performance comparison. Initialize timing and all requests/replies are timestamped in the transcripts. Generated local files are inventoried; no OS-level network/filesystem audit was performed, so isolation claims refer to the configured launch and observed artifacts.

Evidence: [pinned source/tool identities](../../target/perf/editor-readiness/templates-final/identity.json), [ERB summary](../../target/perf/editor-readiness/templates-final/erb/summary.json), [ERB range check](../../target/perf/editor-readiness/templates-final/erb/native-range-check.json), [ERB mapped diagnostics](../../target/perf/editor-readiness/templates-final/erb/mapped-diagnostics.json), [Haml summary](../../target/perf/editor-readiness/templates-final/haml/summary.json), [Haml mapped diagnostics](../../target/perf/editor-readiness/templates-final/haml/mapped-diagnostics.json), and [run log](../../target/perf/editor-readiness/templates-final-run.log). Each provider directory includes initialize capabilities, stderr, raw framed-message transcripts, completion/signature/diagnostic payloads and fixture sources. Passing the probe means the stated experiments completed; it does not mean template acceptance passed.

## PostgreSQL and DuckDB SQL

Ran 15 query fixtures per phase, covering aliases, CTEs, nested queries, quoted names, schema qualification with/without a partial identifier, functions, incomplete/malformed SQL, dialect syntax, DROP, UPDATE, COPY and a quote-escape payload. DuckDB v2.0.0 (`d54cbbe3df`) supplied its installed built-in autocomplete/parser and CLI formatter. PostgreSQL 18.6 supplied catalog metadata in a new, cached-image container with no network or host ports and tmpfs data; the owned container was removed. No PostgreSQL language provider is installed. Neither dialect has an accepted complete editor provider yet.

| Requirement | DuckDB evidence | PostgreSQL evidence |
| --- | --- | --- |
| Offline functions | `abs` suggested at byte 7; catalog gives parameter/return types | Unverified: provider absent |
| Aliases, CTEs, nested queries, quoted query-local names | Expected fixture columns absent, both offline and connected | Unverified: provider absent |
| Schema-qualified tables | `ready.f` suggests `fixture` only after explicit metadata-file access; bare `ready.` fails | Explicit catalog read returns known table columns/types; language completion unverified |
| Syntax diagnostics | `SELECT )` reports byte 7/length 1; incomplete input returns parser output without execution | Unverified: provider absent |
| Dialect syntax/formatting | EXCLUDE/GROUP BY ALL parses; formatter produces exact expected SQL | Syntax/formatting unverified; DuckDB accepting a PostgreSQL-like query does not validate PostgreSQL support |
| Function signatures from optional metadata | Built-in `abs` and fixture macro catalog queried | `double_it(x integer) -> integer` retrieved without calling it |
| Explicit schema refresh / connection failure | New read-only open sees added `fresh_col`; missing file refused without creation | New catalog request sees `fresh_col`; missing database refused |
| Dialect-switch cancellation, persistent selection, editor application | Unverified: no asynchronous SQL adapter or UI exists | Unverified: no provider/adapter or UI exists |

The offline DuckDB probe creates only in-memory engines, disables extension autoinstall/autoload and external access, and does not open a metadata file or PostgreSQL connection. Local in-memory parser use is distinct from an external metadata connection. PostgreSQL offline requests are unavailable, so zero connections there is not evidence of working language assistance. A metadata source is opened only by the explicit connected phase.

Actual DuckDB CLI statement echoes match the generated calls to `sql_auto_complete(edited_sql_as_one_literal)` and `json_serialize_sql(edited_sql_as_one_literal)`. The quoting test first failed, then passed with apostrophes doubled. Edited SQL, including the quote-escape payload, is supplied as data. Connected file opens are read-only; external access remains disabled. PostgreSQL server statement logs show only the fixed catalog queries during assistance. Edited text is not passed to that metadata adapter. Fixture DDL and data-verification reads are separate, labelled harness operations. Both probes assert zero submitted user/EXPLAIN statements, unchanged fixture rows and absent export sentinels. These observations cover these probes, not an unimplemented future provider. SQL server parsing is not used for edited-statement validation.

Freshness currently means another explicit catalog query/open, not a production cache policy. PostgreSQL metadata requests are forced read-only. Credentials are absent from these disposable local fixtures and logs. The test uses an image already present (`--pull=never`); no provider/tool installation occurred.

DuckDB offline completion processes had median 44.59 ms end-to-end (41.15–56.71 ms across the 15 distinct fixtures), including process launch and a Python resource recorder. Child maximum RSS had median 62.26 MiB (58.54–69.27 MiB). These are heterogeneous one-shot requests, not repeated latency percentiles or a persistent-worker estimate. The PostgreSQL container's single cgroup-memory sample was 68.29 MiB; it is not process RSS or a peak/leak measurement. Individual boundary records retain timings; resource records retain each child peak.

A minimal later DuckDB adapter can use only the observed operations: completion receives a query prefix and returns suggestion/start-byte/type; parsing receives SQL as data and returns syntax errors with positions; formatting returns text; catalog refresh returns tables/columns/types/function signatures. Every editor request/result must additionally carry dialect, buffer/version, request and catalog-generation identity. Validate returned ranges and reject replies after any identity changes before applying one transaction. Keep catalog access behind explicit configuration and consent; no operation accepts an executable user statement. A query-local resolver is still needed for the failed alias/CTE/nested/quoted cases. This contract is a candidate boundary, not a claim that such a resolver exists. PostgreSQL provider selection remains open, and any EXPLAIN-based feature must stay disabled.

Earlier attempts are retained: DuckDB's bare-qualified-name acceptance assertion exposed a real missing completion; the prefix case was added without removing the failure from the matrix. PostgreSQL's temporary initialization server fooled the first readiness check; the final probe waits for PID 1 to become the real server. An initial statement counter matched prefixes inside verification queries; final logs compare complete statements. A missing `/usr/bin/time` was replaced by Python's standard-library child-resource measurement.

Evidence: [source/tool identity](../../target/perf/editor-readiness/sql-final-v2/identity.json), [DuckDB outcomes](../../target/perf/editor-readiness/sql-final-v2/duckdb/summary.json), [DuckDB formatter](../../target/perf/editor-readiness/sql-final-v2/duckdb/format.json), [PostgreSQL outcomes](../../target/perf/editor-readiness/sql-final-v2/postgres/summary.json), [actual server statements](../../target/perf/editor-readiness/sql-final-v2/postgres/server-statements.log), and [final run](../../target/perf/editor-readiness/sql-final-v2-run.log). Per-fixture SQL, CLI echoes, output/errors, elapsed times and resource samples are retained beside each summary.

## Foundation handoff and final gate

The proposed [foundation plan](../superpowers/plans/2026-10-23-editor-foundation.md) defines session-owned buffers, worker-only contiguous Documents, exact save outcomes, transition guards, draft-aware navigation, blame, shared saved comparisons and both input adapters. A read-only advisor checked the document boundary. Source inspection confirmed that the existing long-line display scans the hidden tail and that GPUI's app-quit callback cannot veto shutdown; the plan uses pre-close/pre-quit commands and actual window-close hooks instead of treating cleanup as protection.

The full local `scripts/ci-local.sh` gate passed after all readiness code changes, including workspace/core, actual GUI/TUI/parity suites, strict Clippy, collector tests and separate release builds. The stripped Diff executable remains 68,501,304 bytes, matching baseline. This is evidence that test-only probes did not regress the checked build/tests; production editing and its performance remain unimplemented. [Final local gate log](../../target/perf/editor-readiness/after-ci-local.log).

Readiness experiments and the foundation plan are complete; final review corrections and verification are recorded below. Foundation can proceed independently of later provider work; the full release remains blocked on the template and SQL gaps above and on the remaining agreed feature plans. No permission to substitute highlighting or generic completion for those requirements is implied.

## Independent review corrections

The final reviewer found one probe bug and two future-plan safety gaps. PostgreSQL cleanup originally armed a name-based guard before creation, so a failed name collision could remove an unowned container. A mocked Docker command reproduced that failure (three calls instead of the permitted creation attempt). The fix arms cleanup only after successful creation and uses the returned container ID; tests cover failed acquisition and successful ID-based cleanup. The corrected real PostgreSQL metadata probe passed again ([run log](../../target/perf/editor-readiness/sql-reviewed-run.log), [pinned source identity](../../target/perf/editor-readiness/sql-reviewed/identity.json)); cleanup targets only the acquired ID.

The foundation Save contract now captures canonical path plus device/inode from the same opened file as baseline bytes. It refuses same-path replacement by a hard link to another same-content file and refreshes identity after a confirmed replacement. Existing Diff behavior remains distinct. This is a required future implementation/test contract; the readiness phase did not fix the production writer's identity model.

Ordinary macOS Dock/native Quit is a known integration prerequisite, not just an untested platform case. Pinned GPUI registers a non-vetoable termination notification and lacks the required pre-termination hook. The plan now requires a tracked dependency/platform integration and actual two-dirty-window native Quit tests before editing is enabled on that target. Forced process termination is a separate limitation. The integration is not implemented by this readiness phase.

Review scope decisions: production editing/recovery/performance remain future acceptance work; incomplete template/SQL assistance and dialect-switch cancellation remain full-release blockers; native macOS execution and physical latency remain unverified. These decisions preserve the agreed requirements, and each carries the risk of additional implementation/platform work before release. No minor findings were deferred.

After these corrections, the full local gate passed again: [reviewed CI log](../../target/perf/editor-readiness/reviewed-ci-local.log). Six ordinary provider/harness tests and eight storage tests pass; the corrected real PostgreSQL probe passes. Documentation links and `git diff --check` pass. All three Important review findings are addressed; production implementation and the named feasibility/platform prerequisites remain separate work.
