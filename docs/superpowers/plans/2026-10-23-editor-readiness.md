# Editor Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish measured document-ownership and provider feasibility evidence, then produce the concrete foundation plan for the integrated editor.

**Architecture:** Put session-owned editable buffers in `reader-core`, with stable buffer identity independent of paths. Reuse narrowly shared file-writing primitives through `lenz-content`, preserving the existing Diff lifecycle and keeping adapter dependencies separate. Select text storage only after measuring viewport access, snapshots and downstream analysis as well as edits.

**Tech Stack:** Rust workspace, GPUI, ratatui/crossterm, existing LSP transport and provider launch conventions, existing Python benchmark collectors, disposable local fixtures.

**Spec:** [Integrated editor design](../specs/2026-10-23-integrated-editor-design.md).

**Kickoff:** Review the existing text plan against the agreed design; the user decides new consequential behavior. This plan controls readiness work; the linked spec controls product scope. Review findings were presented before these revisions.

**Readiness:** Tasks 1–2 are complete; see the [execution evidence](../../research/editor-readiness.md). Template/SQL probes and the foundation plan remain in progress. No production editor behavior is implemented or validated; template and SQL feasibility remain full-release requirements.

## Scope and sequence

This is the next-phase plan, not an assertion that the whole editor is ready to implement. It produces reproducible experiments, an engineering decision report and a fully specified first implementation slice. The accepted feature scope is unchanged. A failed feasibility experiment remains a required feature to resolve, not permission to quietly downgrade it.

The design spans several subsystems. Write separate implementation plans in this order, allowing later independent work only after the shared interfaces are established:

| Sub-plan | Working deliverable | Main ownership |
| --- | --- | --- |
| 1. Foundation | Existing-file editing, undo/redo, dirty state, safe Save, shared split buffers, a minimal editable comparison, protection on every destructive transition, and preservation of existing navigation and external handoffs in both adapters | Reader document/session/tab/cache/search/index/watch state, shared writer, GUI/TUI input, rendering and close/quit lifecycle |
| 2. Complete editing | Multi-cursor operations, snippets, find/replace, indentation/pairs/comments/line operations, IntelliJ/VS Code presets and overrides | Reader command/action catalog and both input adapters |
| 3. Document lifecycle | New files/Save As, durable recovery, session restoration, full external-change reconciliation and explorer file operations; extend the transition guards already delivered in foundation | Reader workspace/watch/jobs state, file operations, modal adapters |
| 4. Language assistance | Completion/imports/signatures, diagnostics/quick fixes/formatting, project preferences, provider status and requested standalone-language coverage | Reader semantic protocol/providers and editor transactions |
| 5. Review and Git | Full editable comparisons, HEAD/last-save baselines, change gutter, file-status decoration and undoable hunk reversion | Shared diff/Git access, Reader state and both renderers |
| 6. Templates and Markdown | ERB/Haml Ruby and markup assistance, embedded-language integration, Markdown source/preview editing and link assistance | Content languages, Reader semantics/Markdown and adapter presentation |
| 7. SQL | PostgreSQL/DuckDB dialect selection, query-local assistance and optional metadata-only connections | Reader SQL provider/configuration/catalog support and adapter controls |

Recovery, rich assistance and Git decorations are not prerequisites for the first internal foundation milestone. They remain prerequisites for the full agreed release. Templates and SQL feasibility are investigated now because their outcomes can materially change later plans.

## Review decisions and evidence

Inspected source: `/home/alex/.codex/worktrees/cf8f/lenz`, clean revision `abf3d61334a68d3c058630a6e986fd33506aeccf`, before this documentation revision. The review traced shared core behavior, both adapters, existing tests and writer fallback paths. No runtime tests, provider probes or benchmarks were executed. The consequences below are source-supported inferences about adding drafts, not reproduced defects in an editor that already exists.

### Protect drafts from the first editable milestone

The original sequence deferred project/checkout guards to lifecycle. Inspected source shows that [Reload starts a disk load](../../../crates/reader-core/src/session.rs#L2803), [refresh replaces changed snapshots](../../../crates/reader-core/src/session.rs#L5250), [project replacement clears tabs](../../../crates/reader-core/src/session.rs#L5404), and [checkout starts a Git worker](../../../crates/reader-core/src/session.rs#L2167). Request/generation checks protect against obsolete replies; they cannot protect a dirty draft from a current refresh. Git also cannot detect a draft over a clean disk file. The accepted no-loss behavior therefore requires guards in foundation, before any of those operations can discard content or mutate the working tree.

Native close is a separate entry point: the [GUI quit callback](../../../crates/reader-gui/src/lib.rs#L498) flushes workspace metadata, while the [terminal runtime](../../../crates/reader-tui/src/runtime.rs#L197) handles view close and all-view quit before ordinary input handling. A tab-close test alone cannot establish exit protection. Foundation must cover all these paths through shared core decisions. Full reconciliation and durable recovery remain in lifecycle. Authority: the spec's accepted [Save/Discard/Cancel and draft-preservation requirements](../specs/2026-10-23-integrated-editor-design.md#saving-recovery-and-external-changes); this corrects sequencing without adding scope. Acceptance is the expanded Task 5 transition trace.

### Use drafts consistently in existing navigation

Inspected source: [project search reads disk](../../../crates/reader-core/src/search.rs#L278), [the declaration index reads disk](../../../crates/reader-core/src/index.rs#L279), and [project-result activation validates disk](../../../crates/reader-core/src/session.rs#L5152), even when a preview uses a held document. Existing semantic replies have [request/version guards](../../../crates/reader-core/src/session.rs#L3678), but usages/peek enter those project result lists. [Location restoration](../../../crates/reader-core/src/tabs.rs#L565) can relocate a matching line or use an approximate position; it does not supply draft-only search hits or validate every target version.

Agreed during this review: use unsaved drafts for open files and disk for the rest. Carry snapshot identity through results, preview and activation; invalidate or recompute stale targets. Preserve existing navigation in foundation rather than waiting for new completion support. The same draft must yield the same target through single-definition and multi-result/peek routes. Acceptance: Task 5 covers draft-only tokens/declarations, removed disk matches, shifted lines and another edit between response and activation. Disk bytes stay unchanged.

### Preserve attribution and make external handoffs explicit

Inspected source: [blame is keyed to a disk stamp](../../../crates/reader-core/src/session.rs#L6408), while [commit actions select by the displayed caret line](../../../crates/reader-core/src/session.rs#L2238). After draft line changes, invalidate obsolete attribution before allowing commit actions. This is a preservation requirement; the implementation can choose how to recompute it. Foundation acceptance includes insertion, deletion and undo with blame enabled, and pending-state behavior.

The [external-open plan](../../../crates/reader-core/src/session.rs#L2284) supplies a real path with positions from the held document; [external comparisons](../../../crates/reader-core/src/session.rs#L2318) supply disk paths. Agreed during this review: a dirty external open offers Save and Open / Open Saved Version / Cancel. Save failure or uncertainty prevents the launch. Opening the saved version preserves the draft and uses saved coordinates, or omits the position if unknown. Foundation owns this transition and labels remaining external comparison inputs as saved disk versions; its internal last-save comparison shares the draft. Acceptance covers all three choices in both adapters, including failed Save.

### Report save outcomes that the writer can establish

Inspected source: [replacement can fall back to in-place writing](../../../crates/app/src/write.rs#L387); [rollback is best effort](../../../crates/app/src/write.rs#L435); [metadata collection can fail after replacement](../../../crates/app/src/write.rs#L196). The original unconditional `disk_after == disk_before` assertion for every failed save was stronger than this behavior. Refusing in-place writes would also change existing compatibility and would not eliminate post-replacement errors.

Preserve existing per-file saveability and Diff semantics. Proposed engineering contract, implementing the accepted draft-retention requirement: confirmed success advances the save baseline and retains undo; pre-write rejection leaves the target untouched; uncertain I/O failure retains draft/history and the last confirmed baseline, reports uncertainty, and requires disk revalidation before reconciliation or retry. Do not silently retry using a fresh baseline. Do not claim universal atomicity, an all-files transaction or crash durability. Task 2 characterizes the boundaries; Task 5 checks what the user sees and can do next. A deliberate compatibility change or stronger guarantee would require a new user decision.

### Review readiness and feedback

Agreed product scope, languages, completion interaction, change markers, Haml and PostgreSQL/DuckDB requirements remain intact. This review settled the two new user choices above. Storage, source mappings and provider feasibility remain unknown until Tasks 1–4; a missing capability blocks its delivery slice and the full release, not unrelated investigation. The foundation plan must include the guard/navigation/save outcomes above before it can be considered ready for implementation.

Record later discoveries here as expected → discovered behavior, with source/run evidence, affected decision and delivery slice. Reopen only changed consequential choices; retain failed or inconclusive experiments in the execution report. Do not replace this review's source inferences with claims of runtime validation.

## Global Constraints

- "The first release includes ordinary editing, language assistance, file management, recovery, change markers and editable comparisons in both GUI and TUI." — [Agreed release boundary](../specs/2026-10-23-integrated-editor-design.md#intent-and-release-boundary).
- "Independently opened windows/sessions retain independent documents and drafts." — [Agreed ownership](../specs/2026-10-23-integrated-editor-design.md#editing-and-document-ownership).
- "Save is explicit by default." — [Agreed saving behavior](../specs/2026-10-23-integrated-editor-design.md#saving-recovery-and-external-changes).
- "Start providers as needed and preserve cancellation and resource bounds." — [Agreed language support](../specs/2026-10-23-integrated-editor-design.md#language-and-file-support).
- "Do not execute the user's SQL, run EXPLAIN or show result grids." — [Agreed SQL boundary](../specs/2026-10-23-integrated-editor-design.md#postgresql-and-duckdb).
- "Preserve startup, responsiveness and memory against equivalent measured release baselines; any deliberate performance tradeoff needs explicit agreement." — [Agreed validation](../specs/2026-10-23-integrated-editor-design.md#engineering-choices-and-validation).
- Keep experiments out of production executables and dependencies. Use installed tools and disposable projects/databases, isolated settings and owned displays/PTYs. Do not use the user's repositories, databases or active desktop for experiments.
- Required evidence: preserve raw samples, source/executable/provider identity and actual duration. Missing tools or platforms are not verified, never passing results. Technical limitations must be reconciled with the spec before implementation claims.

## Review Focus

1. A saved buffer can be clean yet still own undo history; cache eviction must not discard that history. Exercise in Task 2 and the foundation acceptance trace in Task 5.
2. An efficient insertion implementation can still flatten an entire document for rendering, parsing or LSP on each keystroke. Measure those paths explicitly in Task 2.
3. Writer safety includes caller-side identity checks and mixed line-ending behavior, not just `write_text`. Characterize in Task 2 before specifying extraction.
4. Template offsets can refer to generated text rather than the visible UTF-8/UTF-16 source; Unicode and multiline edits must map back exactly. Exercise in Task 3.
5. SQL tooling can silently rely on EXPLAIN or execute input while obtaining schema/type information. Verify the actual database boundary and offline behavior in Task 4.

## Files and artifacts

Planning maintains this authoritative readiness plan and the accepted product spec. Execution owns the following narrowly scoped additions:

- Create `crates/reader-core/tests/editor_storage_probe.rs`: test-only storage/ownership experiments; no production buffer API is declared by this probe.
- Create `crates/reader-core/tests/editor_provider_probe.rs`: direct protocol/provider feasibility checks for templates and SQL, reusing public `reader_core::semantics::lsp::{read_message, write_message}` and existing provider-launch conventions.
- Modify `crates/app/src/write.rs` and `crates/app/src/edit_session.rs` only to add characterization tests if existing tests cannot establish the required writer behavior. Keep current production behavior intact during this phase.
- Create `docs/research/editor-readiness.md`: actual findings, evidence paths, chosen direction, failed/inconclusive cases and remaining blockers.
- Create `docs/superpowers/plans/2026-10-23-editor-foundation.md`: concrete foundation implementation tasks after evidence is available.
- Retain generated fixtures, immutable executable copies, transcripts, hashes and raw samples under a new `target/perf/editor-readiness/` directory. Use a fresh run directory if it already exists.

The provider probe is independent of production completion support, which does not exist yet. Label direct-provider results separately from end-to-end Lenz acceptance. If experiment-only dependencies are essential, isolate the probe in a standalone test project under the evidence directory rather than expanding production dependencies.

## Task 1: Capture a reproducible baseline

**Files:** Read `docs/benchmarking.md`, `docs/optimization-notebook.md`, `docs/code/performance.md`, `docs/code/tui-performance.md`, `scripts/ci-local.sh`, `scripts/code-launch-bench.py`, `scripts/code-tui-bench.py`. Write evidence under `target/perf/editor-readiness/before/` and the baseline section of the report.

**Interfaces:** Consumes the current source revision and existing collectors. Produces immutable GUI probe/production and standalone TUI binaries, fixture hashes, source/toolchain identity, raw collector outputs and exact commands/settings sufficient for paired reruns.

- [x] Run `scripts/ci-local.sh`, retaining its output and exit status. Diagnose any existing failure separately; record the baseline state before changing runtime behavior.
- [x] Generate ten disposable Rust source files with 500 four-line functions each, plus plain-text fixtures of 100,000 lines and one very long Unicode line. Record byte hashes. Build the GUI with `cargo build --release --locked -p reader-gui --features perf-probe`, copy its executable into the evidence directory, then build the ordinary GUI with `cargo build --release --locked -p reader-gui`. Build the TUI separately with `cargo build --release --locked -p reader-tui`, as required by the benchmark guidance, and retain immutable copies of each executable.
- [x] Run `scripts/code-launch-bench.py` with the saved probe binary, generated Rust file/corpus, 20 measured trials, a reserved unused private X display and fixed geometry. Retain both warm/fresh outputs and child-process memory separately. Follow the documented restricted-PATH procedure for server-free samples; never claim installed-server runs are equivalent.
- [x] Run `python3 scripts/code-tui-bench.py --binary <saved-tui-binary> --out <new-output-directory> --lines 100000 --rounds 30 --stress-seconds 60`. Require its correctness assertions and terminal restoration to pass. Record GUI/PTY endpoint limitations and actual stress duration.
- [x] Verify recorded fixture hashes, binary hashes, source identity and complete raw outputs; confirm fixtures are unchanged and commit the baseline report section, not generated binaries or samples. Existing collector ceilings do not replace paired baseline comparisons.

## Task 2: Prove buffer ownership and select storage

**Files:** Create `crates/reader-core/tests/editor_storage_probe.rs`; read `crates/reader-core/src/{document,tabs,cache,session}.rs`, `crates/content/src/highlight.rs`, `crates/reader-core/src/semantics/mod.rs`, and the two adapters' visible-line consumers. Add writer characterization tests only where needed. Update the report.

**Interfaces:** Consumes Task 1 identities/fixtures. Produces a selected storage approach with measured tradeoffs, a buffer/view ownership contract, a worker-snapshot policy and the exact writer extraction contract. The probe's private types are not promises to downstream production code.

- [x] Add deterministic tests named `shared_views_keep_one_draft`, `saved_history_survives_cache_pressure`, `old_worker_snapshot_cannot_replace_new_text` and `unicode_multirange_undo_round_trips`. Before implementing the experimental owner, require assertions equivalent to:

  ```rust
  assert_eq!(left_buffer_id, right_buffer_id);
  assert_ne!(left_selection, right_selection);
  assert_eq!(undo_after_save_and_eviction, original_text);
  assert_eq!(independent_session_text, original_text);
  assert_eq!(current_version_after_stale_reply, newest_version);
  assert_eq!(text_after_undo_all, original_text);
  assert_eq!(text_after_redo_all, edited_text);
  ```

- [x] In test-only code, compare a simple gap-buffer candidate with one chunked/persistent candidate. Use a stable `BufferId` independent of the canonical-path lookup: new documents, Save As and rename must not change draft identity. Keep per-view positions outside the buffer, and keep draft/history ownership outside the reloadable cache. Run `cargo test --locked -p reader-core --test editor_storage_probe` until the deterministic assertions pass.
- [x] Add an ignored `editor_storage_measurements` test. Run in release with `cargo test --release --locked -p reader-core --test editor_storage_probe editor_storage_measurements -- --ignored --nocapture --test-threads=1`. Measure beginning/middle/end insertion, distant alternating edits, multiline paste, multi-range operations, long Unicode lines, viewport line lookup, snapshot creation with a retained worker version, and edit/undo/save stress. Use the same fixed edit trace for both candidates, retain every sample and assert identical resulting text. Run sustained cycles for 180 seconds and record actual duration and settled/peak memory.
- [x] Include actual visible-line access and `lenz_content::highlight::parse_with`/LSP serialization handoff costs in the experiment. Count full-text materializations and bytes copied; distinguish per-edit work from explicitly coalesced background snapshots. A fast standalone insert benchmark does not establish an acceptable candidate. Keep all parse/LSP queues bounded and test cancellation of obsolete work.
- [x] Characterize mixed LF/CRLF, encoding/BOM round trips, hard links and a symlink retargeted to identical bytes. Inspected source: [encoding](../../../crates/app/src/write.rs#L201) normalizes lone LF when CRLF dominates; [EditSession::finish](../../../crates/app/src/edit_session.rs#L217) additionally checks the canonical target. Plan to preserve untouched Reader line endings and identity checks without changing Diff's existing semantics. Run the existing `identical_bytes_at_retargeted_symlink_are_not_a_safe_save` regression and any new tests.
- [x] Characterize the [save outcomes](#report-save-outcomes-that-the-writer-can-establish) per file. Assert unchanged target bytes for pre-write rejection and pre-replacement failure. Use narrow test-only fault injection for in-place failure with successful and failed rollback, and metadata failure after replacement; record actual disk effects. Every unconfirmed save retains draft/history and the last confirmed baseline. If a boundary cannot be exercised without a broad filesystem simulator, retain the source finding and mark runtime behavior unverified. Preserve existing Diff partial-success handling; no production writer rewrite belongs in this phase.
- [x] Record the chosen representation, unsuccessful candidate, full-text conversion policy, history retention/release policy and writer extraction requirements. Give unsupported sizes/paths explicit evidence rather than a guessed ceiling. Commit the probe/tests and report together.

## Task 3: Prove ERB and Haml assistance

**Files:** Create or extend `crates/reader-core/tests/editor_provider_probe.rs`; read the existing `tests/providers.rs` fixture/launch conventions and `src/semantics/lsp.rs`; retain generated templates and request/reply transcripts in the evidence directory. Update the report.

**Interfaces:** Consumes installed provider identities and temporary Ruby/Rails projects. Produces separate ERB and Haml capability results, a demonstrated source-offset mapping approach and a concrete integration route or a documented blocker for each required capability.

- [x] Add deterministic mapping checks covering `.html.erb` and `.html.haml`, multiline Ruby, interpolation, markup attributes, indentation, CRLF and a non-BMP character before the caret. Pin a completion replacement to the original source range: `assert_eq!(mapped_source_range, expected_source_range)`, and after applying it, `assert_eq!(unchanged_regions_after, unchanged_regions_before)`. Reject edits touching generated scaffolding or another file in full.
- [x] Add ignored `erb_provider_assistance` and `haml_provider_assistance` tests using framed LSP messages or the candidate provider's actual documented protocol. Capture initialize capabilities, versions and configuration. Exercise Ruby member/helper completion, signature help where available, markup tag/attribute assistance and a diagnostic mapped to the correct original location. Test a changed unsaved template and an obsolete response. Run `cargo test --release --locked -p reader-core --test editor_provider_probe provider_assistance -- --ignored --nocapture --test-threads=1`.
- [x] Verify the Ruby/Rails launch uses only disposable project/configuration state and installed dependencies. Record a missing server/gem as an unverified case, not a pass. Ruby LSP's ERB host-language delegation needs client work; do not infer a Haml implementation from ERB success. If no suitable Haml provider path exists, build only the smallest test-only projection prototype necessary to evaluate Ruby and markup assistance.
- [x] Record actual completion/diagnostic payloads, source mapping, readiness/memory, startup side effects and unsupported cases. Stop promising an implementation route that fails the required behavior; investigate an alternative or bring a concrete feasibility conflict to the user. Commit tests and findings without wiring an unproven provider into production.

## Task 4: Prove PostgreSQL and DuckDB assistance without statement execution

**Files:** Extend `crates/reader-core/tests/editor_provider_probe.rs`; retain disposable SQL schemas, query fixtures, provider/engine identity, metadata transcripts and the report.

**Interfaces:** Consumes an explicit dialect and optional disposable metadata source. Produces a verified feature matrix and provider/adapter choice for PostgreSQL and DuckDB, with evidence that the edited SQL is never submitted for execution or EXPLAIN. A candidate may be a language server or a narrow adapter over an installed tool; neither is assumed complete.

- [ ] Add offline fixtures for aliases, CTEs, nested subqueries, quoted identifiers, qualified names, incomplete statements and functions. Include PostgreSQL-specific syntax and DuckDB constructs such as `SELECT * EXCLUDE (...)` and `GROUP BY ALL`. Pin expected names/types or diagnostic source ranges, not merely a nonempty completion list. Include malformed intermediate input and cancellation after switching dialect.
- [ ] Add ignored `postgres_sql_assistance` and `duckdb_sql_assistance` tests. Run `cargo test --release --locked -p reader-core --test editor_provider_probe sql_assistance -- --ignored --nocapture --test-threads=1`. First test without any connection; require zero attempted database connections. Then seed a disposable PostgreSQL instance/DuckDB file with known tables, columns and functions and explicitly enable metadata access. Record exactly which schema-dependent features become available.
- [ ] Observe requests at the actual database boundary. Use edited-document fixtures containing `DROP TABLE`, an update and an export to a sentinel path. Require `assert_eq!(user_statements_submitted, 0)`, `assert_eq!(explain_statements_submitted, 0)`, `assert_eq!(fixture_data_after, fixture_data_before)` and `assert!(!sentinel_path.exists())`. Allow only identified metadata operations after explicit connection. Fixture setup creates its own schema before measurement; it is not editor execution. Redact credentials from retained logs.
- [ ] Evaluate the complete acceptance set separately for each dialect. DuckDB's autocomplete facility is a candidate, not evidence of alias/CTE support or editor readiness. Disable any PostgreSQL-provider feature that depends on EXPLAIN. Record dialect version, schema freshness/refresh behavior, formatting, function help, diagnostics, connection failures, latency and memory. Unsupported features remain unresolved requirements.
- [ ] Commit the tests and findings. If an engine-specific adapter is required, define its minimal request/result contract in the report using payloads observed in these probes; avoid a speculative database plugin framework.

## Task 5: Write the foundation implementation plan from the evidence

**Files:** Complete `docs/research/editor-readiness.md`; create `docs/superpowers/plans/2026-10-23-editor-foundation.md`. The future plan will own `reader-core` document/session/tab/cache state, a narrow writer extraction into `lenz-content`, both Reader adapters and their existing tests.

**Interfaces:** Consumes Tasks 1–4 reports. Produces exact production types/signatures, file ownership, command transitions and executable test steps for the foundation only, plus links to recorded provider decisions for later sub-plans. Unresolved required provider support is recorded as a full-release blocker even if independent foundation work can proceed.

- [ ] Specify session-owned buffers with stable identity, path lookup, version, save baseline and undo history; view-specific selection/scroll; versioned immutable worker inputs; and measured storage/history retention. Keep the existing two-sided Diff `EditSession` intact. Move only reusable file-safety behavior and preserve its caller-side guards. Define exact interfaces only now that the storage and save-contract experiments have answers.
- [ ] Make the first acceptance trace executable through both adapters: open an existing local text file, replace a selection/type/paste, split it, edit through either pane, undo/redo, Save, switch files under cache pressure, return and undo. Verify retained preview tabs, modal input isolation, stale-result rejection and an independent second session. Confirmed Save retains undo; uncertain Save retains the draft and baseline, explains disk uncertainty and does not trigger an external launch or silent retry.
- [ ] Specify and test dirty guards before enabling typing: Reload, watcher/focus/index refresh, Open Folder/Open Worktree Here, checkout/new-branch-with-checkout, tab close, native GUI window close, terminal view close and all-view quit. Automatic refresh retains dirty text and records an external-change conflict; it never replaces the draft. Explicit destructive actions use the agreed Save/Discard/Cancel behavior before clearing state or starting a Git worker. Cancel leaves drafts, project and working tree unchanged; failed Save prevents the action. Unsplitting removes a view without losing a shared draft. Full reconciliation UI may follow in lifecycle, but these protections cannot.
- [ ] Implement the agreed draft-aware navigation. Test a draft-only search token/declaration, removal of a saved match, inserted lines before a symbol, and another edit after results arrive. Verify result list, preview and activation use the same snapshot policy in both adapters; cover usages/peek/multiple definitions and a different dirty target. Revalidate or recompute obsolete targets, preserve selection/bookmark/fold/find correctness and reject obsolete worker results without changing disk bytes.
- [ ] Verify blame and commit actions after insertion, deletion and undo: stale attribution is unavailable while recomputing, and displayed attribution refers to current text. Exercise the agreed dirty external-open choices through both adapters: confirmed Save and Open launches once; Open Saved Version preserves the draft and uses saved coordinates or omits them; Cancel and failed/unconfirmed Save launch nothing. Label existing external comparison inputs as disk versions while the internal comparison below uses the draft.
- [ ] Include the agreed minimal compare-to-last-saved view in that slice: the previous snapshot is immutable and the current side references the same buffer and undo history. Test editing and undo through the comparison and returning to the ordinary tab. This proves ownership before later Git markers, hunk actions and richer comparison UI are added.
- [ ] Map every later design requirement to one of the sub-plans in the sequence table; list remaining feasibility blockers without deleting scope. Include the full local gate, relevant GUI/TUI parity tests, baseline reruns, large-file/typing stress and native-platform gaps in the foundation acceptance steps. Foundation completion is an internal milestone, not completion of the agreed release.
- [ ] Self-review the foundation plan for spec coverage, interface consistency, precise tests and proportional size. Verify local Markdown links and `git diff --check`; commit the completed findings and foundation plan. Present the concrete plan and unresolved evidence before implementation proceeds.

## Completion criteria for this phase

The phase is complete when recorded baseline evidence is reproducible, storage and writer decisions have supporting measurements/tests, ERB/Haml and both SQL dialects each have explicit demonstrated or blocked capabilities, and the foundation plan can be implemented without guessing at buffer ownership or save semantics. A report consisting of tool availability or successful compilation is insufficient.

Execution progress and actual results are recorded in the linked evidence report; unchecked tasks remain pending. Proposed production interfaces are not implemented capabilities. Production implementation remains a separate task; preserve the accepted feature scope while resolving feasibility.
