# Integrated Lenz Reader and Editor

Status: claimed

## Problem Statement

Lenz lets me read and navigate a project, but I still leave it for IntelliJ or VS Code when I need to change code or other text files. Switching applications breaks my reading context, separates comparison from editing, and makes simple changes more cumbersome. I want one integrated, full-fledged reader and editor with autocomplete, reliable saving, useful language support and visible changes in both the GUI and terminal.

## Solution

Make the Lenz Reader an integrated editor: ordinary source tabs accept edits, split panes and comparisons share the same session draft, and navigation continues to work against unsaved text. Provide safe explicit saving, undo/redo, recovery, file management, language assistance, Markdown preview and clear Git/unsaved change visibility through both GUI and TUI.

Deliver the work in testable milestones. The first internal foundation milestone supports existing-file editing, shared buffers/history, safe Save, dirty-transition protection, current-draft navigation and blame, and a minimal comparison against the last saved version. Subsequent milestones complete editing commands/keymaps, document lifecycle/recovery, language assistance, Git review features, templates/Markdown and SQL. Completing the foundation alone does not complete this spec.

## User Stories

1. As a developer, I want to edit a writable local file directly where I am reading it, so that I can make changes without switching applications.
2. As a developer, I want to move between reading, editing and comparisons in one Lenz session, so that I can keep the same working context.
3. As a terminal user, I want to perform the same supported editing operations as a GUI user, so that my terminal workflow does not lose functionality.
4. As a developer, I want to see why a source is read-only and explicitly enable read-only mode, so that I can distinguish a deliberate restriction from an editor failure.
5. As a developer, I want to create new documents and save them to a chosen location, so that I can start files without leaving Lenz.
6. As a developer, I want to use Save As without silently overwriting another file, so that I can preserve the original and deliberately choose a destination.
7. As a developer, I want to type and select Unicode text accurately, so that non-English text, emoji and combining characters remain intact.
8. As a GUI user, I want to compose text through my input method and cancel or commit the composition, so that I can enter text naturally without corrupting selections.
9. As a terminal user, I want to paste text as one editing operation without interpreting it as commands, so that pasted content cannot trigger unrelated actions.
10. As a developer, I want to cut, copy, paste, delete and replace a selection, so that I can perform ordinary text editing efficiently.
11. As a developer, I want to edit multiple selections coherently, so that I can make the same intentional change in several places.
12. As a developer, I want to indent and unindent text using project conventions, so that my edits fit the surrounding source.
13. As a developer, I want to use paired delimiters and comment toggling, so that common code edits take fewer keystrokes.
14. As a developer, I want to move and duplicate lines, so that I can reorganize nearby code quickly.
15. As a developer, I want to find and replace text within the current file, so that I can make controlled local changes.
16. As a developer, I want to expand snippets and move between their placeholders, so that I can insert common structures efficiently.
17. As a developer, I want to have a preview tab retained when I start editing it, so that later navigation cannot replace my draft.
18. As a developer, I want to share one draft and undo history across tabs and split panes of the same file within a session, so that every view reflects my current work.
19. As a developer, I want to keep each split pane’s cursor, selection, scroll and presentation independent, so that I can work at different locations in the same file.
20. As a developer, I want to keep independently opened sessions’ drafts separate, so that one window cannot unexpectedly change another session’s document.
21. As a developer, I want to retain drafts and undo history while switching files or under cache pressure, so that returning to a file does not lose work.
22. As a developer, I want to undo and redo meaningful edits such as typing groups, paste, formatting and completion, so that I can predict what each history action reverses.
23. As a developer, I want to undo edits after saving, so that saving does not become an irreversible editing boundary.
24. As a developer, I want to see unsaved state separately from Git status, so that I can tell which changes exist only in memory.
25. As a developer, I want to save explicitly by default, so that disk files change only when I choose to write them.
26. As a developer, I want to retain encoding, BOM, untouched line endings and file mode when saving, so that a small text edit does not rewrite unrelated file properties.
27. As a developer, I want to have Save reject a changed or substituted target, so that Lenz does not overwrite another file or an external change unnoticed.
28. As a developer, I want to retain my draft and undo history after a failed save, so that I can recover or retry without reconstructing my work.
29. As a developer, I want to be told when a failed save leaves the disk outcome uncertain, so that I do not assume the old or new file is definitely present.
30. As a developer, I want to save a captured version while continuing to edit, so that a completed save cannot incorrectly mark newer edits as saved.
31. As a developer, I want to choose Save, Discard or Cancel before closing a dirty document, so that I control whether its unsaved work is retained.
32. As a developer, I want to receive the same protection before changing project, opening a worktree here or checking out a branch, so that workspace operations cannot silently invalidate drafts.
33. As a developer, I want to have automatic watcher, focus and index refresh retain dirty text, so that background activity cannot replace my draft with disk content.
34. As a developer, I want to compare a dirty draft with an externally changed file and choose how to reconcile it, so that I can resolve concurrent edits deliberately.
35. As a developer, I want to close or unsplit one view without losing a draft still owned by the session, so that layout changes do not destroy work.
36. As a developer, I want to cancel a multi-window or all-view quit without losing another window’s draft, so that one early approval does not destroy work before all decisions finish.
37. As a macOS user, I want to receive draft protection when quitting through ordinary native or Dock Quit, so that the operating system’s normal Quit route is as safe as Lenz’s menu.
38. As a developer, I want to recover private local copies of unsaved and new documents after interruption, so that unexpected termination does not erase recoverable work.
39. As a developer, I want to restore tabs, split layout, positions and recovered drafts when reopening a project, so that I can resume my previous work.
40. As a developer, I want to recognize recovered content and compare it with its original baseline, so that I can decide what should be retained.
41. As a developer, I want to have explicit Discard prevent recovery from restoring the discarded draft, so that a discarded change stays discarded.
42. As a developer, I want to keep source contents and database credentials out of diagnostic logs and shareable settings, so that private project information remains private.
43. As a developer, I want to search open files using their unsaved drafts and unopened files using disk contents, so that search results match what I am working on.
44. As a developer, I want to find draft-only declarations and stop seeing removed saved matches, so that symbol search reflects the current source.
45. As a developer, I want to use definition, usages, peek and multiple-result navigation against current drafts, so that editing does not break existing reading workflows.
46. As a developer, I want to see the same source version in a result, its preview and its activation, so that navigation does not jump to unrelated text.
47. As a developer, I want to have stale results revalidated or recomputed before activation, so that another edit cannot make an old result move the caret incorrectly.
48. As a developer, I want to keep selections, bookmarks, folds, find results and navigation anchors consistent after edits, so that reading aids remain useful while the document changes.
49. As a developer, I want to see blame and commit actions only for the displayed text, so that inserted or deleted lines do not receive another line’s attribution.
50. As a developer, I want to see pending blame status while attribution is recomputed, so that I know why commit actions are temporarily unavailable.
51. As a developer, I want to choose Save and Open, Open Saved Version or Cancel when opening a dirty file externally, so that I control what the external tool receives.
52. As a developer, I want to have Save and Open launch only after confirmed save success, so that a failed or uncertain save cannot open misleading content.
53. As a developer, I want to open the saved version externally without losing my draft or sending incorrect coordinates, so that I can inspect disk content safely.
54. As a developer, I want to know that an external comparison uses saved disk inputs, so that I do not mistake it for a comparison of unsaved drafts.
55. As an IntelliJ user, I want to start with familiar IntelliJ key bindings, so that I can transfer my editing habits to Lenz.
56. As a VS Code user, I want to select a VS Code keymap preset, so that I can retain my preferred shortcuts.
57. As a developer, I want to override individual shortcuts and see conflicts resolved explicitly, so that custom bindings do not silently hide actions.
58. As a terminal user, I want to insert printable keys such as q, j, k, slash and Space when editing text, so that reading shortcuts do not consume the text I type.
59. As a developer, I want to access semantic actions through command menus in both adapters, so that an unavailable terminal shortcut does not make the feature unreachable.
60. As a developer, I want to have active dialogs and completion interactions receive input before the document, so that modal input cannot accidentally edit or save underlying text.
61. As a developer, I want to receive completion automatically while typing and trigger it manually, so that I can discover and insert relevant symbols efficiently.
62. As a developer, I want to turn automatic completion popups off while keeping a manual trigger, so that I can control interruptions.
63. As a developer, I want to see symbol/member suggestions, documentation and signature help, so that I can understand an API without leaving the editor.
64. As a developer, I want to accept selected completion with Tab or Enter and dismiss it with Escape, so that completion behaves predictably.
65. As a developer, I want to type punctuation without implicitly accepting a suggestion, so that ordinary typing does not insert an unintended completion.
66. As a developer, I want to use Tab for snippet placeholders or indentation when completion is not active, so that the key follows the current editing context.
67. As a developer, I want to apply a completion and its supported import edits as one undoable transaction, so that undo restores the file coherently.
68. As a developer, I want to have obsolete completion and analysis responses ignored, so that slow providers cannot edit another version or reopen a dismissed popup.
69. As a developer, I want to see diagnostics, quick fixes, formatting and organize-imports actions where my provider supports them, so that I can correct code in the editor.
70. As a developer, I want to have actions requiring another file’s edits refused with an explanation, so that a single-file action never partially applies a wider refactor.
71. As a developer, I want to format explicitly by default and optionally configure format-on-save per project, so that formatting follows my project’s conventions and my chosen timing.
72. As a developer, I want to respect EditorConfig, indentation and configured project tools, so that Lenz fits the repository without changing unrelated settings or installing tools.
73. As a developer, I want to see the active language tool, project environment, readiness and missing capabilities, so that I can understand and resolve unavailable assistance.
74. As a developer, I want to continue ordinary editing when a language provider is missing or slow, so that tool availability does not block text changes.
75. As a Java or Kotlin developer, I want to use project-aware completion, imports, parameter help, diagnostics and formatting, so that I can edit JVM projects with useful dependency context.
76. As a Java or Kotlin developer, I want to see verified mixed-project behavior and explicit cross-language limitations, so that I can judge which navigation and editing assistance to trust.
77. As a Rust or Go developer, I want to use type-aware completion, imports, diagnostics, quick fixes and formatting, so that I can make normal changes without returning to another IDE.
78. As a Ruby or Rails developer, I want to receive constants, methods, gem context and Rails-aware assistance with configured diagnostics and formatting, so that I can work effectively in application code.
79. As a template author, I want to receive Ruby-aware and markup assistance in ERB and Haml, including .html.erb and .html.haml, so that I can edit templates as first-class source files.
80. As a template author, I want to have completion, diagnostics and edits map correctly through interpolation, indentation, Unicode and multiline Ruby, so that generated language projections cannot corrupt visible template text.
81. As a TypeScript or JavaScript developer, I want to receive completion, imports, diagnostics and formatting in TS, JS and TSX, so that I can work across browser and application code.
82. As a React developer, I want to receive component and prop assistance in TSX, so that I can edit component usage accurately.
83. As an HTML author, I want to receive tag, attribute, closing-tag, formatting and appropriate embedded-language assistance, so that markup editing is more than syntax coloring.
84. As a Python developer, I want to use completion, diagnostics and formatting from the project’s configured environment, so that assistance matches the dependencies I actually use.
85. As a Markdown author, I want to edit source with a live rendered preview, so that I can check the result while retaining precise source control.
86. As a Markdown author, I want to use list continuation, heading and file-link completion, and preview navigation, so that I can write and navigate documents efficiently.
87. As a YAML author, I want to receive schema-aware key/value completion, descriptions, validation and indentation, so that configuration errors are easier to avoid.
88. As a shell author, I want to receive variable/function assistance, diagnostics and formatting appropriate to Bash or sh, so that assistance respects the script’s actual shell.
89. As a developer, I want to edit .env files with predictable key/value highlighting and quoting rules, so that environment values are treated as data.
90. As an SQL author, I want to declare a project SQL dialect and override it for individual or standalone files, so that PostgreSQL and DuckDB files receive the appropriate assistance.
91. As an SQL author, I want to see the active dialect and have the selection persist, so that I do not unknowingly edit under another dialect.
92. As an SQL author, I want to see an unset dialect remain visibly unset until I choose one, so that Lenz does not silently assume PostgreSQL.
93. As an SQL author, I want to receive offline assistance for aliases, CTEs, subqueries, quoted identifiers, qualified names and functions, so that I can write useful queries without connecting a database.
94. As an SQL author, I want to receive dialect-aware syntax diagnostics and formatting while queries are incomplete, so that assistance remains useful during normal typing.
95. As a DuckDB author, I want to receive correct assistance for constructs such as SELECT * EXCLUDE and GROUP BY ALL, so that PostgreSQL similarity does not hide DuckDB differences.
96. As an SQL author, I want to optionally configure a PostgreSQL connection or DuckDB file explicitly for metadata, so that table, column and function suggestions can use my schema when I choose.
97. As an SQL author, I want to open SQL files without initiating database connections, so that browsing a file has no unexpected database side effects.
98. As an SQL author, I want to keep credentials private and database access appropriately read-only, so that schema assistance does not expose credentials or modify my data.
99. As an SQL author, I want to have edited SQL never executed or submitted to EXPLAIN, so that editor assistance cannot run statements against a database.
100. As an SQL author, I want to see schema freshness and connection failures and distinguish syntax/schema checks from database validation, so that I understand the limits of the assistance.
101. As an SQL author, I want to have results from an earlier dialect or request discarded after switching context, so that old assistance cannot affect the new dialect’s document state.
102. As a developer, I want to see blue modified-line marks, green additions and deletion boundary marks, so that I can locate changes while reading source.
103. As a terminal user, I want to identify change types without relying only on color, so that change visibility remains accessible across terminal capabilities.
104. As a developer, I want to compare tracked-file drafts against HEAD, including unsaved edits, so that saving does not erase repository-change markers.
105. As a developer, I want to compare non-Git files against their last saved version and recognize new/untracked files, so that change markers always identify their baseline.
106. As a developer, I want to see modified, added, untracked and conflicted file badges alongside separate dirty/recovered state, so that the explorer and tabs describe both repository and draft state.
107. As a developer, I want to inspect previous content from a marker and open its comparison, so that I can understand a change before acting on it.
108. As a developer, I want to revert a marked change as one undoable draft edit against a clearly labelled baseline, so that I can reverse work without silently writing disk or changing the Git index.
109. As a developer, I want to edit the current side of an in-window comparison using the ordinary tab’s draft and history, so that comparison is another view of my work rather than a separate editable copy.
110. As a developer, I want to refresh repository baselines without losing drafts or applied/undone history, so that repository updates do not erase my editing context.
111. As a developer, I want to create, rename, move and delete files and folders through the explorer, so that routine file management stays inside Lenz.
112. As a developer, I want to have filesystem operations protect dirty drafts and reconcile open-file identity and navigation, so that renaming or moving a file does not strand its editor state.
113. As a developer, I want to use recoverable deletion where supported and explicitly choose permanent deletion, so that ordinary file deletion offers the available recovery path.
114. As a developer, I want to edit large files within measured supported limits with visible analysis reductions, so that performance limits are honest and do not silently remove ordinary editing.
115. As a developer, I want to keep typing, scrolling, navigation and provider interaction responsive, so that language analysis does not stall the editor.
116. As a developer, I want to use supported light/dark appearances, narrow windows and tiny terminals safely, so that layout constraints do not cause hidden actions or lost work.
117. As a developer, I want to open empty views and use Undo, Reload or dialog dismissal safely, so that a fresh or empty editor remains usable.
118. As an existing Lenz user, I want to retain Diff, repository, pull-request and merge behavior while the Reader gains editing, so that the integrated editor does not regress established workflows.

## Implementation Decisions

- The shared Reader core owns commands, application transitions, availability, validation, editing transactions, undo, writes and background orchestration. GUI/TUI adapters translate input and present that state; they cannot maintain competing behavior.
- Each session owns a buffer registry. A stable buffer identity is separate from paths; tabs, split panes and comparison views reference one buffer and one history. View-local selection, scroll and presentation remain independent. Separate sessions retain separate drafts.
- Starting an edit retains a preview tab. Live draft, save baseline and history ownership survive inactive-cache eviction and file navigation. Explicit discard or final buffer closure may release that ownership.
- Use indexed persistent UTF-8 chunks as the selected foundation direction, initially approximately 4 KiB chunks with newline indexes and per-chunk UTF-16 counts. Readiness measurements support cheap immutable snapshots; they do not establish universal superiority over gap buffers or production performance.
- Current input and rendering read immutable chunked text snapshots through a view facade. A contiguous Document remains an explicit background-analysis representation. Share source metadata between the two; materializing draft UTF-8 must not rediscover and lose its original encoding metadata.
- Input, rendering, hit-testing and GUI surrounding-text requests must not implicitly flatten an entire document. Bounded line windows retain absolute source coordinates. Long-line layout uses reusable checkpoints and does not repeatedly scan hidden tails. UTF-16 conversion rejects positions inside surrogate pairs.
- Edits carry the target view, expected buffer/version, validated UTF-8 ranges and an edit kind. Reject stale, overlapping or out-of-scope transactions in full. Every edit, undo and redo advances version identity; dirty state reflects content against the confirmed baseline rather than version-number inequality.
- Keep compact forward/inverse edit records and meaningful groups. The foundation plan coalesces adjacent typing/deletion from the same view and direction within 500 ms, ending a group on caret/selection movement, paste, Save, focus change or an intervening command. Save retains history. Measure retained-history growth separately from leaks; no silent history cap is approved.
- Admit background analysis through bounded shared workers, with one running version and one replaceable pending version per buffer and a bounded global queue of buffer identities. Materialize each admitted version once and share it. Results must match buffer/version, operation request and relevant project/provider/context generation before publication; derived results never replace authoritative draft text.
- Reuse narrow file-writing primitives through shared content services while preserving existing Diff and merge behavior. The Reader uses an exact-line-ending route so untouched mixed endings remain unchanged. Do not repurpose the comparison-specific Diff editing session as the Reader buffer.
- Capture the confirmed canonical target and device/inode identity from the same opened file used for baseline bytes. Reader Save rejects same-path substitution even when content and canonical path match, including substitution by a hard link to another same-content file. Refresh identity after a confirmed replacement; retain relevant caller-side identity and repository guards.
- Save distinguishes confirmed success, known pre-write rejection and uncertain I/O outcome. Success advances only the captured saved version and retains history; newer edits remain dirty. An uncertain outcome retains draft/history and the last confirmed baseline, reports uncertainty, and requires disk revalidation before the next Save, external handoff or destructive project/file transition decision. Explicit user exception (2026-10-24): a fresh Discard Draft and Close for actual tab, pane, window or all-view closure may discard the in-memory draft/history without disk recheck, leaving the current disk bytes untouched. The failed old close remains cancelled; it never resumes automatically. Never silently retry against a newly accepted baseline. Do not promise universal atomicity or crash durability.
- Serialize writes per buffer and keep ownership until completion. Delayed or obsolete save completions cannot resume another action or mark newer text clean. In multi-file save flows, earlier confirmed writes remain successful if a later file fails; this is not an all-files transaction.
- Guard Reload, project/worktree replacement, checkout and new-branch checkout, dirty tab closure, window/view closure and all-view quit before destructive work starts. Save/Discard/Cancel decisions belong to the core. Automatic refresh retains dirty text and records conflicts instead of replacing the draft. Removing a view does not discard a shared buffer.
- All-session quit uses a prepare/commit/cancel batch across independent sessions. Do not remove or discard a session before all participants agree; cancellation or an invalidated permit prevents termination. An earlier explicitly completed Save is not silently rolled back when a later action is cancelled.
- Ordinary native macOS Dock/Quit must join the same batch before teardown. The pinned GUI framework lacks the required vetoable native termination hook. A tracked framework/platform integration and real native acceptance tests are prerequisites for enabling editing on that target; a non-vetoable shutdown cleanup callback is insufficient. Forced termination is separate from ordinary Quit.
- Project text/symbol search and semantic navigation overlay current snapshots of open files on disk content for unopened files. Each target has its own captured source identity, separately from the origin request. Results and previews agree on that snapshot; activation revalidates or recomputes changed targets rather than using old disk coordinates.
- Transform view anchors through transactions and invalidate derived folds, find matches, syntax and attribution when their versions no longer match. Blame recomputes from current draft contents, marks changed/untracked lines appropriately, and disables stale commit actions while pending or failed.
- A dirty external-editor/default-app handoff offers Save and Open, Open Saved Version and Cancel. Launch only once after confirmed Save; saved-version opening retains the draft and uses saved coordinates or omits an uncertain position. External comparisons identify saved disk inputs.
- The initial internal comparison has an immutable last-saved side and a current side referencing the existing buffer/history. Use the existing pure diff engine through shared core calls. Later comparisons add HEAD and external-change reconciliation without creating separate editable copies.
- GUI text composition is presented by the adapter until commit, then applies one core transaction against the snapshot used for position conversion. Terminal bracketed paste is one transaction. Active modal/completion input takes precedence; editable terminal printable keys insert text.
- Provide IntelliJ-default and VS Code keymap presets plus overrides, with centrally exposed semantic actions and disabled reasons. Resolve existing Reload, Type Hierarchy and language-status binding conflicts explicitly. Keep equivalent menu access where terminals cannot deliver a shortcut.
- Completion supports automatic/manual invocation, an automatic-popup toggle, documentation, signatures and supported imports/snippets. Tab/Enter accept, Escape dismisses, punctuation does not accept. Outside completion, Tab visits snippet placeholders or indents. Stale replies cannot reopen a dismissed popup.
- Completion plus related edits, formatting, quick fixes, organize imports and hunk revert are coherent undo transactions. Provider actions touching another document, including server-initiated edits/commands, are unavailable with an explanation; never apply only the current-file portion.
- Formatting is explicit by default with a project-specific format-on-save option. Respect EditorConfig, project formatting/indentation preferences and installed/project-provided tools. Expose tool identity, environment, readiness, limits and setup guidance; ordinary editing remains available without a provider.
- Language coverage includes Java, Kotlin, Rust, Go, Ruby/Rails, ERB, Haml, HTML, TypeScript, JavaScript, TSX, Markdown, YAML, Python, Bash/sh, .env, PostgreSQL and DuckDB. Publish capability-specific evidence and gaps rather than treating a connected server as complete support.
- ERB/Haml require Ruby and markup assistance with correct source mappings. Test-only explicit projections demonstrated a useful primitive but not automatic grammar recognition or first-class support. Reject generated-scaffolding/foreign-file edits as a whole transaction. Native ERB replacement ranges and encoding-dependent signatures require resolution, as do Rails context and Haml markup edit/formatting support.
- Markdown is edited as source with live rendered preview, navigation, list continuation and heading/file-link completion. Direct editing of the rendered preview is outside this release.
- SQL dialect is a visible, persisted project default with file/standalone overrides; unset stays visibly unset. PostgreSQL and DuckDB are distinct accepted dialects. Offline assistance must understand query-local names, functions, syntax and formatting, including incomplete queries and dialect-specific constructs.
- Optional PostgreSQL connections and DuckDB files provide explicitly requested catalog metadata only. Opening a file never connects automatically. Credentials stay private; access is appropriately read-only; edited SQL and EXPLAIN are never submitted for execution. Label syntax/schema checks separately from actual database validation.
- DuckDB builtin completion, parsing, formatting and metadata operations are useful candidate boundaries, but measured alias/CTE/nested/quoted and empty-qualified-name gaps remain unresolved. PostgreSQL language-provider selection remains unresolved. Future requests/results carry dialect, buffer/version, request and catalog-generation identity, with cancellation on context changes.
- Git line markers compare tracked drafts, including unsaved edits, to HEAD; outside Git the baseline is last saved. Save does not clear repository-change markers. New/untracked files are identified without inventing a committed baseline. Use blue modified, green added and deletion-boundary marks, plus non-color identification.
- Explorer/tab Git status remains distinct from dirty/recovered state. Marker inspection identifies its baseline and offers previous content, Show Diff and undoable draft reversion. Reversion reaches disk only via Save and does not change the Git index. Baseline refresh preserves drafts and existing applied/undone behavior.
- Explorer creation/rename/move/delete reconciles buffer paths, navigation, watches and providers while protecting drafts. Prefer recoverable deletion where supported; permanent deletion is explicit. Filesystem moves and renames do not promise semantic reference updates.
- Recovery copies are private, local and separate from original files. Restore the session and identify recovered content, compare its baseline with current disk content, and prevent explicit discarded work from returning. Full recovery remains a later milestone after foundation transition guards.
- Preserve existing startup, responsiveness and memory through equivalent measured release workloads. Keep frontend dependencies separate, work lazy/cancellable, caches bounded and rendering viewport-limited. Any deliberate performance tradeoff requires explicit user agreement; readiness measurements do not approve a production regression.
- Implement the foundation first, then complete editing/keymaps, document lifecycle, language assistance, Review/Git, templates/Markdown and SQL. Later work consumes shared interfaces; this ordering does not remove any requirement from the full release.

## Testing Decisions

- The user confirmed the primary behavioral boundary: drive the public shared Session command interface, then replay the same workflows through the real GUI/TUI inputs and compare observable state and file bytes. Add focused save/provider/performance integration checks where this main boundary cannot expose the relevant effect.
- A good test exercises a user-visible operation and verifies its externally observable outcome, including exact selected text, dirty/read-only/conflict state, available actions, launched external tools and final file bytes. Avoid tests that merely reproduce private algorithms, field layout or implementation steps.
- Prefer existing session command/completion tests, real-constructor GUI adapter tests, terminal event/render tests and cross-adapter parity replays. Extend these established seams instead of creating separate test-only editor controllers in each subsystem. Keep platform-specific rendering assertions separate from shared behavior.
- Replay the first foundation trace in both adapters: open an existing file, replace a selection, type/paste Unicode text, split, edit through either view, undo/redo, Save, switch files through actual cache eviction, return and undo. Assert shared current text/history inside the session, independent view positions and an unchanged second session.
- Cover preview retention, clean buffers with undo history, shared comparison ownership, explicit read-only mode, unavailable/virtual/binary/lossy sources, empty constructors and empty Undo/Reload/dialog dismissal. Validate data survives navigation and view/layout changes.
- Use the existing writer regression and fault-injection approach against disposable files. Assert encoding/BOM/mixed-ending/mode preservation, hard-link compatibility, retargeted-symlink refusal and same-path same-content hard-link substitution refusal. Include successful replacement followed by another Save using refreshed identity.
- Exercise pre-write rejection, failed partial-write rollback and errors after replacement without assuming all failures leave disk unchanged. Assert draft/history/confirmed baseline retention, visible uncertainty, no silent retry and no external launch. Delay Save completion until after another edit or pending action changes.
- Table-test every dirty transition, including current automatic refresh, deleted external files, project/worktree changes, checkout/new-branch checkout, last tab close, native window close, terminal view close and all-view quit. Assert cancellation preserves drafts and prevents destructive work; explicit prior successful saves remain separately accounted for.
- Test the actual native macOS termination route with two dirty windows: approve one, cancel the other and assert both survive. Repeat for failed/uncertain Save and a changed prepared draft. Require a functioning pre-termination integration before enabling editing there; synthetic callbacks alone do not prove it.
- For navigation, introduce draft-only tokens/declarations, remove saved matches, insert lines before a symbol, navigate from A to dirty B and edit B after results arrive. Verify list, preview, usages/peek/multiple definitions and activation agree on captured/current snapshots without changing disk.
- Insert/delete/undo with blame visible and assert old attribution/actions disappear immediately, pending state is visible, and recomputed annotations match current text. Exercise every external-open choice and assert exact launch count and saved-coordinate policy.
- Test the same buffer/history through ordinary tabs and last-save/HEAD/external-change comparisons. Verify baseline labels, blue/green/deletion markers, unsaved versus Git status, Save/undo/HEAD refresh, staged/new/untracked/conflicted/deleted cases and reversion with an unchanged Git index.
- Exercise real GUI input methods and terminal paste, non-BMP/combining text, multi-cursor coherence, snippet placeholders, keymap conflicts and all modal routes. Confirm underlying drafts/disk stay unchanged when a dialog owns input. Check light/dark, non-color, narrow windows, tiny terminals, resizing and End followed by Up.
- Validate completion/import/format/quick-fix transactions against current versions and reject obsolete or cross-file provider edits in full. Verify popup dismissal remains effective when a late response arrives. Test user operations through the shared command boundary, with real provider protocol checks for advertised capabilities.
- Use the existing real-language-provider fixture approach to verify every claimed capability/environment independently, including mixed Java/Kotlin projects, Rails, React/TSX, YAML schemas and the configured Python/shell tools. A missing tool or empty response is explicitly unverified or unsupported rather than a passing feature test.
- For ERB/Haml, assert exact original-source replacement and diagnostic ranges through multiline Ruby, interpolation, attributes, indentation, CRLF and non-BMP characters. Apply a candidate edit and verify surrounding bytes remain unchanged. Reject generated scaffolding and foreign-file edits, exercise unsaved changes and obsolete replies, and test markup edit syntax rather than only candidate labels.
- For each SQL dialect, pin expected query-local names/types, diagnostic ranges and formatted output for aliases, CTEs, subqueries, quoted/qualified names, functions, incomplete input and dialect syntax. Test persistence/unset/override behavior and cancellation after switching dialect; these remain requirements even where the current candidates fail.
- Run SQL first without metadata and verify no attempted external database connection. Then explicitly enable disposable metadata sources and verify what schema-dependent behavior changes. Observe actual database requests using server/CLI traces; pass DROP, UPDATE, export and quote-escape documents as data, assert zero edited-statement/EXPLAIN submissions, unchanged fixture rows and absent export sentinels.
- Test metadata refresh, stale catalog results, read-only access, unavailable sources and private credential handling. Fixture setup/verification operations are labelled separately from assistance. Resource cleanup may act only on successfully acquired resources, using stable returned identity rather than an unowned reusable name.
- Validate recovery/restore and explicit discard using private disposable state; original files must not be written by recovery. Exercise explorer moves/deletion with open/dirty/shared views and verify identities, navigation, watches and provider state remain coherent.
- Reuse the existing release GUI/PTY collectors and preserve their endpoint definitions. Compare equivalent before/after startup, interaction and peak/settled memory on the same machine/configuration; record child-provider resources separately and retain raw samples, identities and actual duration. Report skipped platforms and distinguish RSS from GPU memory or leak proof.
- At minimum exercise the readiness corpus of 2,000-line Rust, 100,000-line text and a 900,001-byte Unicode line. Run sustained editing, distant edits, paste, Save/undo, split/comparison, active analysis and stale-result stress for at least 180 seconds per candidate workload. Verify exact text alongside timing and measure sustained retained-history growth separately from repeated undo-to-original cycles.
- Check long-line editing/hit-testing/IME beyond character 10,000, bounded source windows, stale layout rejection and cancellation during analysis. Test-only instrumentation may establish no input/render full-document conversion and bounded admitted work; functional tests must still verify the visible behavior and exact source bytes.
- Use the repository’s full local gate, existing core/GUI/TUI/parity suites, writer/Diff/merge regressions and relevant collector tests. There is no hosted test CI to substitute for that gate. GUI automation uses an owned private display and disposable repositories.

## Out of Scope

AI completion; project-wide replace; multi-file symbol rename/refactoring; integrated application execution, test running and debugging; Git staging/committing/pushing; automatic language-tool installation/updating; direct editing of rendered Markdown; SQL execution, EXPLAIN and result grids.

These exclusions do not remove ordinary filesystem rename/move, existing Diff/repository/pull-request/merge workflows, or any language/template/SQL assistance expressly required above. Features deferred from the foundation milestone remain in scope for the full editor release.

## Further Notes

Readiness work is complete; production editor implementation is not. Baseline measurements, storage experiments, writer fault outcomes, real installed template/SQL probes and a concrete foundation implementation plan have been recorded. The full local gate passed after independent review corrections. This spec synthesizes those agreed requirements and decisions; it does not claim that the editor or all provider capabilities already work.

The readiness results select indexed persistent chunks as a direction, not a general data-structure performance winner. The compared gap candidate lacked line indexes, chunks had higher process memory and slower small-file distant edits, and downstream materialization still costs time. Added UTF-16 indexing, long-line rendering and production behavior require their own measurements. No deliberate performance tradeoff has been approved.

Template evidence demonstrated safe mapping for explicit Ruby projections, but native ERB returned an unsafe replacement range, signature behavior varied with encoding, and complete automatic template/Rails/Haml support remains unproven. DuckDB provides useful functions/parser/formatter/catalog operations but failed required query-local completion cases; PostgreSQL catalog access passed while a complete language-provider route remains unresolved. SQL dialect-switch cancellation is unverified. These are full-release requirements to resolve, not permission to reduce the feature to highlighting or generic suggestions.

The foundation Save plan now requires captured target file identity in addition to content/path validation. The native macOS termination integration is a known enablement prerequisite. Production editing/recovery/performance, native macOS execution and physical display latency remain unverified until their implementation and acceptance checks run.

Treat this as the complete feature spec with sequenced delivery, starting with the foundation. This local tracker spec is published with `Status: ready-for-agent`. That label indicates the agreed work can be taken forward; it does not certify that all later provider/platform prerequisites are already solved.
