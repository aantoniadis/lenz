# Editor follow-ups

Record findings here instead of expanding the current editor implementation. Deal
with these after the approved work is done. A finding that blocks an approved
ticket's acceptance remains in that ticket; this file does not waive its checks.

## Deferred observations

- **TUI overhead:** the final snapshot-window comparison measured startup RSS
  +0.592 MiB and completed Go To median +0.085 ms, with substantial timing
  variation. Settled RSS decreased. Retain these observations for the batch's
  performance decision; do not start another optimization investigation now.
  Evidence: `docs/research/reader-snapshot-tui-performance.md` on the TUI branch
  and `target/perf/editor-implementation/09/native-final-summary.json`.
- **Find before results arrive:** `Session::find_step` currently returns without
  navigating while the match list is empty. Check whether Enter pressed during
  the initial asynchronous search should be remembered and applied when results
  arrive. This was found while preparing the native reveal regression.
- **Find query selection:** inspect Ctrl+A while the GUI Find field is focused.
  `ReaderView::find_key` currently lets this shortcut fall through to the parent
  handler. Verify the intended behavior before changing it; this is an observed
  routing question, not yet a demonstrated defect.
- **Additional native shaping coverage:** extend owned-display checks to more
  fonts and Arabic/Indic shaping at projection boundaries. The current native
  diagnostic covers six bounded samples; TestPlatform metrics differ from the
  native backend. Do not claim the broader coverage already exists.
- **Pathological first-content measurement:** measure native first feedback and
  completed layout separately for very long combining-mark runs. The retained
  window leak has a regression check; equivalent native startup measurements for
  that specific input remain a follow-up.
- **Pinned benchmark helper selection:** fake-provider fixture setup currently
  invokes Cargo before locating its helper beside the test executable. A future
  explicit prebuilt-helper option would make immutable benchmark archives easier
  to replay and prevent accidental compilation inside quiet runs. The current
  measurements retain the expected directory layout and use `CARGO=/usr/bin/true`.
- **Presentation work during native reveal:** review whether automatically issued
  commands such as `ColourResults` should supersede a pending native reveal. The
  cancellation classifier currently exempts a small set of adapter feedback
  commands. Reproduce semantic-result navigation with fresh result coloring before
  treating this as a defect; do not start a separate investigation now.
- **Local gate build concurrency:** the combined 08–10 gate launched many
  simultaneous linkers and showed substantial host I/O pressure while building
  workspace test binaries. Compare a bounded Cargo job count on a later cold
  build before changing the gate; this is an operational observation, not a
  demonstrated product regression. Evidence: the batch gate log under
  `target/perf/editor-implementation/merge-08-10/`.
