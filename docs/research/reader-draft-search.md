# Draft-aware project and symbol search

Ticket 16 overlays current open-file snapshots on disk sources for project text
search, Find Declaration, and Search Everywhere's All/Symbols scopes. Loaded
sources and buffers pinned by drafts/history override saved matches; unopened
paths use disk. Inventory filtering and canonical aliases remain respected.
Text traversal preserves folder/path order, so result rows and preview indices
remain stable, with one cumulative 10,000-hit cap. Declaration ranking folds
into the existing best 200 and total match count; modal output keeps its
existing 100-row limit.

Panel queries use sequential Search-slot continuations; modal symbol queries
have an independent Navigation slot in the same admission service. Both share
source conversion, cancellation, leases, extraction and identity helpers. The
existing combined 64-owner, six-submission and three-background bounds remain;
there is no second executor or worker waiting for another worker. Closing or
replacing a modal query leaves independently displayed panel work alone.

Each query captures one source map. Hit previews retain their exact snapshots;
processed no-hit sources release text and keep compact identities. File-path
exclusions remain compact. Later draft edits mark the query stale without
replacing its captured preview. Activation checks current buffer/version,
metadata and path mapping again at publication, including undo to identical
text and version-preserving rename. A missing current captured source refuses
activation rather than substituting disk contents.

Disk results carry Stamp plus a separate SHA-256 digest of decoded text from
the read/extraction that supplied their coordinates. Declaration records retain
this identity with or without the disposable cache. The existing conditional
language/raw-byte cache fingerprint retains its original meaning. Stable preview
and activation reads must match the captured Stamp and decoded digest; changed
sources report stale instead of displaying another version. This proves decoded
coordinates, not raw identity across equivalent encodings.

Exact source fences also permit reuse of existing declaration records within
the same 32 MiB index budget, avoiding repeated parsing of unchanged open sources
for modal query edits. Digest and weak scope bookkeeping are charged to that
budget. Query records do not enter the persistent saved-file cache. Activation
promotes the checked payload when its target is unloaded, recovers authoritative
retained text first, and supersedes redundant loading. Cache refusal cannot
replace the checked source with a later reread. Promotable disk payloads use
ticket 13's exact-read decoder, retaining Save provenance.

Focused Session traces cover shifted drafts plus disk, removed saved matches,
folder runs/filters/caps, aliases, historical previews, held validation, same-text
new versions, metadata rename, query replacement/close and weak release. Disk
checks cover cache on/off, UTF-8/BOM/UTF-16/legacy decoding and same-size/mtime
rewrites. Panel and symbol activation followed by Edit/Save preserves BOM/CRLF.
The real GUI/TUI replays cover text, declarations, All/Symbols, stale refusal and
visible picker/result feedback. Production typing remains gated.

Raw evidence and source/configuration identities are under
`target/perf/editor-implementation/16-core` and `16-adapter-tests`. Affected
core/adapter lint passes at the recorded checkpoints. The full gate and equivalent
release/performance/stress comparison remain the shared 11/12/13/16/18 batch;
no standalone ticket-16 performance acceptance or native macOS/IME proof is claimed.
