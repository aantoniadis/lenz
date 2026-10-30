# Integrated Lenz editor

Date: 2026-10-23

Status: consolidated design from the accepted interview decisions Q1–Q30, including the additions of Haml, PostgreSQL and DuckDB, and the draft-aware navigation and external-handoff decisions accepted during the plan review. This describes required behavior, not implemented capabilities. The [readiness plan](../plans/2026-10-23-editor-readiness.md#review-decisions-and-evidence) records the review and remaining feasibility work.

## Intent and release boundary

Make Lenz Code a full-fledged text editor with an integrated reading, editing and comparison experience. The user currently uses IntelliJ and VS Code and wants to stay in Lenz when changing code and other text files. Preserve the Reader's navigation, appearance, responsiveness and memory characteristics.

The first release includes ordinary editing, language assistance, file management, recovery, change markers and editable comparisons in both GUI and TUI. Prioritize practical support across the requested languages before advanced features for one language. A connected language server alone does not establish support: verify each advertised capability and publish the remaining gaps.

Deferred from this release: AI completion, project-wide replace, symbol rename and refactoring across files, integrated application/test execution and debugging, Git staging/committing/pushing, a language-tool installer/updater, and SQL execution, EXPLAIN and result grids. File renaming and moving are included as filesystem operations; they do not rewrite source references. Existing Diff, repository, PR and merge workflows must retain their behavior.

## Editing and document ownership

- Writable local source tabs accept typing immediately. Offer an explicit read-only mode; show the reason when a source cannot be edited. Preserve restrictions for virtual sources, unavailable originals and files that cannot safely round-trip through decoding.
- Provide new documents, Save As, Unicode text input and selection, IME composition in the GUI, terminal paste handling, cut/copy/paste, deletion, multi-cursor editing, indentation, paired delimiters, comment toggling, line movement/duplication, in-file find/replace and basic snippets with placeholders.
- Starting an edit retains a preview tab before another navigation can replace it.
- Within one session, a file has one draft and undo history shared by its tabs, split panes and comparison views. Panes keep independent cursor/selection, scroll and presentation state. Independently opened windows/sessions retain independent documents and drafts.
- Use meaningful undo transactions: typing groups, paste, a completion with its import edits, formatting and hunk reversion must each be understandable and reversible. Saving preserves the session's ability to undo edits.
- Large files remain editable within measured supported limits. Expensive highlighting, completion or analysis may be reduced with a visible explanation. Specify and verify limits rather than promising unlimited sizes.

The precise buffer structure, undo representation and cache strategy are builder decisions. Unsaved content must survive inactive-tab eviction and background refresh; reloading the disk file is not a substitute for retaining a draft. Preserve the repository's requirements for bounded work, compact undo and viewport-limited rendering.

## Saving, recovery and external changes

Save is explicit by default. Show unsaved state separately from Git status. Closing a dirty document, quitting, replacing a project or performing a checkout that would invalidate drafts uses Save/Discard/Cancel. File navigation and pane changes preserve drafts without prompting.

Reuse the existing safe-write behavior at the shared-core boundary: maintain encoding, BOM, line endings and file mode; validate source identity and external changes; preserve error handling for symlinks, hard links, failed writes and lossy decoding. A failed save leaves the draft available.

Distinguish a save rejected before modification from an I/O failure whose disk outcome is uncertain. Keep the draft, undo history and last confirmed save baseline after an unconfirmed save; report the uncertainty and recheck disk before reconciliation or another write. Existing in-place writing uses best-effort rollback, so preserving its compatibility does not establish universal atomicity or crash durability. Do not silently retry against a newly read baseline.

Keep private local recovery copies of unsaved work, including new documents. Reopening a project restores tabs, split layout, positions and recovered drafts. Clearly identify recovered content, check the original against the recovery baseline and ensure explicit Discard prevents a discarded draft from returning. Recovery does not write the original file. Keep source contents and database credentials out of diagnostic logs and shareable project configuration.

When disk content changes under a dirty draft, retain the draft and offer a comparison. Let the user take the disk version, retain their version or resolve the differences. Recheck the chosen disk baseline at save time. A clean document may follow external changes using the existing refresh behavior.

Formatting is an explicit command by default. Offer project-specific format-on-save and respect the project's formatter and indentation configuration. Resolve EditorConfig/project preferences without changing unrelated files or installing tools.

## Reading and navigation while editing

Project search, symbol search and navigation use unsaved drafts for open files and disk for unopened files. Results, previews and activation must agree on the text and source version. If a target changes after a result arrives, revalidate or recompute it; do not navigate using obsolete disk coordinates or replace a draft with disk text. Preserve this behavior for existing usages, peek and definition workflows, including a different dirty target file.

Blame annotations and their commit actions must refer to the displayed text. Invalidate obsolete attribution after edits; while recomputation is pending, show that state and prevent actions on stale annotations. Keep folds, bookmarks, selections and find results consistent with the edited document as well.

Opening a dirty file in an external editor or app offers **Save and Open**, **Open Saved Version**, and **Cancel**. Save and Open launches only after confirmed save success. Open Saved Version leaves the draft untouched and uses a position from the saved text, omitting a precise position when it cannot be established. Cancel launches nothing. Existing external comparisons must identify their disk inputs; internal editable comparisons use the shared draft as specified below.

## Keyboard and completion interaction

Offer IntelliJ and VS Code keymap presets, defaulting to IntelliJ, plus individual shortcut overrides. Resolve conflicts explicitly: current Reader bindings include Ctrl+R for Reload, Ctrl+H for Type Hierarchy and F2 for language-server status. The same semantic actions must remain accessible through both adapters' menus, including when terminals cannot deliver a particular shortcut.

In editable terminal text, printable keys insert text. Existing reading shortcuts such as q, j, k, / and Space cannot consume text input. Focus movement uses explicit commands/shortcuts. Active dialogs, menus and completion interactions take precedence over the underlying editor according to the shared modal rules.

Completion appears automatically while typing and has a manual trigger and an automatic-popup toggle. Offer symbols/members, documentation, signature help, supported automatic imports, and snippets. Tab and Enter accept the selected suggestion; Escape dismisses it. Punctuation does not implicitly accept suggestions. Outside completion, Tab visits snippet placeholders or indents.

Apply a completion and its related edits as one undoable transaction. Validate document versions and ranges before applying any provider result. Typing and navigation stay responsive while providers work; stale requests cannot reopen a dismissed popup or edit a different document. Incomplete or unavailable assistance has a visible status. Multi-cursor operations must remain coherent and must not apply unrelated edits to incompatible cursor contexts.

Diagnostics, quick fixes, formatting and organize-imports commands belong to the first release where supported by the configured provider. An action requiring changes outside the current document is unavailable in this release with an explanation; never partially apply it. These limits also apply to server-initiated edits and commands.

## Language and file support

| Family | Required experience |
| --- | --- |
| Java, Kotlin | Completion/imports, parameter help, diagnostics, formatting and correct project/dependency discovery. Verify mixed projects and identify cross-language gaps. |
| Rust, Go | Type-aware completion, imports, diagnostics, quick fixes and formatting. |
| Ruby, Rails | Constants/methods, gem/project context, Rails-aware assistance and configured diagnostics/formatting. |
| ERB, Haml | First-class template editing with Ruby-aware assistance and markup assistance, including `.html.erb` and `.html.haml`. Respect source locations, interpolation and indentation. |
| TypeScript, JavaScript, TSX | Completion/imports, React component/prop assistance, diagnostics and formatting. |
| HTML | Tags, attributes, closing tags, formatting and appropriate embedded-language assistance. |
| Python | Completion, diagnostics and formatting using the project's configured environment/tools. |
| Markdown | Source editing with live rendered preview, list continuation, heading/file-link completion and preview navigation. Direct editing of rendered Markdown is deferred. |
| YAML | Schema-aware key/value completion, descriptions, validation and indentation. |
| Bash, sh | Variable/function assistance, diagnostics and formatting appropriate to the script's shell. |
| `.env` | Predictable key/value editing, highlighting and appropriate quoting rules. Treat environment-file contents as data. |
| PostgreSQL, DuckDB SQL | Explicit dialect, query-aware completion, function help, diagnostics and formatting as specified below. |

Reuse installed and project-provided servers/formatters. Show the active tool, environment/project, readiness and missing capabilities, with clear setup guidance. Start providers as needed and preserve cancellation and resource bounds. A missing provider does not prevent ordinary text editing.

### PostgreSQL and DuckDB

Each project can declare a default SQL dialect. Files can override it, including standalone files; show and persist the active selection. An unset dialect remains visibly unset until selected, rather than silently treating all SQL as PostgreSQL.

The acceptance bar covers aliases, CTEs, subqueries, quoted identifiers, schema-qualified names, function help, syntax diagnostics and dialect-aware formatting. Test incomplete queries during typing and dialect-specific syntax separately for PostgreSQL and DuckDB.

Without a configured database, provide dialect and query-local assistance. Optionally use an explicitly configured PostgreSQL connection or DuckDB database file for catalog metadata such as tables, columns and functions. Opening an SQL file does not initiate a connection. Keep connection credentials private and open database resources with appropriate read-only access.

Connections are for schema information only. Do not execute the user's SQL, run EXPLAIN or show result grids. Metadata queries are permitted; database-backed validation that requires submitting the edited statement is outside this release. Distinguish syntax/schema diagnostics from actual database validation. Provider selection must respect this boundary, including disabling execution-based features where necessary.

## Review and change visibility

Show blue gutter marks for modified lines, green for additions and a small boundary marker for deletions. Respect appearance themes and provide non-color identification in the terminal. The markers coexist with line numbers, folds, bookmarks and optional blame.

For a tracked Git file, compare the current draft to HEAD. Include unsaved edits and retain repository-change markers after Save. A marker disappears when the draft matches its committed baseline. Outside Git, compare against the last saved version. New/untracked files have no committed baseline and must be identified as new. Refresh baseline identity after repository changes without losing drafts.

Highlight changed files in the explorer and tabs with status colors/badges for modified, added, untracked and conflicted files. Keep actual Git status and unsaved/recovered state distinguishable, including a draft whose disk file is still Git-clean.

Activating a gutter marker shows the previous content and offers Show Diff and Revert This Change. Equivalent keyboard commands exist in both adapters. Reverting alters the draft as an undoable edit and reaches disk only through Save. Label the baseline so the user can distinguish reverting to HEAD from reverting to the last save; changing a draft does not alter the Git index.

Open comparisons within the same window/session. Support comparison to the last saved version and the Git baseline, as well as external-change reconciliation. The prior version is read-only; the current side edits the same draft and undo history as the ordinary editor tab. Switching views preserves work and position. A comparison is not a separately editable copy of the file.

## File management

The explorer supports creating, renaming, moving and deleting files and folders. Protect unsaved drafts and reconcile open-file identity, navigation, watches and provider state when paths change. Use recoverable deletion where supported; permanent deletion requires an explicit action. Filesystem operations do not claim to perform semantic reference updates.

## Existing foundations and gaps

These are source-inspection findings at design time, not editing acceptance evidence:

- The Reader has an immutable [document snapshot](../../../crates/code-core/src/document.rs) and command-owned [session](../../../crates/code-core/src/session.rs). Its inactive-cache/reload assumptions need reconsideration for drafts.
- Diff already has [safe writing](../../../crates/app/src/write.rs) and a [hunk-apply draft lifecycle](../../../crates/app/src/edit_session.rs). The latter is not a general text-editing buffer. Reader comparisons currently launch a separate executable.
- Existing semantic providers cover Rust, Python, TS/JS/TSX, Java, HTML/CSS, Ruby and Kotlin. The client advertises `workspace.applyEdit = false` and rejects edit requests. See the [provider guide](../../code/providers.md) for tested navigation behavior and limits.
- The Reader has gutter infrastructure and repository identity, but no Git-change gutter state or per-file status presentation. Shared Git blob reading and Diff's change-set model are possible reuse points.
- ERB and Haml lack recognized language variants, grammar integration, template routing and dedicated fixtures. Ruby/Rails startup support does not establish template support.
- SQL has a single generic syntax grammar, with no dialect selection, SQL semantic provider, schema catalog or database settings.
- Reader keybindings are hard-coded in the adapters; menu shortcut labels do not drive input handling.

## Engineering choices and validation

The builder owns buffer/storage design, module boundaries, provider selection, projection/source-map techniques for embedded languages, schema-cache strategy, recovery format and implementation sequencing. Use the existing workspace's Rust architecture and direct in-process shared-core calls. Keep state transitions, availability, validation, undo, file safety and background orchestration in the shared core; adapters own input translation and presentation.

Resolve the high-risk feasibility work early: Haml/ERB Ruby/markup assistance, PostgreSQL/DuckDB assistance without statement execution, editable comparison ownership, efficient sustained editing and recovery. Document unsuccessful trials. Do not substitute highlighting-only template support or generic SQL suggestions for the accepted requirements without returning to the user. Provider limitations may be documented, but they cannot silently remove the explicitly required experiences above.

Before architecture or performance-sensitive implementation, read the [benchmark guide](../../benchmarking.md), [optimization notebook](../../optimization-notebook.md), [Reader performance guidance](../../code/performance.md) and [terminal capability limits](../../code/tui.md). Preserve startup, responsiveness and memory against equivalent measured release baselines; any deliberate performance tradeoff needs explicit agreement.

Acceptance must include:

1. The same GUI/TUI scenarios for input, selection, multi-cursor editing, snippets, undo/redo, file switching, saving/reloading, modal input, cancellation and empty states, with matching semantic state and file bytes.
2. Completion/import/quick-fix/formatting transactions against current document versions; rejection of obsolete or out-of-scope edits; real-provider checks per claimed language capability.
3. Draft survival across preview replacement, split/comparison views, cache pressure, external modification/deletion, project transitions, recovery and explicit discard.
4. Encoding/BOM/line-ending preservation, write-failure behavior and concurrent external-change protection using disposable fixtures.
5. Correct Git versus unsaved markers across save, undo, commit/HEAD changes, staged changes, new files and deletions; marker reversion leaves the index unchanged.
6. ERB/Haml Ruby and markup cases, distinct PostgreSQL/DuckDB fixtures, offline SQL assistance and explicit metadata connection tests proving edited statements are not executed.
7. Light/dark appearances, narrow layouts, tiny terminals and actual input-to-command mappings. GUI automation uses a private display and disposable repositories.
8. Equivalent release startup, typing/completion feedback, large-file edits, peak/settled memory and sustained editing stress, including child language-server resources where relevant. Keep raw samples and identities; report untested platforms, including native macOS when not exercised.
9. Draft-aware project/symbol search and result preview/activation, including edits after results arrive and dirty target files; accurate blame actions after line insertion/deletion/undo; explicit external-open choices without silent saves or launches after save failure.

Use `scripts/ci-local.sh` as the local gate, the real Reader GUI/TUI adapter and parity suites, and the Diff adapter checks when shared or comparison behavior changes. Follow the repository's documentation-only checks while this remains a design change.

## External references

- [IntelliJ gutter markers](https://www.jetbrains.com/help/idea/editor-gutter.html): reference for change types and marker actions.
- [Ruby LSP ERB support](https://shopify.github.io/ruby-lsp/#erb-support): host-language delegation requires editor integration. A corresponding Haml implementation path remains unverified.
- [DuckDB dialect](https://duckdb.org/docs/current/sql/dialect/overview): PostgreSQL ancestry does not make the dialects interchangeable.
- [DuckDB autocomplete extension](https://duckdb.org/docs/current/core_extensions/autocomplete): an existing completion facility, not proof of an editor integration.
- [Postgres Language Server initial release](https://supabase.com/blog/postgres-language-server): illustrates catalog-based completion and EXPLAIN-based validation; the latter is outside the accepted scope.

## Decisions the builder must report

Alongside the implementation plan, report selected providers and demonstrated capability limits, the proposed editable-document ownership model, measured size limits, and any requirement whose feasibility remains unproven. These are engineering investigations; the accepted product scope above remains the target.
