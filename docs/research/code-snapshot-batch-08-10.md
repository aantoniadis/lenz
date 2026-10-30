# Snapshot rendering and analysis batch: 08–10

Completed on 2026-10-24 at integration `bb16273815e958c2153c732f186874a0c0e95184`.
The Reader now renders bounded current-snapshot windows in both adapters and
admits background analysis through shared bounded ownership. Production editing
remains disabled until the later editing tickets.

## Verification

The `scripts/ci-local.sh` checks were completed in phases, with focused repairs
rather than a fresh full-suite rerun after each failure. Raw commands, source
identities, logs and statuses are retained under
`target/perf/editor-implementation/merge-08-10/`.

- Both workspace Clippy configurations and all panic-site ratchets passed.
  A new unchecked lookup was replaced with an explicit guard; budgets did not rise.
- Workspace Diff/content/engine/writer/terminal checks, provider integration
  (100 passed), snapshot/layout integration and doctests passed.
- Reader core: 440 passed, 19 ignored after repairing Unicode/tab/hint vertical
  navigation. Cold long-line navigation remains resumable and bounded.
- Diff GUI: 261 passed. Reader GUI: 153 passed initially; its remaining fixture
  was updated for bounded viewport excerpts and passed separately. The actual
  Unicode and wrapped GUI arrow checks also passed on the repaired source.
- Reader TUI with GUI parity: 112 passed initially; the remaining click fixture
  now renders both adapters while settling background work. Its focused check
  and all 12 affected parity tests passed. Exact selections and file bytes remain
  asserted. Strict affected core/GUI/TUI Clippy checks passed.
- All six isolated content-memory checks passed; collector tests ran 66 checks
  with one skip. Terminal releases built, including the repaired Reader source.
  Diff's stripped release remains 68,508,728 bytes, below the 80,000,000 limit.

The unconstrained test build was cancelled during linking after substantial I/O
pressure, before tests began. The gate resumed with four Cargo jobs and retained
compiled caches. Passed suites were retained; only affected checks were repeated.
This is a completed phased gate, not a single fresh end-to-end execution.

## Performance decision and limits

The user accepted the documented batch costs on 2026-10-24: syntax readiness
+0.72 ms for 56 KB and +9.87 ms for roughly 1 MiB; stress RSS +1.79 MiB settled
and +5.03 MiB peak; TUI startup RSS +0.59 MiB and completed Go To +0.085 ms.
Cached navigation improved. Acceptance is limited to the recorded measurements.

See the [GUI measurements](code-snapshot-gui.md),
[TUI measurements and repairs](code-snapshot-tui-performance.md) and
[combined analysis measurements](code-analysis-admission.md) for source and
executable identities, unsuccessful experiments, exact endpoints and limits.
These measurements precede the final narrow caret repair; no additional stress
run was made afterward. Future batches perform full validation and stress after
several tickets, with focused checks during implementation.

Native macOS, physical display latency, GPU memory, arbitrary-font shaping and
real external-provider performance remain unverified. Nonblocking observations
are recorded in [the deferred backlog](../editor-follow-ups.md).
