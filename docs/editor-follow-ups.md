# Editor follow-ups

Record findings here instead of expanding the current editor implementation. Deal
with these after the approved work is done. A finding that blocks an approved
ticket's acceptance remains in that ticket; this file does not waive its checks.

## Deferred observations

- **TUI overhead:** the final snapshot-window comparison measured startup RSS
  +0.592 MiB and completed Go To median +0.085 ms, with substantial timing
  variation. Settled RSS decreased. The user accepted the documented batch costs
  on 2026-10-24; retain this as a later optimization opportunity.
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
- **Unwrapped shift-arrow column policy:** ordinary Up/Down again preserves shared
  display columns across Unicode, expanded tabs and hints. The pre-existing
  unwrapped `Extend` path retains source-byte columns. Evaluate that separate
  selection policy before making shift-arrow behavior match ordinary navigation;
  it was not changed by the integration gate repair.
- **Local gate build concurrency:** the combined 08–10 gate launched many
  simultaneous linkers and showed substantial host I/O pressure while building
  workspace test binaries. Compare a bounded Cargo job count on a later cold
  build before changing the gate; this is an operational observation, not a
  demonstrated product regression. Evidence: the batch gate log under
  `target/perf/editor-implementation/merge-08-10/`.
- **Draft-aware blame presentation:** ticket18 now invalidates old annotations and
  commit actions on draft edits and recomputes against admitted current bytes.
  Focused core and opt-in GUI/TUI input/render traces cover insertion, newline
  deletion, Undo and failure. Final acceptance and performance remain pending the
  11/12/13/16/18 batch; production typing remains gated for21/22.
  Legacy drafts containing characters their original encoding cannot represent,
  and lossy-decoded sources, show explicit Failed attribution with commit actions
  disabled. Representable legacy bytes retain exact attribution; this boundary
  follows the strict writer encoder rather than substituting invented bytes.
- **Typing-group allocation:** compact history currently joins inserted/deleted
  group text when adjacent edits coalesce. This copies the group's changed text,
  not the document. Use the scheduled batch measurements before deciding whether
  mutable group-string capacity is worthwhile; no extra benchmark run is needed
  per ticket.

- **Many-tab dirty indicator cost:** `State::tab_dirty(TabId)` currently locates
  the tab by scanning the tab collection. Header painting now queries dirty state
  for each tab; include many-tab counts in the planned batch measurements before
  deciding whether to expose a borrowed-tab or iterator form. The draft dirty
  comparison itself is constant time; no text is copied by this lookup.

- **Clock-dependent blame fixtures:** the focused18 run crossed UTC midnight and
  exposed a status assertion comparing the commit's creation date with a later
  `today()` call. Preserve the exact attribution assertion and rerun that single
  case with a fresh fixture for18; deterministic fixture dates remain deferred.
  Evidence: `target/perf/editor-implementation/18-blame/core-final.log`.

- **Inactive-tab Save feedback:** buffer-owned rejection/uncertainty and recheck
  state survive navigation; activating the tab reveals its full shared status.
  Headers currently expose its dirty marker. Consider a persistent inactive-tab
  save-state badge or actionable session notification, using the same core
  state rather than frontend-owned errors. This is presentation followup;
  draft/history retention and Save blocking remain enforced while inactive.
- **Compact-index query batching:** declaration and modal symbol continuations
  currently rank up to 32 adjacent saved-file record sets per turn. Use the
  scheduled batch comparison on a many-file index before changing that batch
  size; preserve independent query cancellation and completed-action timing.
- **Intermittent GUI declaration tracer timeout:** the final batch GUI run at
  `bb2f55f4` timed out once in `outline_and_declaration_search_through_keys`.
  The unchanged test then passed in isolation, in 25 consecutive isolated
  reproductions, and in a complete GUI confirmation run. No cause or runtime fix
  is claimed. Add stage-specific failure diagnostics or a controlled scheduler
  reproduction if it recurs; preserve the keyboard, selection and activation
  assertions. Original failure and all attempts are retained under
  `target/perf/editor-implementation/merge-11-18/reader-gui-*.log`.
- **Ticket14 native delivery acceptance:** production Reader windows install a
  veto, and focused tests replay that exact handler plus real keyboard decisions.
  GPUI TestPlatform has no native close-event simulator; replay one ordinary Linux
  titlebar close via an owned display at the next adapter batch boundary with
  disposable fixtures using the existing test-support draft gate. Native macOS
  termination remains ticket03/24, unverified.


## Reader batch performance attribution (2026-10-24)

- Distinguish live admitted backing from anonymous allocator residency after large-file Find/reload. The 11–18
  campaign retains candidate TUI 37.59/46.10–46.42MiB residency bands; phase samples put most of the upper-band
  difference in anonymous memory. Current source admission invalidates old analysis scope, and no leak or
  exclusive allocator cause has been proved. Do not average away the upper-band cost. Evidence and attribution
  limits are in the [batch performance report](research/reader-editor-batch-11-18-performance.md).

- Deferred optimization candidate, not implemented: unchanged snapshots created by `Chunk::shared` retain the
  original full text/index Arcs behind their chunk ranges. Investigate sharing that backing during admitted
  `materialize_while` conversion instead of String→Arc flattening when complete contiguous coverage is verified.
  Large originals span several ranges; do not assume a single full-range chunk or apply this to arbitrary edited
  snapshots. Preserve source/version fences, worker admission, cancellation and edited/chunked fallback behavior.
  This may reduce conversion allocation; it is not a measured explanation or guaranteed remedy for the RSS bands.
