# Reader current-source ownership contraction

Ticket 11 removes the compatibility current `Document` owner and its whole-line
reader APIs. `Sources` owns the authoritative `Arc<TextSnapshot>` by `BufferId`.
A tab retains identity, explicit loaded/released state, reading positions and
version-scoped derived data. `State::view` resolves its current source; a snapshot
kept only by the inactive cache does not make its released tab loaded.

The inactive cache retains reloadable snapshots within the existing 32 MiB
budget. Taking, stale stamps, replacement, oversized refusal, LRU eviction,
unavailable-source recovery and split lifetime retain their behavior. Source
eviction also invalidates analysis ownership. Canonical aliases share a buffer;
separate Sessions keep separate sources. Metadata-only rename changes the
retained source-metadata identity while preserving text, version and held old
snapshot metadata.

Contiguous Documents remain explicit loader, admitted analysis, protocol and
immutable search/semantic-result payloads. A weak loader Document permits reuse
by the bounded analysis service; it is not a current-text owner or the saved
baseline. A separate confirmed saved source/version fence controls saved-index
eligibility. Selection expansion uses per-view analysis demand. Current-file search, outline,
current previews and target validation instead use typed, independently owned
result slots in the existing bounded scheduler when they require contiguous text.
There is no new pool or implicit materializing current getter. Project draft
overlay remains ticket 16.

Exact-version source dependencies share one materialization cell between current
analysis and captured result intents. Conversion records are weak; admitted
owners retain their dependencies until replaced, cancelled or acknowledged.
Materialization workers capture the snapshot/cell/cancellation rather than the
owner handle. Waiting result intents consume no consumer CPU slot and never
block a worker waiting for another worker. The existing combined 64-owner,
six-submission and three-background bounds remain in force. A captured version
can finish after its view is released or a newer current version replaces it;
acknowledgement releases the final captured owner. Search/outline may publish
captured content; opening a validated result rechecks any retained current target
and rejects changed content or a renamed captured target.

A gated Session regression caught current-file search being cancelled when its
source view was released. It now completes with both exact captured-file hits
while another file remains current and both files remain unchanged on disk.
Gated scheduler checks also cover version replacement, shared conversion,
publication acknowledgement, capacity, cancellation and owner release.

One focused regression caught an intermediate mistake: admitting a newer
immutable preview result into the cache could replace an already loaded source.
The result/cache boundary now leaves loaded sources alone. The public workflow
asserts the preview payload, unchanged current Arc, exact Copy output and
unchanged disk bytes.

Current status, coordinates, navigation anchors, notices, availability,
breadcrumbs, provider status and adapter presentation read snapshots and source
metadata. Syntax/folds/declarations, semantic decorations and Markdown previews
are checked against their current source identity. GUI/TUI geometry keeps the
existing snapshot, metadata, hint and layout guards. Terminal preview selection
also retains metadata identity. An inactive retained Markdown tab keeps its
preview scroll while its reloadable snapshot is released; a real terminal
input/render replay verifies its restored first visible byte. Virtual sources
reopen through their existing buffer without another provider request or a disk
fallback. Navigation anchors requested before target loading remain pending and
are restored once its snapshot arrives. Navigation context reads at most 120 characters
(480 UTF-8 bytes) per line; comparing an anchor does not allocate a String per
line. Existing context relocation may still examine many lines once during a
reload; it does not materialize their complete text.

The old `Tab.document`, `State.document`, Tab whole-line rows/display/hit helpers
and legacy wrap cache are removed. The pure `shown_line` formatter remains for
immutable result previews and reference tests; current-source paint/input uses
the bounded projection and layout APIs from tickets 08/09.

## Accounting and loaded-file policy

`TextSnapshot::storage_bytes` now describes accounted retained source/index
storage, replacing its former meaning of additional storage beside a Document.
It includes source metadata/path/encoding/image storage, snapshot/chunk/index
bookkeeping and Arc headers. Each distinct original or local-edit text/newline
allocation is counted once within the snapshot, including its full backing when
only a small slice survives. Original shared newline indexes include their
initial offset. Accounting runs at source construction/edit, not per frame.
An empty zero-chunk snapshot retains metadata and its empty index, but does not
retain the discarded loader's text or original line-start allocation. Allocator
rounding/arenas, renderer resources and separately bounded worker/protocol
representations are not claimed as this cache's byte total or as RSS.

Source metadata records the encoded load's reader restriction and load version.
An unchanged snapshot and a worker rematerialized without the loader preserve
that policy in both directions: a large ASCII UTF-16 load stays restricted even
when decoded UTF-8 is smaller, while a permitted UTF-16 CJK load stays permitted
when its decoded UTF-8 is larger. A changed draft keeps the original restriction
and additionally applies the explicit normalized current-text cost limit.

## Validation boundary

Development uses the approved shared Session command/View seam, existing real
GUI/TUI input/render tests and focused storage/provider resource boundaries.
Red/green evidence includes retained backing after large deletion, both encoded
size directions, current status before worker admission and preview/current
source separation. The affected public semantic/navigation target passes 98
tests (five existing ignored checks). Focused current-source regressions,
materialization checks, captured-result lifetime/validation and core all-target
Clippy pass. Adapter workers verified actual GUI/TUI empty, image, Markdown,
reload, native Go To, split and click workflows and affected feature-test Clippy.
Protocol retention checks observe the actual desired Document text allocation,
including a positive blocked-writer retention control and release after the final
request barrier; original reload/expansion counts and memory limits are preserved.
Existing cache pressure, split refresh, alias/rename,
independent Session, empty/image/SVG, Markdown, native Go To, style and stale
geometry checks were migrated to current snapshots rather than retaining a
production compatibility getter for fixtures.

Focused results and raw logs are retained under
`target/perf/editor-implementation/11-core`, `11/session-tests`,
`11/result-readiness` and `11-adapter-tests`; the final
handoff records exact source identities and checks. Early relative-target fake
provider setup failures are excluded as runtime evidence: a child Cargo invocation
built the helper in a nested crate target, while the fixture expected the private
workspace target. The corrected absolute-target reruns pass.

Implementation readiness is separate from final batch acceptance. No standalone
ticket-11 full suite, release comparison or stress run is claimed. The user
requested one full gate and equivalent performance/stress validation after the
next batch (11/12/13/16/18). Baseline source `bb162738` and the accepted 08–10 raw
samples/executables remain retained by the integration owner. Native macOS,
physical display latency, GPU memory and longer sessions remain unverified.
