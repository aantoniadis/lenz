# Shared draft and history foundation

Ticket 12 adds `Command::Edit { tab, expected, edits, kind }`, `Undo { tab }`
and `Redo { tab }` at the existing Session boundary. The expected SnapshotKey
and all original-version UTF-8 ranges are validated before publication. Stale,
invalid or overlapping transactions change neither current text nor history.
Undo/redo and edits use the same version allocator as file loading.

Sources remains the sole current snapshot registry. A buffer's Draft holds its
confirmed text baseline and compact forward/inverse changes, rather than every
intermediate snapshot. Dirty state compares exact content with that baseline;
the presentation reads a cached boolean. It is independent of Git status and
of the saved semantic-index identity. Editing retains the initiating preview.
Images, including SVG source presentation, and retrieved sources remain read-only.

Adjacent typing and deletion from the same view coalesce within 500 ms. Deletion
direction must agree. Movement, selection, focus, paste and other intervening
user commands separate groups; asynchronous layout feedback does not. Groups
survive file switching. Separate views share current text/history while their
selection and scroll anchors transform independently. Separate Sessions own
separate drafts. Active edits reuse the bounded native caret-reveal path.

Dirty buffers and clean buffers with history are retained independently of the
reloadable inactive-cache budget. Rejected/no-op edits create no lasting baseline
owner. Current language-server text demand continues while a dirty view is
unloaded, through the existing bounded dependency scheduler. No new executor or
implicit contiguous current-document getter is added.

A large-deletion trace distinguishes a genuine confirmed-baseline owner from
intermediate snapshots, which expire. Surviving original chunks charge their
whole backing; replacing those remaining chunks releases that backing from the
current snapshot. History is not silently capped. Confirmed baseline advancement
and ownership retirement after Save belong to ticket 13; that operation must
preserve undo/redo and account separately for still-needed historical text.

Focused Session checks cover Unicode/CRLF, shared split edits, independent
Sessions, fresh versions, rejection, adjacent-removal undo, dirty equality,
500 ms grouping, actual cache eviction, split removal, image admission, compact
history ownership and semantic readiness after final-view release. Existing clean
cache-eviction and command-catalog checks remain green. Core library/test Clippy
passes. The adapter slice supplies real input/render replays with an explicit,
default-off test-support opt-in; production writable input remains gated until
the later editing/safety checkpoint. Save and lifecycle guards remain later work.

Raw focused evidence is retained under `target/perf/editor-implementation/12-core`.
The user requested one full gate and equivalent release/performance/stress checks
after batch 11/12/13/16/18. No standalone ticket-12 gate, performance acceptance,
native macOS or IME verification is claimed here.

The explicit Save continuation and baseline-retirement behavior are documented in
[Reader explicit Save](code-explicit-save.md).
