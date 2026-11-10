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
