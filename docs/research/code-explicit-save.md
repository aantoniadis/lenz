# Reader explicit Save foundation

Ticket 13 extends the shared buffer/history from ticket 12 with explicit Save,
Rejected/Uncertain outcomes, disk recheck and matched confirmation. GUI/TUI expose
Save in the File command menu and on Ctrl+S; the same catalog provides Recheck
Saved File and Confirm Rechecked Save. Dirty remains separate from Git status.
Production writable input remains gated until its editor-input milestone.

`CurrentSource.save` retains the exact writer baseline/target and request state.
Loading, cached result previews, result validation and semantic navigation carry
optional exact-read provenance on their immutable loader/result Document. The
shared content `ExactRead` supplies raw bytes, same-descriptor metadata and target
identity; Reader uses those same bytes in its existing decoder, then borrows the
decoded encoding/BOM/lossy facts to build the writer snapshot. There is no second
read, decode or decoded-text copy. Reader stability retries, encoded-size policy,
images and virtual-source checks remain in place. A source without provenance
cannot silently become writable by reading a new baseline when Save starts.

Save reserves bounded ownership before capturing text, ends the typing group,
and pins its buffer until its outcome is accounted. The separate write-intent
lane shares the existing Analysis conversion cell, 64-owner admission and six
submitted CPU credits (including the existing three-background bound) on the
same three Jobs workers. Waiting for a source consumes no consumer worker slot.
A changed project/readiness context rejects before submission; no automatic
rebind/retry occurs. Submitted writes run independently of tab/project/read
cancellation. Their request-owned outcome cell, keyed by BufferId/request, is
polled exactly once; the channel only wakes dispatch, so a full inbox cannot lose
a completed disk outcome. A submitted worker panic is uncertain. Session Drop
cancels reads, closes the inbox, and finishes submitted writes before Jobs teardown.
This covers Session lifetime, not crash recovery or universal filesystem atomicity.

Confirmed success installs the returned writer identity and only the captured
text baseline. Undo/redo remain intact. Saving v2 after an edit creates v3 leaves
v3 dirty; undo to the exact v2 content becomes clean. A second Save uses refreshed
target identity. Saved semantic/index eligibility follows the confirmed captured
version; current text and metadata are not replaced by an older completion.
Loaded twin stamps are refreshed and shared blame observes the new disk identity.
A renamed source whose writer baseline still names the old path must explicitly
recheck/confirm the current path before Save. If metadata changes after capture,
a confirmed old-target write is reported without marking the new named source
clean; its draft/history/baseline remain protected until current-file recheck.
Save outcomes never launch an external action or resume an obsolete continuation.

Rejection retains draft/history/baseline and reports its cause. Uncertainty also
retains the attempted snapshot and blocks another Save until explicit recheck and
confirmation. Recheck reports whether the observed disk bytes match the last
confirmed baseline, the attempted Save, or neither; observing alone never changes
the baseline. Confirm Rechecked Save accepts only a matched observation, refreshes
its exact target identity and retains the draft/history. Unrelated or unreadable
disk content remains blocked for explicit reconciliation. There is no silent
retry or acceptance of a new baseline during Save. A fresh actual view-close decision may instead choose
**Discard Draft and Close** without rechecking: it abandons in-memory draft/history, leaves disk as it stands,
and never revives the failed Save/close. Replacement transitions and external opens retain their recheck guards. Existing writer behavior for
encoding/BOM/mixed endings/mode, symlinks and in-place hard links is reused.

## Backing retirement

When a confirmed baseline retires, Save may compact current/baseline chunks on the
existing bounded worker lane. It triggers only when deduplicated retained text
and newline-index backing exceeds twice the live UTF-8 plus live newline-index
bytes, plus 64 KiB. That measure is cached during ordinary snapshot construction;
checking it neither scans nor flattens the document on input/render. Chunkwise
compaction preserves source key, metadata, bytes, history and view positions.
A newer current version is matched before replacement; stale compaction results
cannot overwrite it, and the latest retained version is reconsidered.

The matching current or pending Analysis dependency is rebound to compact backing
while sharing its existing conversion cell/task and updating the conversion map's
weak demand handle. One retired weak dependency per compacted scope preserves
captured-result demand after the new current group closes. Reopening current
demand uses compact backing with the same conversion cell, rather than adopting
that old backing. Running conversion input and captured result owners can retain
their original immutable input until completion/acknowledgement. Tests distinguish
that legitimate lifetime from retired backing: actual original text/index Weak
probes expire after the conversion, without a second materialization. Compact undo
payloads remain legitimate history ownership; Save does not impose a history cap.

An unsuccessful initial criterion counted all index/bookkeeping overhead as waste,
causing dense-newline text to repack repeatedly. A 50,000-line Save reproduction
went red then green with the retained-backing/live-index distinction above.

## Focused evidence and remaining batch validation

- Shared Session Save → real cache eviction → return → Undo/Redo preserves exact
  Unicode/BOM/mixed-ending bytes and mode. Delayed Save captures only v2 while v3
  remains dirty; subsequent Save verifies refreshed identity.
- Actual GUI and terminal Ctrl+S traces both exercise real inactive-cache pressure
  with two 17 MiB sources, return and Undo. Original disk output remains saved.
- Real disposable-file fault tests cover known pre-write rejection, in-place
  failure/failed rollback and post-replacement uncertainty. Both adapters render
  rejection/uncertainty/recheck/confirmation, preserve modal input isolation and
  exercise narrow layouts. Terminal narrow status reserves space for Save notices.
- Writer regression tests cover encoding/mode/hard links, same-content target
  substitution, symlink retargeting, refreshed identity, lossy/unrepresentable
  refusal and the raw-read builder. Reader checks cover UTF-16LE/BE with BOM and
  Windows-1252; binary/lossy sources remain read-only.
- Focused scheduler checks cover reservation before capture, shared credits,
  pre-write context rejection, submitted panic, full notification inbox,
  project/read cancellation and exactly-once accounting. Backing retirement has
  small-slice and dense-index red/green traces.

The module regression run also exposed a pre-existing gated test ordering race:
it released workers before the central cancellation reap that Session dispatch
always performs. The fixture now pumps while its gates remain held; its original
exactly-one captured conversion and acknowledgement/release assertions remain.

Raw development logs/source identity are retained under
`target/perf/editor-implementation/13`. Early compiler diagnostics from adding the
new command/API and one compile overlapping the merge of the encoder/blame API
are not application regressions or performance evidence. Native macOS, physical
display latency, crash recovery and long-session behavior remain unverified.

Implementation readiness is separate from final acceptance: no standalone full
workspace gate, release comparison or sustained stress is claimed. Those run once
for the agreed 11/12/13/16/18 batch against the retained 08–10 baseline and raw
executables. No deliberate performance tradeoff is accepted by these focused tests.

## Fresh close decision after an unconfirmed Save (2026-10-24)

The explicitly approved exception is limited to actual Tab, Unsplit and Session close intents, including
all-view preparation. Save failure still cancels its original close ticket. A fresh current-identity decision
may abandon draft/history without rechecking or writing disk; submitted writes/rechecks must finish first.
Reload, workspace/Git transitions and external opening retain their existing guards.

Focused shared tests cover pre-replacement rejection, post-replacement uncertainty, stale tickets,
all-view closure and exact disk bytes (including an intervening external write). Actual GUI/TUI controls
show **Discard Draft and Close**, preserve dirty/history before consent and leave disk unchanged.
Existing recheck/confirmation and replacement-transition checks pass, as does affected strict Clippy.
Evidence is retained under `target/perf/editor-implementation/14-fresh-uncertain-discard/`. No native
campaign, full suite, timing or stress was repeated for this policy-only change.
