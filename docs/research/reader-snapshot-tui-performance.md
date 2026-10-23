# Bounded terminal snapshot rendering — ticket 09

The terminal source renderer now consumes shared, resumable snapshot projections
and exact shared logical rows. Native terminal cell measurement and hit testing
stay in the adapter; navigation, pending work, horizontal reveal policy and
validated scroll feedback stay behind `Session::dispatch`.

This report records the production candidate `ac6222a9dc228ac06b2442bcef0a5eaaea96eb6e`
against the accepted foundation baseline
`1d874aee50c620c10201d0695de0e97fe140a83a`. The
[integrated batch report](reader-snapshot-batch-08-10.md) records completed checks
and the user's acceptance of the documented costs.

## Correctness and ownership

- Source painting, wrapped navigation and hit coordinates use current snapshots,
  absolute UTF-8 source bytes and global projected display offsets. They no longer
  read the capped legacy `Tab` source/wrap renderer.
- Projection windows can split a normal grapheme. The adapter retains its bounded
  undecidable final grapheme and measures it with the following window. Ordinary
  family emoji, combining sequences and keycaps retain their terminal widths.
- Cold seeks reuse measured checkpoints and the existing per-pane projection
  worker lane. The adapter keeps viewport cells, bounded checkpoints and at most
  32 offscreen line entries; it does not retain a skipped source prefix or create
  another worker pool. A frame admits at most 128 KiB of raw projection work.
- Hit geometry validates tab, snapshot, metadata Arc, dimensions, scroll,
  hints/declarations, folds and relevant pane/header layout. Pending native seeks
  reject old source hits. Reload, followed rename, resize and late hint traces
  exercise stale geometry rejection.
- `NativeReveal` requests name an exact source position, tab, projection identity
  and native metric context. `MeasuredReveal` validates the current request and
  applies shared margin-three horizontal policy. `NativeViewport` validates
  complete-extent clamps, including semantic epoch and original scroll position;
  a pan away and back cannot revive an old clamp.
- Wrapped rows may pan horizontally when native cell width exceeds shared logical
  width. Measurement includes actual row indent and the EOF caret slot. A two-cell
  keycap in one text cell gets a one-cell indicator with its original source hit.
  Incomplete row extents cannot clamp the viewport.
- Resizing a pending seek replays its current shared layout intent or native source
  target at the new dimensions, using the same bounded worker lane.

Focused regressions passed for long ASCII/family GoTo, Find acceptance and exact
END copy, wrapped keycap overrun, EOF caret, one-text-cell keycap presentation,
pending wrapped resize, continued-row whitespace end hits, suppressed zero-width
tails, stale geometry, hint ownership release, and existing Unicode/split rendering.
The four public projection-session regressions and native reveal/clamp tests
passed. Strict TUI all-target Clippy also passed. Development used focused checks;
the full workspace/GUI/TUI/local gate is deferred to the single integration batch.

## Measurement setup

Both immutable executables were built in the ticket's private Cargo target with
the repository release profile and four build jobs, without release overrides or
GUI dependencies. The Linux host used Rust 1.99.1. Baseline executable SHA-256:
`40cfdc81db06f10d35c098e3e4294f4de710bf46ef194c945828c050d106f199`;
candidate:
`a41a275f6cbb29115b64bfee1ee777e582e918e0868deb33db5871be71f21f8e`.

Raw evidence is retained under the integration workspace's
`target/perf/editor-implementation/09/`: `baseline-identity.json`,
`candidate-ac6222a-identity.json`, immutable executables, source hashes,
`native-final-baseline-pty/`, `native-final-candidate-pty/`,
`native-final-unicode-pty/`, `native-startup-pairs/` and
`native-final-summary.json`. Each run retained raw PTY bytes and samples.

Canonical collector schema 2 was unchanged. Both runs used 100,000 lines,
120×36 initial terminal cells, 30 input rounds, 60 seconds requested stress,
isolated HOME/XDG state, disabled providers and `PATH=/nonexistent`. Builds and
other agents' timing work were held throughout the paired window. The additional
eight startup/GoTo pairs alternated baseline/candidate order with fresh application
state and warm filesystem cache; all 16 transcripts contained exactly 21,849 bytes.

| Observation | Baseline | Candidate |
| --- | ---: | ---: |
| First content, eight-pair median | 7.728 ms | 7.669 ms |
| GoTo first PTY feedback, eight-pair median | 0.479 ms | 0.472 ms |
| GoTo completed source marker, eight-pair median | 1.532 ms | 1.617 ms |
| Startup RSS, eight-pair median | 14.992 MiB | 15.584 MiB |
| Canonical input first-byte median | 0.418 ms | 0.439 ms |
| Canonical stress first-byte median | 0.486 ms | 0.499 ms |
| Actual stress duration / cycles | 60.076 s / 741 | 60.008 s / 740 |
| Settled RSS after canonical stress | 36.973 MiB | 36.789 MiB |
| Peak observed RSS, canonical run | 39.512 MiB | 40.254 MiB |
| Idle output / CPU | 0 bytes / 0 s | 0 bytes / 0 s |

Startup RSS increased by about 0.592 MiB in the repeated fresh-state samples.
Completed GoTo increased by 0.085 ms between medians; the median paired difference
was 0.067 ms, with individual paired differences from −0.937 to +0.602 ms. These
samples show substantial variation and do not establish a precise stable latency
cost. The input/stress differences and the single-pair peak RSS increase remain
reported observations. The user accepted the documented 08–10 batch costs on
2026-10-24, including startup RSS and completed GoTo, and requested continued
implementation. This is not a blanket preservation claim or approval of future costs.
Earlier baseline peak observations were about 40.2 MiB. The earlier candidate's
roughly 1.5 MiB settled-RSS increase did not recur in the final production run.

The separate real Unicode replay passed ordinary family emission, GoTo source
byte 50,001, Shift-Right plus OSC52 copying exactly END, shared Soft Wrap, End/Up,
28×10/1×1/120×36 resize, scrolled Help dismissal, unchanged fixture bytes, exit zero
and observable termios/alternate-screen restoration. Its 60.133-second wrapped
PageUp/PageDown stress completed 735 cycles with initial, settled and peak RSS all
11.207 MiB. Unwrapped GoTo first feedback was 0.468 ms and the completed END marker
was observed at 3.004 ms. This small fixture is correctness and sustained-resource
evidence, not a replacement for the canonical comparison.

## Experiments and limits

Removing unused UTF-16 conversion from raw projection reads preserved focused
projection correctness but did not remove the intermediate completed-GoTo cost.
The plain style lookup already bypassed snapshot line-range queries when syntax
and semantic tokens were absent; profiling did not support blaming that path.
The native reveal contract then removed the duplicate unwrapped logical-position
scan and fixed real GUI/PTY horizontal visibility. Final samples above supersede
the intermediate candidate measurements; all intermediate raw artifacts remain.

The first custom Unicode collector incorrectly expected END after End on a fixture
with an empty final line. Schema 2 removed that final newline and ensured a fresh
marker replay. Schema 3 additionally excludes the post-marker quiet interval from
completed-marker latency. Earlier failed/calibration artifacts remain, and their
33 ms completion values are not used above. Collector Python checks passed.

PTY observations measure emitted bytes, not physical terminal display latency.
RSS does not measure GPU memory or prove absence of leaks. Native macOS, physical
display behavior, and the final combined 08–10 executable remain unverified here.
Editing remains gated by the later editor-enablement tickets.

## Integration gate repair

The integrated gate exposed a real unwrapped vertical-navigation regression:
ordinary Up/Down had fallen back to source-byte offsets instead of the baseline's
display columns. The repair uses the same bounded shared layout position/column
queries as wrapped navigation. Original Unicode/CRLF and tab/inlay assertions pass,
as do a real two-cell TUI input/render replay and a public Session cold Down/Up
trace at display column 12,000. No full-prefix synchronous scan was restored.

Three older expectations were updated to exercise the selected native contract:
core horizontal visibility follows explicit measured feedback; wrapped explicit
pan is corrected by current complete extents while stale clamps remain inert; and
the TUI paints the translated viewport on its scheduled next frame. Terminal
extent checks now also assert visibility of the EOF caret cell. Focused affected
checks and strict core/TUI Clippy pass. These navigation repairs followed the
production measurements above; no additional stress or full gate was rerun here.
