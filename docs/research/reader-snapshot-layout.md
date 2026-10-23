# Bounded source projection and logical rows

Ticket 08's shared core supplies both adapters with snapshot-backed projection and
resumable logical wrapping. Authoritative current text remains in `Sources`;
derived cursors and caches retain source identity, metadata and hint handles, not
another current-text snapshot. Queued/running jobs borrow an `Arc<TextSnapshot>`
for their bounded work.

## Contracts and limits

- `projection::Cursor` identifies the snapshot version, retained source metadata
  and hint set. A followed rename can preserve `SnapshotKey`, so metadata identity
  is checked too. The stream is independent of viewport width and native metrics.
- Each projection window charges source/hint progress and emitted display bytes,
  with a maximum 4,096 work bytes. It can stop inside a hint or suppressed mark
  run. Source coordinates are absolute UTF-8 offsets; display ranges are global
  bytes in the projected line, and each `Shown` map is local to its window.
- Source tabs, control replacement and a continuous 30-mark suppression rule run
  before a streaming grapheme guard. Source and inlay-hint scalars share the mark
  count, including across adjacent labels, projection windows and logical rows.
  Suppressed hint scalars still consume work and retain their original anchor;
  type carets remain before the type label. If another scalar would extend a projected
  grapheme beyond 256 UTF-8 bytes, that scalar is displayed as `[U+XXXX]`. A
  preceding Unicode Prepend may attach to `[`, so the maximum projected grapheme
  is 257 bytes. The remaining ASCII escape creates a boundary. No invisible
  separator or synthetic editable source byte is inserted.
- Ordinary emoji, combining, Indic, flag and keycap sequences below the limit
  retain their display. Escapes map to their original scalar range, hints to
  their source anchor; Copy reads the original snapshot. Adapters separately
  bound their complete native shaping/measurement input.
- Logical wrapping sums scalar widths, preserving its whitespace, punctuation
  and hanging-indent conventions. Breaks occur between complete projected
  graphemes. A grapheme wider than the logical room stays whole. These logical
  columns do not represent terminal cells or font pixels.
- Wrapped logical rows also have a strict projected-byte ceiling,
  `layout::ROW_BYTES = 4096`. Before a complete grapheme would exceed it, the
  scanner creates a real continuation row, including within leading or hanging
  whitespace and inlay labels. No display or source tail is omitted. Graphemes
  are already bounded to 257 bytes, so no overshoot is needed. Continuation
  indentation is at most `min(cols / 2, ROW_BYTES / 2)`; a row body plus its ASCII
  indent therefore occupies at most 6,144 bytes. Normal wrapping below the byte
  ceiling is unchanged. This bounds wrapped native input independently of a
  whitespace run's total length; unwrapped adapters still bound their own native
  shaping/measurement input.
- `layout::Row` has absolute source and global display ranges. `complete=false`
  is a stable prefix with an unknown row end; its final whitespace is not an
  end-of-row hit target. An unwrapped row has known source extent and one visual
  row without counting its projected tail; native projection finds horizontal
  extent.
- `View::layout_rows`, `layout_position`, `layout_cursor`, `layout_step` and
  related queries return exact results or resumable pending work. One synchronous
  facade call spends at most 4,096 source/display work bytes across at most 64
  small queries. A worker result spends at most 65,536 work bytes, checking
  cancellation between projection windows. Multi-line stepping also yields
  after 1,024 metadata steps per query.
- Each tab's derived cache retains at most 128 source lines, 128 sparse
  checkpoints per line, 4,096 recent rows per line, 128 mapping points per kind,
  16 query continuations and 16 multi-line step continuations. An estimated 4 MiB
  limit evicts older lines. Checkpoints coarsen rather than growing with every
  row; their small carried Unicode state remains bounded. Recent row context
  and completed query checkpoints support bidirectional scrolling without
  repeatedly scanning a long prefix.

`PrepareProjection` and `PrepareLayout` use separate lanes on the existing
`Jobs` pool. Each pane has one running job and one replaceable pending request
per lane. Layout never waits for contiguous `Document` analysis. Session checks
tab, snapshot, metadata, hints and width before publishing. Semantic cold seeks
resume only while their selection, top position and visibility context are
current; later input supersedes them even if that input leaves the position
unchanged. Adapter dimension/projection feedback is not semantic navigation.
Session cancels the lanes before closing its completion receiver on drop.

Core caret movement, wrapped scrolling, End, selection visibility, navigation
reveal and sticky-band clearance consume these exact rows. Initial wrapped arrow
input without a caret starts at the visible top row. Existing whole-line helpers
remain compatibility APIs for consumers outside this migration.

## Focused verification

The shared Session long-line Down regression first selected byte 10,000 instead
of 12,010. The migrated path passes. A 400,000-byte line exercises End, Up,
initial arrow input near the tail, Home superseding an unfinished End and later
Left input superseding an unfinished Down. A 200,000-byte fixture requires
multiple real worker completions to reach a cold row.

Primitive checks cover later-line CRLF absolute origins, hanging indentation,
ordinary grapheme boundaries, overwide emoji, source/hint mapping, fragmented
labels, suppressed marks, direct Prepend overflow, source metadata changes and
width changes. The hint-mark regression first emitted 409,467 projected bytes
instead of 105; the continuous guard passes at 16, 31, 64 and 4,096 work budgets.
The row-limit regression first returned one 50,000-byte whitespace row, and the
wide-column indentation regression first returned an 8,193-column indent.
Leading/trailing whitespace, whitespace with marks, whole graphemes at the byte
boundary, one huge or many adjacent hint labels, continuation mapping and exact
source Copy now pass the strict row/indent bounds. Work-limit checks compare equal first rows with 100,000 and
1,000,000 unseen whitespace bytes. The metadata stress seeks End in 200,000
one-column rows, visits 6,000 rows, then performs 1,000 forward/backward row
queries while asserting bounded retention and neighborhood reuse.

The real Session command boundary also exercises byte-limited whitespace rows:
Down reaches source byte 4,096, Shift+Down reaches 8,192, Copy returns exactly
4,096 original spaces, and Up returns to byte 4,096. The file remains unchanged.

Relevant pre-existing wrap regressions pass, including hint caret placement,
selection visibility, short wrapped target rows, resize and navigation reveal.
Strict core Clippy checks pass. The full integrated gate is run once for the
08–10 batch, after GUI/TUI integration.

## Performance evidence and remaining validation

The clean `1d874ae` release GUI baseline and owned-display workload were saved
before architecture edits under `target/perf/editor-implementation/08/` in the
integration checkout. GUI and TUI baseline/candidate measurements cover the
shared path through actual adapter input and paint. The bounded-work assertions
above establish work and retention limits; their test elapsed times are not
startup, display-latency or leak measurements. The batch's performance reports
record final source/executable hashes and equivalent workload samples.

Native macOS input, shaping and window behavior remain unverified without a
native runner. See the [benchmark guide](../benchmarking.md) and
[adapter capability limits](../tui.md).
