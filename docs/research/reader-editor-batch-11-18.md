# Draft ownership, editing, Save and search batch

This batch implements editor tickets 11, 12, 13, 16 and 18. Correctness checks and
release measurements are complete; the measured performance decision is pending.
Production reader typing remains gated until the later input and safety tickets.

## Delivered behavior

- [Current-source ownership](reader-current-source-ownership.md) keeps current
  text in immutable snapshots and materializes admitted analysis versions once.
- [Draft history](reader-draft-history.md) provides shared edit/Undo/Redo commands,
  exact dirty state and history retention across inactive-cache eviction.
- [Explicit Save](reader-explicit-save.md) preserves encoding and exact endings,
  accounts submitted writes independently of read cancellation, and retains
  drafts and history after rejection or uncertainty.
- [Draft-aware search](reader-draft-search.md) uses captured open-file drafts and
  disk for other files, with matching preview/activation identities.
- Draft blame attributes current text, clears stale commit actions immediately
  after edits, and presents pending or failed attribution in both adapters.

## Verification in progress

Raw commands, source identities, logs and statuses are retained under
`target/perf/editor-implementation/merge-11-18/`.

The first gate attempt stopped at the unchecked-lookup ratchet before tests.
Explicit guards removed the new unchecked lookups; the budget stayed unchanged.
The retry at `c8bf0a98` passed both workspace Clippy configurations and the
ratchets, but Reader core reported 486 passing, eight failing and 19 ignored
tests. The independent remaining phases then passed on that same source:

- Core integrations, including 98 provider/semantic checks, default Reader
  adapters and workspace doctests.
- Six isolated content-memory checks and 66 collector checks, with one skip.
- Actual Diff GUI: 261 passed; Reader GUI: 158 passed and seven ignored;
  Reader TUI with GUI parity: 122 passed.
- Separate terminal releases and the Diff release. Stripped Diff size was
  68,510,392 bytes, below the 80,000,000-byte limit.

Seven Reader-core failures were repaired by `e5ead6e` and `6dc0010`, merged at
`17e86110`. A reload now marks captured queries stale. Existing fixtures were
updated for captured preview identity, asynchronous navigation and the shared
editor command catalog. Ordinary plaintext selection uses a bounded snapshot
fallback; long selections retain admitted asynchronous work.

The remaining failure exposed a real priority gap between source conversion and
consumer submission. Its repair temporarily reserves executor capacity across
that handoff, then restores ordinary background capacity. Focused checks cover
source/result/write handoffs, cancellation, replacement, project changes, close,
Drop, panic recovery and all-three-worker background throughput. Existing owner
and submission limits remain unchanged.

The repaired integration at `bb2f55f4` passes the complete Reader core library:
499 passed, none failed, 19 ignored. Actual Reader GUI/TUI checks also pass on
that source: 158 GUI tests with seven ignored, and 122 TUI tests. The first GUI
attempt had one outline/declaration keyboard timeout. That unchanged test passed
in isolation, in 25 consecutive isolated attempts, and in the complete GUI
confirmation run. Its cause remains unresolved and is recorded in the deferred
backlog; no runtime fix or absence of intermittent failures is claimed. Passed
independent phases were retained while affected checks were rerun. This is a
completed phased correctness gate, not a fresh single end-to-end script pass.

## Measurements and limits

The comparison baseline is `bb162738`, the accepted 08–10 batch; the candidate is
`bb2f55f4`. Alternating core and startup comparisons, sustained GUI/TUI workloads,
native GUI Go To, moving-viewport scroll and candidate-only edit/Save checks have
completed. Both three-minute core stress builds have passing results. The initial
baseline attempt failed its RSS trend heuristic after a late residency step; one
repeat of that workload passed with the original executable and assertions.
Both attempts are retained. No other stress workload was repeated.

The initial GUI long-line probe repainted a fixed viewport. Its samples remain
labeled as repaint intervals. Separate scrolling checks demonstrated 30 distinct
source crops in each mode/build. The candidate-only edit/Save harness checks exact
disk output, backing release and history across file switching; its timings do
not establish frontend typing performance.

The detailed result report and performance decision are being finalized.
Acceptance of earlier costs does not accept this batch's measured costs.

Native macOS, physical display latency, GPU memory, crash recovery and real
external-provider performance remain unverified. Nonblocking observations belong
in [the deferred backlog](../editor-follow-ups.md).
