# Prepare, validate, package and publish 0.0.14
Status: claimed
Type: task

Bump workspace/lock metadata to0.0.14, write release notes covering PR49–52,
run required automated gates and full release workloads, then tag the validated
commit. Monitor all three native package jobs, verify exact asset identities and
CLI/bundle checks, publish the existing draft as a beta and update real checksum
pins in the repository and Homebrew tap. Preserve failed attempts and limitations.

## Validation findings

- Initial candidate `efe382d3662897bf260b9f795f985411d9fb9555`: local CI
  passed (2,958 workspace/serial results and 925 adapter results; Python85,
  one macOS-only skip). The supervisor returned0 and reaped818 orphans;
  PID1 zombie identities remained the same767.
- The normal-release performance gate failed the 100k-line view-build ceiling:
  3.93ms initially, then5.25/3.76/3.67ms in isolated repetitions against2.4ms.
  Other compiled app component gates passed separately. Preserve this failure.
- Test lazy allocation of per-line inline ranges when no block has inline
  changes, preserve refinement/clearing behavior, compare retained production
  GUI/TUI binaries, rerun CI and gates, then rebuild the unpublished candidate.
  The initial tag/package run must never be mistaken for the final release.

- Lazy inline storage alone improved view build but remained above the ceiling.
  Direct row iterators improved it further; the full second gate still measured
  2.51ms. Reserving known uncollapsed sizes gives three alternating pairs at
  baseline4.09/3.74/3.96ms vs candidate2.29/2.16/2.01ms. All55 view correctness
  tests pass, including inline ranges arriving and being removed by refinement.
  Scroll samples overlap; no physical rendering improvement is claimed.

- Third complete normal-release performance gate passed all components; view
  build1.98ms. A subsequent runtime starting/ready transition discarded the outer
  execution session and left its completed supervisor (PID987810, exit0) as one
  additional PID1 zombie. No live validation descendants remained. All8 supervisor
  lifecycle checks passed afterward with the count stable at768. The fixture
  probe must use lenz-content (the initial lenz probe selected zero tests).

- First full suite stopped at the obsolete `Log filter:` PTY marker after26
  cases; retain full-1. The app's editable prompt uses a separate heading/field.
  Updated the collector to require heading plus visible editing controls and
  to wait for actual prompt/panel dismissal. Use workload version7 and keep
  version6 results separate. The corrected 2,000-line/11-round native repository
  and conflict workflow passes in12.33s; Python86 passes (one macOS skip).
  No additional product Rust change was needed. Revalidate the combined tree.

- CI3 stopped in the LSP writer-cleanup test. The old liveness probe called
  kill-0, then treated a missing /proc entry after reaping as alive. Diagnostics
  captured successful kill-0 followed by ENOENT. It reproduced in isolation;
  process counts stayed stable. Use one Linux /proc observation, account for
  disappearing entries and parse command names with spaces. A real-process
  running/zombie/reaped regression fails against the old helper and passes now.
  All160 available semantics tests and100 repeated writer-cleanup runs pass.
  Keep diagnostic builds/logs; this changes test fixtures, not production cleanup.

## Final local validation

The clean final source is `cc4411322c58756d5486e127ba9ef45af1772c16`.
Full local CI passes: 2,959 workspace/serial plus 925 dedicated frontend passing
results across invocations, zero failures; Python 86 tests with one macOS-only
skip. The complete normal-release gate and workload version 7 full suite pass:
133 cases, 394 metrics, no skipped workloads. Mixed stress actually lasts
60.027 seconds with zero last-quarter RSS growth. Final Code TUI stress lasts
10.042 seconds / 123 cycles on the 100k-line fixture.

Five startup pairs and six GUI/native-TUI interaction pairs pass. The extra
three reversed-order pairs leave Find-scroll and sub-millisecond resize
variation inconclusive; retain the measurements without claiming improvement.
All later process snapshots stay at 768 existing zombies, without fork failures.
Evidence and failed attempts are indexed in the
[release validation report](../../../docs/research/release-0.0.14-validation.md).

Final native run: [42784617241](https://github.com/aantoniadis/lenz/actions/runs/42784617241).
Publication and real distribution checksum updates remain pending that run.
