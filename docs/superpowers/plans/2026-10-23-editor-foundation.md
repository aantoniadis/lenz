# Integrated Editor Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver safe existing-file editing, shared split/comparison drafts and preserved Reader workflows in both adapters as the first internal editor milestone.

**Architecture:** A session owns buffers, confirmed save baselines and compact undo history. Tabs reference buffer identity and retain independent view state. Immutable indexed chunks serve current input/rendering; explicitly materialized contiguous Documents serve admitted background analysis. Commands own edits, saves, guards and publication; adapters translate input and present state.

**Tech Stack:** Existing Rust workspace, GPUI, ratatui/crossterm, `lenz-content` writer/Git helpers and `lenz-engine`; no new runtime dependency is proposed.

**Spec:** [Accepted integrated editor design](../specs/2026-10-23-integrated-editor-design.md). [Readiness evidence](../../research/editor-readiness.md) governs measured claims and provider blockers. This plan is proposed implementation work; readiness probes did not implement it.

## Global Constraints

- "Within one session, a file has one draft and undo history shared by its tabs, split panes and comparison views." Independent sessions remain independent.
- "Save is explicit by default." "Saving preserves the session's ability to undo edits."
- "Project search, symbol search and navigation use unsaved drafts for open files and disk for unopened files."
- "Keep state transitions, availability, validation, undo, file safety and background orchestration in the shared core; adapters own input translation and presentation."
- "An action requiring changes outside the current document is unavailable in this release with an explanation; never partially apply it."
- "Keep source contents and database credentials out of diagnostic logs and shareable project configuration."
- Preserve existing Diff/merge/repository/PR behavior, separate frontend dependencies and the measured release baseline. Read [benchmarking](../../benchmarking.md), [optimization findings](../../optimization-notebook.md), [Reader performance](../../code/performance.md), [GUI/TUI parity](../../tui.md) and [Reader terminal limits](../../code/tui.md) before implementation.
- Typing becomes reachable only after transition protection, current-text navigation and both adapter controls exist. No intermediate commit may expose editable drafts with an unguarded close/refresh path.
- This milestone covers existing files. New/Save As, durable recovery, complete editing commands, language completion, HEAD gutters and richer comparisons remain required later deliverables, not removed scope.

## Review Focus

1. Retargeted symlinks, hard links and same-content path substitution must not redirect a Save unexpectedly; exercise Task 2's identity/failure fixtures.
2. A save completing after another edit or after an attempted project change must never mark newer text saved or resume an obsolete action; exercise Tasks 2 and 4's delayed completions.
3. A native close or all-view quit must not bypass a modal or destroy an earlier window's draft when another window cancels; exercise Task 4's runtime callbacks, not only tab commands.
4. Non-BMP/combining text beyond character 10,000 must remain editable and hit-testable without full-line/full-document conversion; exercise Tasks 1, 3 and 7's long-line/IME checks.
5. A result from clean A targeting dirty B must not activate obsolete B coordinates or reuse stale blame after B changes; exercise Tasks 5 and 6's captured-target cases.

## Files and ownership

| Area | Files to create or change | Responsibility |
| --- | --- | --- |
| Current text and history | New `crates/reader-core/src/text.rs`, `buffer.rs`; existing `document.rs`, `lib.rs` | Indexed chunks, immutable snapshots, shared metadata, compact transactions and confirmed baselines |
| Session/view adoption | `session.rs`, `tabs.rs`, `cache.rs`, `shown.rs`, `wrap.rs`, `places.rs`, `expand.rs` | One owner per buffer; view-local anchors/layout; bounded current-text access |
| Analysis/navigation | New `crates/reader-core/src/analysis.rs`; existing `find.rs`, `search.rs`, `index.rs`, `markdown.rs`, `semantics/mod.rs`, `semantics/lsp.rs`, `decor.rs`, `navigation.rs` | Coalesced materialization, versioned products, draft overlays and target validation |
| Shared writes | New `crates/content/src/write.rs`; content `lib.rs`, `Cargo.toml`; app `write.rs`, `edit_session.rs`, `Cargo.toml` | Extract writer primitives, preserve Diff API, add exact-ending Reader outcome API and test-only fault feature |
| Guards and close | New `crates/reader-core/src/transition.rs`, `close.rs`; session command/availability; both adapter lifecycle files | Deferred destructive actions, save outcomes, native close and all-session prepare/commit |
| Blame/comparison | `reader-core/src/blame.rs`; new `comparison.rs`; core `Cargo.toml` | Draft attribution and last-saved immutable/current sides using the existing engine |
| GUI/TUI | New `crates/reader-gui/src/editing.rs`; GUI `lib.rs`, `tests.rs`; TUI `app.rs`, `runtime.rs`, `render.rs`, `input_tests.rs`, `render_tests.rs`, `parity_tests.rs` | Input/IME, prompts, current-draft rendering and semantic effects |
| Proof | New `crates/reader-core/tests/editor_foundation.rs`; existing writer/core/adapter tests; benchmark scripts only where necessary | Behavioral traces, fault cases, actual adapter parity and equivalent release workloads |

## Shared interfaces

The names below are the contract between tasks. New fields stay private unless listed as command payloads or read-only snapshots. `State` owns the registry; only `Session::dispatch` mutates it. Do not reuse Diff's comparison-specific `EditSession` as a Reader buffer.

```rust
// text.rs / buffer.rs
pub struct BufferId(pub u64);
pub struct SnapshotKey { pub buffer: BufferId, pub version: u64 }
pub struct TextEdit { pub range: Range<usize>, pub text: Arc<str> }
pub enum EditKind { Typing, Delete, Paste, Replace }
pub struct TextWindow<'a> {
    pub bytes: Range<usize>,     // Absolute UTF-8 source bytes.
    pub utf16: Range<usize>,     // Absolute document UTF-16 units.
    pub text: Cow<'a, str>,
}
pub struct TextSnapshot; // key, shared SourceMeta, indexed persistent chunks
pub struct Buffer;       // current, confirmed baseline, undo/redo, save/conflict state

impl TextSnapshot {
    pub fn key(&self) -> SnapshotKey;
    pub fn source(&self) -> &Path;
    pub fn language(&self) -> Option<lenz_content::highlight::Language>;
    pub fn encoding(&self) -> &str;
    pub fn len_bytes(&self) -> usize;
    pub fn line_count(&self) -> usize;
    pub fn line_range(&self, line: usize) -> Option<Range<usize>>;
    pub fn line_with_ending(&self, line: usize) -> Option<Range<usize>>;
    pub fn line_of(&self, byte: usize) -> usize;
    pub fn floor_char_boundary(&self, byte: usize) -> usize;
    pub fn previous_char_boundary(&self, byte: usize) -> usize;
    pub fn next_char_boundary(&self, byte: usize) -> usize;
    pub fn byte_to_utf16(&self, byte: usize) -> Option<usize>;
    pub fn utf16_to_byte(&self, unit: usize) -> Option<usize>;
    pub fn line_window(&self, line: usize, from: usize, max_bytes: usize)
        -> Option<TextWindow<'_>>;
    pub fn copy_range(&self, bytes: Range<usize>) -> Option<String>;
}

// tabs.rs / session.rs: None while a tab has not loaded or has no text source.
pub struct View<'a>; // borrows Tab and its Buffer; contains no duplicate current text
impl State { pub fn view(&self, tab: TabId) -> Option<View<'_>>; }
impl View<'_> {
    pub fn tab(&self) -> &Tab;
    pub fn snapshot(&self) -> &Arc<TextSnapshot>;
    pub fn dirty(&self) -> bool;
    pub fn edit_disabled(&self) -> Option<&str>;
}

// analysis.rs; Document retains contiguous worker APIs, never a hidden lazy flatten.
pub struct AnalysisInput { pub key: SnapshotKey, pub document: Arc<Document> }
pub struct AnalysisRequest { pub key: SnapshotKey, pub context: u64, pub request: u64 }
fn materialize(snapshot: Arc<TextSnapshot>, current: impl Fn() -> bool)
    -> Option<AnalysisInput>; // private, called only on an admitted worker

// Add session commands; existing navigation commands retain their names.
Command::Edit { tab: TabId, expected: SnapshotKey, edits: Vec<TextEdit>, kind: EditKind }
Command::Undo
Command::Redo
Command::Save
Command::SetReadOnly(bool)
Command::CompareSaved
Command::CloseComparison
Command::ResolveTransition { id: u64, choice: TransitionChoice }
Command::PrepareClose { ticket: u64 }
Command::CommitClose { ticket: u64 }
Command::CancelClose { ticket: u64 }
```

`SourceMeta` consolidates the existing Document path/encoding/BOM/lossy/language/image/fingerprint metadata in one immutable shared structure. Preserve decode metadata when materializing draft UTF-8; do not call `Document::decode` again on draft bytes. The buffer's save baseline separately holds `Arc<TextSnapshot>`, `lenz_content::write::Snapshot` and confirmed canonical target. A confirmed external rename updates path lookup and metadata without changing BufferId; it invalidates old path-dependent work. Hard-linked distinct paths retain current Reader identity rules; do not silently unify their independent buffers by inode.

Use the measured approximately 4 KiB persistent chunks with newline indexes, adding per-chunk UTF-16 counts. Validate UTF-8 edit boundaries and reject UTF-16 positions inside surrogate pairs. Copy at most the requested window when a read crosses chunks. Root-vector copies and linear chunk lookup are known costs to measure, not a rope claim. No full snapshot is stored per undo step: retain inserted/removed text and view anchors. Dirty state compares current content with the confirmed baseline, so undo back to saved bytes becomes clean despite a newer version number. Use shared chunks/fingerprints as fast paths, with exact comparison before accepting equality. Save/cache pressure never clears history; explicit discard or final buffer close may. History memory may grow with retained changes; measure that growth separately from leaks, and do not introduce an unreviewed silent history cap.

## Task 1: Implement indexed text snapshots and transactions

**Files:** `text.rs`, `buffer.rs`, `document.rs`, `lib.rs`, `tests/editor_foundation.rs`.

**Interfaces:** Produces BufferId/SnapshotKey/TextSnapshot/TextEdit and buffer-local edit/undo/redo methods consumed by Tasks 2–7. Add private `Buffer::edit(expected, edits, kind, view, now) -> Result<SnapshotKey, EditError>`, `undo() -> Option<SnapshotKey>` and `redo() -> Option<SnapshotKey>`; `view` is TabId, `now` is Instant. EditError distinguishes read-only, stale version and invalid/overlapping ranges. Each operation is all-or-nothing and allocates a fresh monotonically increasing version, including undo/redo.

- [ ] Add `editor_text_unicode_transactions` and `editor_history_survives_save_and_cache`: use the readiness fixtures, assert exact bytes after multi-range insert/replace/delete and complete undo/redo; hold an old snapshot and assert unchanged text. Exercise all chunk boundaries, CRLF, empty text, final newline, combining marks and surrogate-pair rejection. Assert a stale/overlapping transaction leaves text/history/version unchanged.
- [ ] Run `cargo test --locked -p reader-core --test editor_foundation editor_text_` and the history test; expect missing interfaces/assertions to fail before implementation.
- [ ] Implement the measured chunk structure and compact inverse transactions. Coalesce adjacent typing/deletion only for the same view/direction within 500 ms and with no caret/selection movement, paste, Save, focus change or intervening command; paste/replacement is one transaction. Save ends a group without clearing history. Copy/cut materializes only its selected range. Read-only reasons include explicit mode, virtual/binary/lossy/unavailable source.
- [ ] Add `editor_text_long_line_windows`: at offsets before and after character 10,000 and near the end of the 900,001-byte Unicode fixture, assert window length ≤ requested budget, correct absolute source range, exact UTF-8/UTF-16 round trips and no full-document materialization. A too-small budget may return an empty window; callers request at least four bytes and must ensure traversal progresses.
- [ ] Run both named groups and targeted strict Clippy; expect all pass. Commit the storage/history change with no editable UI entry point yet.

## Task 2: Extract the writer and implement confirmed/uncertain Save

**Files:** content `write.rs`, `lib.rs`, `Cargo.toml`; app `write.rs`, `edit_session.rs`, `Cargo.toml`; core `buffer.rs`, `session.rs`, `tests/editor_foundation.rs`.

**Interfaces:** Move the existing `Snapshot`, `WriteError`, writer functions and their tests to `lenz-content::write`, making only cross-crate-used helpers public. App `write.rs` re-exports them so existing callers preserve behavior. Retain `write_text(&Snapshot, &str) -> Result<Snapshot, WriteError>` and dominant-ending semantics. Add `write_exact(snapshot: &Snapshot, expected_target: &Path, text: &str) -> WriteOutcome`, with `WriteOutcome::{Saved(Snapshot), Rejected(WriteError), Uncertain(WriteError)}`. Reader uses exact endings and the confirmed canonical target. Add `Completion::Saved { request: u64, key: SnapshotKey, outcome: WriteOutcome }`.

- [ ] Add `editor_save_preserves_encoding_and_endings` for UTF-8 BOM, declared UTF-16 with/without BOM, legacy encoding, mixed LF/CRLF, file mode and hard links. Assert untouched line endings are byte-identical. Re-run existing Diff normalization tests unchanged. Add canonical-target substitution and retargeted-symlink refusal tests.
- [ ] Add delayed Save and the readiness fault cases: pre-write rejection leaves disk unchanged; partial rollback may leave original or partial bytes; post-replacement error may leave complete new bytes. Assert uncertain outcomes keep current draft/history/confirmed baseline, block another implicit write and external launch, and report “Save outcome is uncertain; recheck the file before saving again.” A successful save of version N while current is N+1 advances only the baseline to N and leaves N+1 dirty.
- [ ] Run the new tests RED. Extract without changing Diff `EditSession` or merge semantics; retain caller-side canonical checks and repository guards. Add an exact encoder route that never normalizes untouched endings. Only failures known to precede mutation are Rejected; classify unproven I/O outcomes conservatively as Uncertain. Correct the old writer comments claiming every error leaves disk unchanged.
- [ ] Keep fault injection out of production with content `write-testing` enabled only by app/core dev-dependencies. Move writer fault tests with the writer; retain EditSession's uncertain-save regression through that dev feature. Verify production dependency/build features exclude it.
- [ ] Save runs off the input thread over a captured snapshot; serialize writes per buffer and retain its ownership until completion. Reject obsolete requests/context and never mark a newer draft clean. An uncertain outcome requests a read-only disk recheck; neither recheck nor retry silently accepts a new baseline. Explicit Reload/Discard is available through Task 4; full reconciliation UI remains later work.
- [ ] Run `cargo test --locked -p lenz-content write::`, `cargo test --locked -p lenz write::`, `cargo test --locked -p lenz edit_session::`, and core `editor_save_` tests; expect all pass. Commit.

## Task 3: Make sessions and both renderers read the current buffer

**Files:** core `session.rs`, `tabs.rs`, `cache.rs`, `analysis.rs`, `shown.rs`, `wrap.rs`, `places.rs`, `expand.rs`, worker consumers; GUI `lib.rs`; TUI `app.rs`, `render.rs`.

**Interfaces:** Produces `State::view`/View and admitted AnalysisInput/AnalysisRequest. Replace `Tab.doc` ownership with `Option<BufferId>`; migrate render/coordinate helpers from Tab to View. Add `Document::from_snapshot_parts(source: Arc<SourceMeta>, version: u64, text: Arc<str>) -> Self` internally to retain shared metadata with materialized text. All derived completion variants carry AnalysisRequest plus their existing operation-specific identity; search results also carry Task 5's captured-target identity.

- [ ] Add `editor_shared_views_and_independent_sessions`: split one file, edit via a core command in each view, assert equal BufferId/current version with independent selections/scroll; another Session remains unchanged. Edit a preview, open another source and assert the edited tab is retained. Evict actual clean inactive-cache entries and assert draft/baseline/history survive.
- [ ] Add `editor_analysis_admission_and_stale_publication`: hold one in-flight version, submit 100 edits and assert only one replaceable pending snapshot per buffer; deliver obsolete parse/find/Markdown/LSP products and assert they change neither current text nor current derived state. Close/rename/reopen a source and reject the previous context's reply.
- [ ] Run these tests RED, then implement the registry and view facade. Cache eviction may release reloadable analysis, never a live buffer's current/baseline/history. Remove old duplicate-current-document publication paths. Preserve empty/loading/image/virtual-source constructors and their disabled reasons.
- [ ] Admit materialization on the existing bounded worker system: at most one running version and one replaceable pending version per buffer, a bounded global ready queue of buffer IDs, and existing worker concurrency. Queue entries never retain additional full text. One materialized Arc<Document> per admitted version is shared across syntax/find/Markdown/index/LSP consumers; obsolete work checks cancellation before materializing, during cancellable work and before publishing. Track current key + operation request + context/provider generation. Derived products never replace Buffer.current.
- [ ] Implement bounded line windows and absolute-source mappings in Shown/hit-testing. Reuse layout checkpoints for horizontal/soft-wrap navigation so a far caret does not rescan the full prefix on every frame. Never call the existing full-line `display_line` on the whole long line: it counts the hidden tail. Clip spans/find/selection to window ranges and handle combining/control characters at window boundaries. Move word/caret/selection/goto/availability reads to chunk access; copying text is an explicit action, not a paint-time getter.
- [ ] Test stale hit-test/layout keys, 1×1 empty state, image-to-text transitions, End then Up and long-line movement in both adapters. Run core tests, `cargo test --locked -p reader-gui --features test-support`, and `cargo test --locked -p reader-tui --features gui-test`; expect all pass while typing remains disabled. Commit.

## Task 4: Guard every destructive transition and external handoff

**Files:** core `transition.rs`, `close.rs`, `session.rs`; GUI `editing.rs`, `lib.rs`, `tests.rs`; TUI `app.rs`, `runtime.rs`, input/render/parity tests.

**Interfaces:** `TransitionChoice::{Save, Discard, Cancel, SaveAndOpen, OpenSavedVersion}`. `PendingTransition { id, affected: Vec<BufferId>, action }` is private; expose a read-only prompt with centrally defined labels/reasons. Existing Reload/OpenRoot/checkout/close/external-open commands enter it before doing work. Add `CloseBatch::new(ticket: u64, sessions: Vec<u64>)`, `ready(session: u64) -> bool`, `cancel()` in core; it coordinates independent sessions without sharing their text. `Effect` adds close-ready/close-permitted ticket fields; adapter window/process removal requires the matching permit.

- [ ] Add table-driven `editor_dirty_transition_guard`: Reload, Open Folder, Open Worktree Here, checkout, new-branch-with-checkout, last tab close, window/view close and all-view quit. For each, assert no tab/root/Git worker/disk mutation before a choice; Cancel changes nothing; failed or uncertain Save prevents continuation; confirmed Save resumes exactly once. Closing only one split/comparison view keeps the shared buffer without prompting; closing its last owner asks.
- [ ] Add `editor_refresh_retains_dirty` for watcher, focus and index refresh, changed/deleted files and current (not merely stale) refresh replies. Assert current draft/history remain and external-conflict state is visible. Clean refresh retains existing behavior. Unsplitting retains drafts from removed panes as ordinary tabs when necessary.
- [ ] Run RED, then implement guards before any destructive dispatch. Save handles affected buffers in stable order, with existing per-file partial success: a later failure cancels the pending action but retains earlier confirmed saves and every remaining draft. Discard is deferred until the entire destructive action is authorized. Cancel before Save leaves disk unchanged; Cancel after an explicitly completed Save does not roll back that save. Editing is blocked only in the active decision modal; stale modal IDs cannot resume actions.
- [ ] Implement prepare/commit/cancel close across all independently open sessions. No session is removed or discarded until all have agreed. Cancel in any session cancels the batch. Track writes already explicitly confirmed separately; do not claim an all-files save transaction. A new edit invalidates a prepared permit. Route native GUI `Window::on_window_should_close` and terminal runtime Ctrl-Q/Ctrl-Shift-Q through this protocol. GUI menu/keyboard Quit must prepare before calling `cx.quit`; `on_app_quit` is cleanup only and cannot veto in pinned GPUI. Forced process termination/OS shutdown remains outside this milestone's loss-prevention guarantee; durable recovery belongs to lifecycle.
- [ ] Add `editor_dirty_external_open`: Save and Open launches exactly once after confirmed success; Open Saved Version preserves draft/history and omits precise position unless mapped to confirmed saved text; Cancel/failure/uncertainty launches nothing. Apply the same core gate to editor and default-app handoffs. Existing external comparisons explicitly label inputs as saved disk versions.
- [ ] Exercise actual native-close callbacks, two-window/two-terminal-view batches, every modal input route, tiny dialogs, empty windows, cancel after scrolling, and underlying bytes unchanged. Run core guard tests and both actual adapter suites; expect all pass. Commit with typing still disabled until Tasks 5–6 preserve existing workflows.

## Task 5: Make search and semantic navigation consistently draft-aware

**Files:** core `search.rs`, `index.rs`, `navigation.rs`, `find.rs`, `places.rs`, `semantics/mod.rs`, `semantics/lsp.rs`, `session.rs`; existing core and adapter navigation tests.

**Interfaces:** Add `SourceVersion::{Draft(SnapshotKey), Disk { stamp: Stamp, fingerprint: [u8; 32] }, Virtual { uri: PathBuf, version: u64 }}` and carry it per result target/preview, independently of the originating AnalysisRequest. Project requests capture `DraftSet { generation: u64, snapshots: Arc<HashMap<PathBuf, Arc<TextSnapshot>>> }` and project generation. Open-file overlays replace disk records; never concatenate stale disk matches for an overlaid path. Materialization remains worker-only.

- [ ] Add `editor_navigation_uses_drafts`: saved token removed, draft-only token/declaration added and lines inserted before an existing symbol. Assert project text/symbol results, previews, single definition, multiple definitions, usages and peek select the expected draft bytes; disk remains original. Include requests from A to a different dirty B and edits to B between answer and activation.
- [ ] Add `editor_navigation_revalidates_targets`: stale results cannot reopen disk text over a draft or silently clamp obsolete coordinates. Recompute the affected query/target against current text; if identity cannot be established, show a stale-result notice and leave selection unchanged. Preview is labelled/held for the result snapshot; activation validates the current target key separately. Cancelling or changing root rejects the old manifest.
- [ ] Run RED, then overlay current open-buffer snapshots in search/index/semantic target resolution. Invalidate the draft-set generation on edit/open/close/path changes; share derived snapshots rather than flattening in result activation. Synchronize each provider to the admitted version before requests; preserve original provider-session/position-encoding guards. Unknown/stale provider target versions remain unavailable until re-requested.
- [ ] Transform selections/bookmarks/history anchors through committed transactions with explicit left/right bias; collapsed folds/expand chains/find matches remain version-tagged and recompute when needed. Deleted anchors clamp to the edit start and are labelled approximate when exact relocation is lost. Do not persist stale disk line numbers as new authoritative positions.
- [ ] Run the new tests, existing semantics/navigation suites and GUI/TUI result input/render tests. Assert exact selected text and unchanged file bytes in common adapter replay. Commit.

## Task 6: Preserve blame and add the shared last-saved comparison

**Files:** core `blame.rs`, new `comparison.rs`, `session.rs`, `Cargo.toml`; GUI `lib.rs`, `editing.rs`; TUI `app.rs`, `render.rs`; core/adapter tests.

**Interfaces:** Add `SavedComparison { baseline: Arc<TextSnapshot>, current: BufferId, request: AnalysisRequest }` and a derived engine result tagged with both baseline/current keys. Use `lenz-engine` as a normal Reader-core dependency; never depend on the GUI/Diff app. Add `Completion::Compared` carrying those identities. Comparison left is read-only; right resolves the current BufferId and uses the same Edit/Undo/Redo/Save commands. Refresh the comparison baseline after confirmed Save; discard obsolete comparison output.

- [ ] Add `editor_saved_comparison_shares_history`: edit in a tab, compare to saved, edit in the comparison, undo from the tab and redo from the comparison. Assert one BufferId/history, immutable prior bytes until Save and unchanged disk until explicit Save. Closing comparison retains positions and the current draft.
- [ ] Add `editor_blame_tracks_draft`: insert/delete/undo around committed lines with blame enabled. Immediately invalidate old attribution and disable commit actions; after recompute, changed lines are Not committed and unchanged lines retain their correct commits. Reject stale batches and show pending/failure status instead of old line attribution.
- [ ] Run RED, then use the existing pure diff engine off-thread for minimal side-by-side last-save comparison. Preserve each side's view selection/scroll. No hunk revert, Git markers or HEAD comparison is claimed in this slice.
- [ ] For draft blame, reuse `lenz_content::blame::run_blame` with `--contents -`, the admitted current bytes and existing ignore-revs handling; tag request and snapshot key, preserve cancellation/child cleanup. Untracked files are Not committed without reading stale disk bytes. Do not introduce a second Git subprocess manager.
- [ ] Expose Compare with Last Saved, return-to-source and accurate attribution state in both adapters and central menus. Run core/adapter tests and existing Diff comparison regressions; expect pass. Commit.

## Task 7: Enable editing through actual GUI and terminal input

**Files:** GUI `editing.rs`, `lib.rs`, `tests.rs`; TUI `app.rs`, `render.rs`, `runtime.rs`, input/render/parity tests; central session catalog/commands and core tests.

**Interfaces:** All text mutations dispatch Task 1's versioned Edit command. GUI implements GPUI's current EntityInputHandler contract using bounded surrounding text and exact UTF-16 conversion. Composition presentation remains adapter-owned until commit; committed text becomes one core transaction. Core availability and menu labels are authoritative; no adapter mutates a Buffer.

- [ ] Add `gui_editor_foundation_trace`, `tui_editor_foundation_trace`, and common `editor_foundation_parity`. Open `alpha\nbeta\n`; replace `beta` with `β😀`, type/paste multiline text, split, edit in each view, undo/redo, Save, open enough files for actual cache eviction, return and undo. Assert identical current/saved text, dirty state, history behavior and file bytes across adapters; another Session stays independent. Use real constructors/event mappings.
- [ ] Run these tests RED. Enable direct typing, selection replacement, Backspace/Delete, cut/copy/paste and Enter only for writable text focus. Printable q/j/k/`/`/Space insert in TUI; route reading actions through explicit commands. Ctrl/Cmd-S saves, Ctrl/Cmd-Z undoes, Ctrl/Cmd-Shift-Z redoes. Keep Reload guarded. Later keymap work owns conflicting Ctrl-R/Ctrl-H/F2 presets; all semantic actions remain in both menus now.
- [ ] Implement GUI IME preedit/cancel/commit, including replacement ranges, non-BMP and combining characters. Key committed ranges to the snapshot used for conversion; invalidate composition on conflicting edits/focus change instead of applying stale ranges. Return bounded surrounding context with its actual UTF-16 range; do not flatten the document for an IME query. Terminal bracketed paste is one transaction and never interprets pasted control sequences as editor commands.
- [ ] Render dirty/read-only/save-busy/conflict/uncertain state separately from Git status. Input reaches the active modal before document, toolbar or save actions; key/mouse/accessibility actions share the same gate. Preserve supported explicit global view-switch commands without allowing a close bypass. Check light/dark and non-color presentation.
- [ ] Add actual adapter tests for long-line typing/hit-testing/IME beyond character 10,000; narrow 1×1 terminal and 320-pixel window; empty Undo/Redo/Reload; stale selection after switching; composition cancellation; source-to-comparison transitions; native close and every external-open choice. Assert zero input/render materializations via test-only counters and matching disk output.
- [ ] Run the full relevant core/GUI/TUI suites; expect pass. Commit the first reachable editor slice only after all preceding protections are green.

## Task 8: Verify performance, parity and the internal milestone

**Files:** existing benchmark collectors only when new workload support is necessary; their Python tests; `docs/research/editor-foundation-performance.md`, Reader performance docs, README and optimization notebook if optimization changes are made; `docs/code/tui.md`, `docs/tui.md`.

**Interfaces:** Consumes the immutable readiness binaries, fixture hashes and raw samples. Produces an honest before/after report and supported coverage, not a new API.

- [ ] Run `scripts/ci-local.sh`, including workspace/core tests, serial memory tests, `cargo test -p lenz --features gui-test gui_`, actual Reader GUI and TUI/parity suites and collector tests. Every gate must pass; compilation alone is insufficient. Re-run writer Diff/merge tests after extraction.
- [ ] Repeat readiness warm/fresh GUI launches (20 measured + three warmups), ten kept tabs, and standalone TUI startup/navigation stress with the same machine/configuration/fixtures and separate release builds. Keep test/probe instrumentation out of ordinary binaries. Retain raw samples, executable/source identity and skipped workloads. Investigate repeated regressions; no deliberate tradeoff is pre-approved.
- [ ] Add release editing workloads for 2,000-line Rust, 100,000-line text and the 900,001-byte Unicode line: beginning/middle/end insertions, distant alternating edits, paste, selection replacement, split/comparison edits, Save/undo/redo, active analysis and rapid stale-result cancellation. Run each sustained candidate at least 180 seconds, including both adapters with disposable fixtures/private display. Distinguish first feedback, completed edit and physical display limitations. Capture parent/child RSS and peak/settled samples.
- [ ] Test sustained history growth separately from repeated undo-to-original cycles; account retained changed-text bytes and transaction overhead. Confirm released snapshots/work queues do not grow with obsolete versions. Measure added UTF-16 indexes and long-line layout checkpoints; the readiness prototype did not measure these. Assert zero input/render flattening, bounded read windows, at most one running/one pending version per buffer, exact text/undo correctness and fixture bytes after Save.
- [ ] Publish only measured editing coverage. The readiness fixture range is a minimum acceptance workload, not proof of unlimited file size. Existing large-file analysis reductions must remain visible and must not silently disable ordinary edits in the tested range. Report native macOS and hardware rendering as unverified until actually exercised, including native close/IME behavior.
- [ ] Verify local documentation links and `git diff --check`, update capabilities/gaps and performance evidence, then commit. Run an independent whole-change review. Foundation completion means this internal slice passes; it does not mean the accepted editor release is complete.

## Remaining release requirements and dependencies

| Subsequent plan | Requirements retained from the design | Dependencies / unresolved evidence |
| --- | --- | --- |
| Complete editing | Multi-cursor, indentation/pairs/comments, line move/duplicate, snippets/placeholders, in-file replace, IntelliJ default/VS Code presets/overrides and conflict resolution | Foundation transactions, input focus and shared catalog; verify terminal shortcut limits |
| Document lifecycle | New/Save As, private durable recovery, restored tabs/splits/positions/drafts, explicit discard tombstones, external-change reconciliation, explorer create/rename/move/recoverable delete and confirmed permanent delete | Foundation guards are already required; reconcile buffer/path/provider/watch identities; no semantic rename claim |
| Language assistance | Automatic/manual completion toggle, docs/signatures, Tab/Enter acceptance, Escape, no punctuation acceptance, imports, diagnostics/quick fixes/formatting/organize imports, explicit format-on-save option, EditorConfig/project settings, single-file atomic edits and rejected cross-file/server edits | Validate actual Java/Kotlin mixed projects, Rust, Go, Ruby/Rails, TS/JS/TSX, HTML/embedded languages, Python, YAML, Bash/sh and `.env`; provider installation remains deferred |
| Review and Git | Blue/green/deletion gutters, HEAD vs saved baseline labels, Git/unsaved/recovered file badges, new/untracked/conflicted cases, marker Show Diff/revert, unchanged index, editable HEAD/external-change comparisons and refresh | Minimal saved comparison/ownership comes from foundation; Git baseline changes require independent identity and performance proof |
| Templates and Markdown | First-class ERB/Haml Ruby and markup help, `.html.erb`/`.html.haml`, interpolation/indentation; Markdown live preview/list continuation/heading and file-link completion/navigation | Explicit projections prove a primitive only. Native ERB range defect, encoding-dependent signature help, missing Rails tools, automatic template parsing, Haml edit syntax and formatting remain blockers |
| SQL | Visible persisted project dialect/default and file override/unset, PostgreSQL and DuckDB offline query-local completion, function help, diagnostics, formatting, explicit optional private read-only metadata, refresh/failure state | DuckDB alias/CTE/nested/quoted/empty-qualified completions fail; PostgreSQL language provider absent; dialect-switch cancellation unverified. Never execute edited SQL/EXPLAIN; no result grid |

AI completion, run/test/debug tools, project-wide replace/refactors, Git stage/commit/push, tool installation and SQL execution remain explicitly deferred as in the accepted design. Missing assistance must not block ordinary editing, and an incomplete required language/template/SQL capability still blocks the full release.

## Plan review record

Source inspected at readiness commit `8cdc635`, including contiguous Document consumers, Tab ownership, refresh/search/index paths, writer fallbacks, runtime quit interception and pinned GPUI `fb69067` close/shutdown APIs. A fresh advisor recommended the worker-Document/current-Snapshot separation; long-line tail scanning and the non-vetoable app-quit callback were checked in source. These are architectural decisions, not implemented or benchmarked behavior.

Self-review: every accepted requirement maps to a foundation task or the later-release table; the five review-focus cases have named tests; shared type names and producer/consumer task dependencies agree. Tasks 1–6 establish protections and current-text behavior before Task 7 enables typing. Provider blockers remain explicit. Production implementation requires a separate handoff after this plan is reviewed.
