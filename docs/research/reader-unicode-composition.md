# Unicode composition and long-line editing

Ticket 22 adds a transient, read-only input-method presentation through the
existing View, logical-layout, projection and native Row pipeline. Session
commands validate its captured source, metadata, selection and pane before
registering it. Unique versions come from the shared Session allocator. Its
separate tab/layout cache never enters Sources, draft history, Save, providers
or a sibling pane. Existing bounded layout/projection workers also accept this
exact registered snapshot and reject its results after replacement or cancel.
There is no additional executor, full-document facade or second shaper.

GUI callbacks share one virtual coordinate frame: the captured source prefix,
pending replacement and source suffix. Partial replacements within the marked
text preserve the remaining preedit. Queries and geometry use absolute UTF-16
coordinates; supplementary-scalar interiors are rejected before clipping.
Surrounding-text allocation is capped at 4 KiB with an absolute adjusted range.
AppKit marked replacements use explicit relative coordinates, following the
pinned GPUI editor; its insertText accepts the marked region. Linux callbacks
use absolute replacement ranges. Unsupported expansion outside the pending
marked splice is refused atomically, rather than guessed from which range fits.

Preedit has an underline and reuses bounded wrapping, shaping and hit mapping,
including multiline text. Base syntax, hints, folds, occurrences, attribution
and automatic source documentation are suppressed in the shifted presentation;
the annotation gutter retains its width. Source and permission changes,
selection/focus/modal changes, viewport, font and wrap changes invalidate the
capture. Obsolete terminal callbacks remain tombstoned, rather than becoming
ordinary typing in a newly selected source. A weak post-terminal source and
context fence permits a new marked begin in that exact ended context, including
X11's preedit-before-dead-key ordering; changed contexts cannot revive it.

An explicit commit or unmark accepts the pending splice as one captured-version
Replace transaction. Repeated unmark does not add history. Escape and the pinned
Linux marked-range cancellation discard the presentation without changing the
buffer or history. Ordinary source input continues through ticket 21's shared
commands. The EntityInputHandler is registered in canvas paint, and committed
printable input uses GPUI's delivery path. Already-handled Enter, Tab and shortcut
commands stop propagation so the framework does not insert their text twice.
All writable input remains behind the existing test-support opt-in and the
shared capability gate; this change does not enable production typing.

Focused evidence includes core isolation and actual layout-worker publication /
late-cancel checks, plus real Reader TestWindow callbacks through
PlatformInputHandler and ElementInputHandler. They cover long-line and wrapped
multiline preedit, partial replacements, explicit commit, unmark, cancellation,
surrogate refusal before distant-query clipping, candidate-bound / point
roundtrips, stale source including Undo-to-identical-text, selection, read-only,
modal and resize changes. Repeated composition after commit and cancellation
retains distinct Undo steps without reviving old terminal callbacks. A painted
simulate_input replay establishes actual
registration and exactly-once committed delivery. The existing ordinary-input
parity test caught duplicate Enter/Tab fallback and now passes with the fix.

Real GUI/TUI replays type, select, paste, undo across split panes and hit-test
past character 12,000 and near the end of a roughly 1 MiB Unicode line containing
emoji, combining marks, tabs and control characters, in dark and light
appearances. They compare exact source/state and unchanged fixture bytes.
Test-only per-thread counters establish that these input/render callbacks call
neither full Document conversion API; background analysis remains separate.

Persistent-chunk preedit construction rebuilds the chunk-entry index and copies
the explicit pending text. It is not constant time in document chunk count.
Measure this cost at several document sizes in the agreed multi-ticket batch;
no per-ticket benchmark or stress campaign was run. The full gate, performance
acceptance and native-system input delivery remain separate batch / checkpoint
work. Native macOS, physical display latency and live input-method cancellation
sequences remain unverified; no macOS runner was available.

Raw focused logs and pinned configuration/source identities are retained under
`target/perf/editor-implementation/22-core` and `22-adapters` in the integration
checkout. See [basic input](reader-text-input.md) for the shared command and
production-gate boundary.
