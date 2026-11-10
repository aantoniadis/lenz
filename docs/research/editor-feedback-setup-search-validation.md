# Editor feedback, server setup and Find validation

This Linux validation starts from merged PR #51, `6f7a186`, on 2026-11-10.
Raw logs, attempts, source manifests, executable hashes, fixtures and collectors
are retained in `perf-results/editor-diagnostics-install-search/`.

## Changes and regression evidence

Ordinary dirty-tab navigation already passes on the base tree. The added core
replay covers Java, Rust and Ruby, settled and pending analysis, three round
trips, exact draft identity, on-disk bytes and Undo. A separate split-pane replay
reproduces a remaining failure on the base: another pane retains delivered
analysis, the scheduler deduplicates it, and the restored view has cleared its
colors. Restoration now shares the peer's current syntax and semantic payloads,
guarded by exact source/version identity. It does not copy document text, add a
cache or replace either pane's independent view positions.

The paired GUI/TUI replay edits malformed Rust using actual input, leaves and
returns to the file in one split, and checks both panes' lexical colors and error
underlines. It runs in light/dark appearances and narrow layouts, including
12 × 4 terminal cells. The original file bytes remain unchanged.

Missing-server status now opens Install directly; F2 initially selects Install
for a missing server. The shared preview offers runtime selection before a
Node-based server exists, progress, cancellation, details and retry. Both adapters
replay F2 → Install → choose Node → install through actual input and a real
owned fake-manager subprocess. Existing modal-first, tiny-layout and process-tree
cancellation tests remain active. Ruby LSP 0.26.11 and gopls v0.24.0 join the
curated npm recipes, using existing compatible toolchains and private prefixes.
Custom launches and unrelated/unknown settings remain guarded and preserved.

Native Find validation also found a separate integration bug: a completed search
with no hits updated its state but returned no repaint request when revealing
the nonexistent match. The event-driven TUI could display “searching” indefinitely.
The new regression fails on that path and passes after every accepted match
completion requests repaint. A normal production TUI replay passes without an
extra redraw key. Earlier collector mistakes and a diagnostic replay with such
a key are retained and excluded from timing claims.

## Real package-manager checks

- Real Node/npm installs and validates `bash-language-server@5.8.1`, then saves
  its managed launch successfully.
- Ruby's real manager caught two command mistakes during development: `--norc`
  belongs after `install`, and Ruby uses `-c` for syntax checking. A fresh private
  installation from official, SHA-verified cached gem packages now builds native
  dependencies, installs six gems, validates syntax/version/managed paths and
  persists settings successfully. This offline run does **not** validate registry
  discovery: the environment blocks `index.rubygems.org` with HTTP 403.
- A real Go 1.27.3 invocation downloads gopls dependencies but cannot complete:
  a module download redirected to `storage.googleapis.com` receives HTTP 403.
  Live Go installation remains unverified. Real subprocess fixtures separately
  verify its command/environment, exact version refusal, managed executable and
  provider-only settings persistence.

No registry failure is counted as a passing install. Native macOS/Windows and
additional language-server package versions remain unverified.

## Normal-release Find comparison

The baseline content executable was built from `6f7a186` in its own checkout and
target directory, adding only test-only measurement code. Candidate and baseline
use the normal repository release profile. Three alternating process pairs each
run seven alternating samples per case, with exact hit/range/cap comparisons and
at least 20 seconds of sustained scanning per process. Timings exclude query
construction, document materialization, frontend work and physical presentation.

An exact copy of the old implementation inside the probe initially suggested
regressions even on unchanged Unicode/regex code. The separately built baseline
shows that private-module/reference placement changes compiler output. The table
therefore compares the public root implementation in each independent executable,
not the private reference. All experiments remain in the raw logs.

Medians below pool 21 samples per build/case. The ordinary corpus has 100,000
ASCII source lines; dense and Unicode cases retain their own documented inputs.

| Completed matcher scan and bounded collection | Main, ms | Candidate, ms |
| --- | ---: | ---: |
| Absent ASCII literal | 7.174 | 1.974 |
| Sparse case-sensitive ASCII literal | 16.189 | 1.475 |
| Common ASCII literal, 10,000-hit cap | 0.742 | 0.460 |
| Dense whole-word line, 10,000-hit cap | 0.134 | 0.135 |
| Repeated-prefix absent literal, 4 MiB line | 11.961 | 0.151 |
| Unicode case-sensitive | 0.373 | 0.371 |
| Unicode case folding | 1.376 | 1.349 |
| Regex | 9.663 | 9.666 |

The scanner is prepared once per bounded collection and borrows the literal.
Case-sensitive literals use `memmem`; ASCII case-insensitive scans use an anchor
byte with `memchr`/`memchr2`. Dense lines use the original compact iterator;
Unicode case folding and regex retain the original matching path. Cancellation,
non-overlap, whole-word boundaries, document ordering and the cap remain checked.
Generated deterministic corpora compare exact results with the regex oracle.
The optimization applies to consumers of `collect_matches`; project search and
fuzzy navigation are not claimed to have become faster.

All six sustained scan processes run 20.001–20.014 seconds. They alternate the
root and old reference scans with exact results: baseline completes 1,428–1,460
cycles, candidate 2,234–2,283. Sampled RSS stays within 39.26–39.84 MiB, with at
most 0.063 MiB first-to-last growth per baseline process and none in candidate
samples. This is bounded workload evidence, not proof of indefinite leak freedom.

## Environment and process ownership

Validation uses cached Rust 1.99.1, rootless native libraries and matching Clang
headers. No Cargo profile, jobs, incremental or compiler-flag override is used.
Standalone TUI builds exclude GUI dependencies and precede GUI feature builds.
Inactive incremental output is removed only while no Cargo process owns it.

The initial settled count is nine processes and one pre-existing PID-1 Git
zombie. An unsupervised Go SDK version inspection spawns its detached telemetry
helper, leaving a second PID-1 zombie at 00:26:03 UTC. Two subsequent supervised
version checks retain the same two zombies. Individual validation/build/benchmark
commands and the Linux gate run under `scripts/reap-children.py`; snapshots and
orphan counts are retained. No fork/thread creation failure occurs.

After the completed performance campaign, during an environment starting/ready
transition, the eleventh gate's outer supervisor receives SIGKILL (exit status 9). Its
detached test session continues without its subreaper, accumulating orphaned
descendants at PID 1. That run is stopped and is **not** a passing gate. The new
settled baseline is 767 zombies. Eight fresh supervisor lifecycle tests and a
20-commit synchronous Git batch pass with exactly 767 before and after. There is
no cgroup PID ceiling or fork/thread failure. The final gate isolates the
supervisor's session with `setsid --wait` to survive the observed outer-group
cleanup. These counts are retained rather than describing the interrupted run
as supervised success. Previously completed timings precede this transition.

Earlier failed gates are retained: test/probe Clippy errors, a GUI status tuple
compile error, an async GUI settings-save/restart test race, a runtime-repair
replay assuming F2 always selects row zero, and a timed existing semantic-helper
cleanup case during a competing feature-graph build. The restart test now waits
for actual availability, fixed-row menu replays use Home, and the helper cleanup
case passes alone. A background-lane test now checks cancellation against the
foreground definition request ID: following it may legitimately cancel highlights
of the old file. Successful navigation is still required. Tests of the new
initial Install selection remain active. A Diff TUI Markdown-close test also
waits for the closed view's cancelled worker to unwind before requiring its last
weak owner to be gone; settling only the remaining view did not establish that.
The existing bounded release assertion remains mandatory.

## Complete local gate

`scripts/ci-local.sh` passes on the final production and test tree. Both strict
Clippy feature graphs and all three panic-site ratchets pass. Workspace and
serial-memory invocations report 2,958 passed / 145 ignored; dedicated adapter
invocations report 925 passed / 30 ignored. These are invocation totals with
overlapping coverage, not distinct test counts. Python runs 85 tests with one
macOS-only skip, including the supervisor lifecycle tests. The Rust detached Git
maintenance fixture regression passes again.

The gate builds standalone terminal applications before GUI features and checks
the normal release GUI. Stripped `lenz` is 68,909,520 bytes against its
80,000,000-byte limit. The final isolated supervisor reports success in 228.554 seconds, collects
821 orphaned descendants and requires no forced cleanup. Post-gate health retains
the same 767 PID-1 zombies present after the investigated interruption, with no
further accumulation. `ci-local-13.log` and `ci-final-summary.json` contain the
complete final results. The earlier fully passing ninth gate and all failed or
interrupted attempts remain retained.

## Production frontend comparison

Both executables use normal Cargo profiles without test/performance features.
Retained baseline GUI/TUI executables have production sources identical to
`6f7a186`; the only intervening changes from their runtime commit are tests.
New standalone TUI and GUI binaries are built separately in this checkout.
Hashes, source claims, toolchain/environment and dependency graphs are retained.
Measurements run sequentially without compilers or test suites, using private
application state, an owned Xvfb/Mesa lavapipe display and actual dark appearance.

The original GUI feedback collector exposed a real measurement race on both
builds: after resetting its NonEmpty XDamage region it could discard a late
notification, leaving the region nonempty and suppressing the next one. A
deterministic private-Xvfb control records one discarded notification, zero for
the next paint without reset, and one after proper reset. Schema 2 now drains
preceding events before rearming and never discards notifications afterward.
Failed and earlier GUI attempts remain retained; they are excluded from the
final table. The final input collector uses exactly that logic with the same
owned fixture outside Git for both builds (its separate schema 3). Three
alternating pairs / 20 measured rounds each pass every byte oracle.

GUI startup alternates seven measured launches per build after one excluded
warmup. Its first client-image endpoint receives the first nonuniform 512 × 128
client crop, before complete source/analysis readiness. GUI input ends at first
XDamage, including possible background paint; Save has a separate exact-byte
completion oracle with 2 ms polling. TUI Find schema 3 clears prior Find state
before each sample, then measures query paste through a fresh `1/1` counter,
Enter and the complete target source line. Every selection is verified with
exact OSC52 clipboard bytes. Three alternating process pairs run at least
20 seconds each on the same 100,000-line corpus, excluding an explicit warmup
query in each process. Neither frontend endpoint measures physical presentation.

| Endpoint, median | Main | Candidate | Samples per build |
| --- | ---: | ---: | --- |
| GUI mapped window, ms | 135.905 | 141.579 | 7 processes |
| GUI first received client image, ms | 650.508 | 661.158 | 7 processes |
| GUI startup settled RSS, MiB | 179.695 | 176.000 | 7 processes |
| GUI edit first damage, ms | 12.213 | 9.134 | 60 actions |
| GUI Undo first damage, ms | 4.472 | 3.363 | 60 actions |
| GUI Redo first damage, ms | 3.395 | 3.380 | 60 actions |
| GUI Save exact bytes, ms | 13.705 | 12.058 | 60 actions |
| GUI editing settled RSS, MiB | 165.359 | 161.812 | 3 processes |
| GUI editing peak RSS, MiB | 182.445 | 176.688 | 3 processes |
| TUI first complete source line, ms | 17.964 | 16.923 | 3 processes |
| TUI completed Find and reveal, ms | 9.875 | 3.450 | 165 / 168 queries |

Completed native TUI Find improves about **2.86×**, including core scheduling and
observable reveal. The setup, clipboard check and deliberate settling intervals
are outside each latency measurement; cycle counts are not a throughput claim.
All six processes run 20.156–20.293 seconds, verify unchanged source bytes,
survive narrow/1 × 1 resizes and restore observable terminal state. A separate
candidate absent/present replay runs 20.399 seconds / 36 cycles: zero-hit counter
completion has median 3.898 ms. The base's zero-hit paint can remain pending
until another input, so no bounded baseline latency or speedup ratio is invented.

GUI startup samples vary in both directions, with first-image ranges
637.82–685.51 → 625.58–694.68 ms. Input process medians also vary; no GUI repaint
or startup speedup is claimed. Exact Save/Undo/Redo pass on the 100,000-line
Unicode/CRLF fixture. Startup idle CPU medians are zero sampled ticks per second.

TUI RSS reproduces both residency modes in both builds: settled ranges
29.941–39.262 → 29.809–39.629 MiB. After initial settling, first-to-last quarter
median growth is at most 0.102 MiB; no accumulating live search state is observed.
Three processes per build do not establish identical mode frequencies or
indefinite leak freedom. RSS is not GPU allocation accounting. No production
allocator tuning, profile exception, additional full-document cache or relaxed
correctness assertion is accepted.

## Evidence and limits

All raw evidence is local under ignored
`perf-results/editor-diagnostics-install-search/`:

- `tab-repro-2.log`, `core-focused-3.log`, `paired-new-flows-5.log`,
  `native-find-regression-{3,4}.log`: failing reproductions and passing core/input
  regressions, including the no-match repaint signal.
- `live-shell.log`, `live-ruby-offline-final.log`, `live-go.log`, package metadata,
  gem SHA-256 records and blocked-host headers: actual install coverage and limits.
- `ci-local-13.log`, `ci-final-summary.json`, earlier gate logs and
  `health.jsonl`: validation and process ownership.
- `baseline-source.json`, `candidate-source.json`, `binary-identities.json`,
  normal-profile build logs and `standalone-tui-dependencies.txt`: identities.
- `content-comparison.json`, `content-summary.json`, `gui-startup/`,
  `gui-final-*`, `tui-find-{0..2}-*`, `tui-find-complete/`,
  `frontend-summary.json`: final samples, fixtures, screenshots and definitions.
- `xdamage-repro.*`, earlier numbered probes/GUI attempts and `method-notes.md`:
  rejected experiments, collector corrections and limitations. Not every
  transient early scanner source snapshot was retained; final sources and
  independently built executables are retained.

Native macOS/Windows/Wayland, hardware GPU/frame pacing, physical terminal/display
latency, prolonged external-provider sessions and indefinitely long workloads
remain unverified. Live Ruby registry discovery and a complete real Go install
are blocked here. Fuzzy navigation and project search performance are outside
the measured optimization. Merging and releasing remain separate actions.
