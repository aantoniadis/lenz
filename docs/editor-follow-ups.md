# Editor follow-ups

## Integrated editor Linux review — 2026-10-27

The [end-of-epic Linux report](research/code-integrated-editor-linux-validation.md) records the fresh full gate, available real providers, native SQL/X11/PTY boundaries and the completed Linux release performance/stress campaign. Native macOS tickets 03/24 and full ticket 68 remain open; Wayland remains gated. Fresh Kotlin was unavailable from the official download endpoint in this environment, so its earlier ticket evidence is retained separately.

The completed campaign retains two absolute-budget failures and an unexplained five-second GUI Undo paint-feedback timeout. Subsequent unchanged repetitions passed; they do not explain the original timeout. Trace recurrence and preserve the original limits; see the report for baseline comparisons.

The review repaired diagnostic-cache address reuse and stale publication ordering, pending root/repository lookup identity, a Git-status panic leaving its single-flight flag set, queued recovery discard overtaking a newer Save, marker baselines retained outside the inactive-document cache, and watcher registration arriving after file load. The final semantic owner also stops remaining owned servers before exit, with launch/stop ownership serialized. Home/End now moves the caret in both adapters; modified document bounds and Shift selection share core motions.

The old failure rates below describe the pre-review source. The late-completion fixture now waits for a request actually admitted to the wire before testing its late cancellation; unsent current-version work can legitimately be cancelled, covered separately in the core. Save As uses worker barriers to prove that the write is still pending at its pre-confirmation assertion. SQL settings waits for the persisted value, and the recovery restart fixture waits for initial workspace scanning before its flush barrier. No timeout was raised. These preserve the safety assertions and passed targeted repeats and the final full gate.

Both known watcher gaps were reproduced with deterministic held jobs/notifications, then fixed; the actual TUI followed-rename replay passed 128 runs at concurrency eight. Save As and recovery restart each passed 128 repeated runs at concurrency eight after their fixture barriers; SQL settings passed 100 serial runs. Late completion passed 80 repeats. Template parity, semantic admission/request retirement and GUI outline each passed 40 targeted runs; the original timeout in the latter cases was not independently reproduced, so those figures are evidence, not a root-cause claim. Both old Diff Markdown cases passed 40 repeats each; their original causes remain unproven. Fourth-click replay now waits for its intervening render; the older failure did not reproduce in 640 isolated attempts. Preserve recurrence diagnostics rather than increasing deadlines.

The Linux freed-memory fixture now tests a touched large allocation, avoiding a false requirement that glibc release small freed pages immediately; the macOS fixture is unchanged. Collector cleanup pauses its own timed-out group before slow descendant discovery, with a deterministic regression and cooperative-SIGINT cleanup proof. Successful measurement boundaries remain unchanged.

Completed costs and passing full-duration editing stress are quantified in the report; remaining performance follow-ups and the full-campaign cost decision are pending. That evidence does not silently accept a deliberate tradeoff or expand provider inference/template capability beyond the guide. Physical latency, native macOS/Wayland and hardware GPU memory remain unverified. Two Gradle fixtures required cleanup of a separate Gradle daemon; the captured commands distinguish it from the language server.


Record findings here instead of expanding the current editor implementation. Deal
with these after the approved work is done. A finding that blocks an approved
ticket's acceptance remains in that ticket; this file does not waive its checks.

## Deferred observations

- **TUI overhead:** the final snapshot-window comparison measured startup RSS
  +0.592 MiB and completed Go To median +0.085 ms, with substantial timing
  variation. Settled RSS decreased. The user accepted the documented batch costs
  on 2026-10-24; retain this as a later optimization opportunity.
  Evidence: `docs/research/code-snapshot-tui-performance.md` on the TUI branch
  and `target/perf/editor-implementation/09/native-final-summary.json`.
- **Find before results arrive:** `Session::find_step` currently returns without
  navigating while the match list is empty. Check whether Enter pressed during
  the initial asynchronous search should be remembered and applied when results
  arrive. This was found while preparing the native reveal regression.
- **Find query selection:** inspect Ctrl+A while the GUI Find field is focused.
  `ReaderView::find_key` currently lets this shortcut fall through to the parent
  handler. Verify the intended behavior before changing it; this is an observed
  routing question, not yet a demonstrated defect.
- **Additional native shaping coverage:** extend owned-display checks to more
  fonts and Arabic/Indic shaping at projection boundaries. The current native
  diagnostic covers six bounded samples; TestPlatform metrics differ from the
  native backend. Do not claim the broader coverage already exists.
- **Pathological first-content measurement:** measure native first feedback and
  completed layout separately for very long combining-mark runs. The retained
  window leak has a regression check; equivalent native startup measurements for
  that specific input remain a follow-up.
- **Pinned benchmark helper selection:** fake-provider fixture setup currently
  invokes Cargo before locating its helper beside the test executable. A future
  explicit prebuilt-helper option would make immutable benchmark archives easier
  to replay and prevent accidental compilation inside quiet runs. The current
  measurements retain the expected directory layout and use `CARGO=/usr/bin/true`.
- **Presentation work during native reveal:** review whether automatically issued
  commands such as `ColourResults` should supersede a pending native reveal. The
  cancellation classifier currently exempts a small set of adapter feedback
  commands. Reproduce semantic-result navigation with fresh result coloring before
  treating this as a defect; do not start a separate investigation now.
- **Unwrapped shift-arrow column policy:** ordinary Up/Down again preserves shared
  display columns across Unicode, expanded tabs and hints. The pre-existing
  unwrapped `Extend` path retains source-byte columns. Evaluate that separate
  selection policy before making shift-arrow behavior match ordinary navigation;
  it was not changed by the integration gate repair.
- **Local gate build concurrency:** the combined 08–10 gate launched many
  simultaneous linkers and showed substantial host I/O pressure while building
  workspace test binaries. Compare a bounded Cargo job count on a later cold
  build before changing the gate; this is an operational observation, not a
  demonstrated product regression. Evidence: the batch gate log under
  `target/perf/editor-implementation/merge-08-10/`.
- **Draft-aware blame presentation:** ticket18 now invalidates old annotations and
  commit actions on draft edits and recomputes against admitted current bytes.
  Focused core and opt-in GUI/TUI input/render traces cover insertion, newline
  deletion, Undo and failure. Final acceptance and performance remain pending the
  11/12/13/16/18 batch; production typing remains gated for21/22.
  Legacy drafts containing characters their original encoding cannot represent,
  and lossy-decoded sources, show explicit Failed attribution with commit actions
  disabled. Representable legacy bytes retain exact attribution; this boundary
  follows the strict writer encoder rather than substituting invented bytes.
- **Typing-group allocation:** compact history currently joins inserted/deleted
  group text when adjacent edits coalesce. This copies the group's changed text,
  not the document. Use the scheduled batch measurements before deciding whether
  mutable group-string capacity is worthwhile; no extra benchmark run is needed
  per ticket.

- **Many-tab dirty indicator cost:** `State::tab_dirty(TabId)` currently locates
  the tab by scanning the tab collection. Header painting now queries dirty state
  for each tab; include many-tab counts in the planned batch measurements before
  deciding whether to expose a borrowed-tab or iterator form. The draft dirty
  comparison itself is constant time; no text is copied by this lookup.

- **Clock-dependent blame fixtures:** the focused18 run crossed UTC midnight and
  exposed a status assertion comparing the commit's creation date with a later
  `today()` call. Preserve the exact attribution assertion and rerun that single
  case with a fresh fixture for18; deterministic fixture dates remain deferred.
  Evidence: `target/perf/editor-implementation/18-blame/core-final.log`.

- **Inactive-tab Save feedback:** buffer-owned rejection/uncertainty and recheck
  state survive navigation; activating the tab reveals its full shared status.
  Headers currently expose its dirty marker. Consider a persistent inactive-tab
  save-state badge or actionable session notification, using the same core
  state rather than frontend-owned errors. This is presentation followup;
  draft/history retention and Save blocking remain enforced while inactive.
- **Compact-index query batching:** declaration and modal symbol continuations
  currently rank up to 32 adjacent saved-file record sets per turn. Use the
  scheduled batch comparison on a many-file index before changing that batch
  size; preserve independent query cancellation and completed-action timing.
- **Intermittent GUI declaration tracer timeout:** the final batch GUI run at
  `bb2f55f4` timed out once in `outline_and_declaration_search_through_keys`.
  The unchanged test then passed in isolation, in 25 consecutive isolated
  reproductions, and in a complete GUI confirmation run. No cause or runtime fix
  is claimed. Add stage-specific failure diagnostics or a controlled scheduler
  reproduction if it recurs; preserve the keyboard, selection and activation
  assertions. Original failure and all attempts are retained under
  `target/perf/editor-implementation/merge-11-18/code-gui-*.log`.
- **Ticket14 native delivery acceptance:** production Reader windows install a
  veto, and focused tests replay that exact handler plus real keyboard decisions.
  GPUI TestPlatform has no native close-event simulator; replay one ordinary Linux
  titlebar close via an owned display at the next adapter batch boundary with
  disposable fixtures using the existing test-support draft gate. Native macOS
  termination remains ticket03/24, unverified.


## Reader batch performance attribution (2026-10-24)

- Distinguish live admitted backing from anonymous allocator residency after large-file Find/reload. The 11–18
  campaign retains candidate TUI 37.59/46.10–46.42MiB residency bands; phase samples put most of the upper-band
  difference in anonymous memory. Current source admission invalidates old analysis scope, and no leak or
  exclusive allocator cause has been proved. Do not average away the upper-band cost. Evidence and attribution
  limits are in the [batch performance report](research/code-editor-batch-11-18-performance.md).

- Deferred optimization candidate, not implemented: unchanged snapshots created by `Chunk::shared` retain the
  original full text/index Arcs behind their chunk ranges. Investigate sharing that backing during admitted
  `materialize_while` conversion instead of String→Arc flattening when complete contiguous coverage is verified.
  Large originals span several ranges; do not assume a single full-range chunk or apply this to arbitrary edited
  snapshots. Preserve source/version fences, worker admission, cancellation and edited/chunked fallback behavior.
  This may reduce conversion allocation; it is not a measured explanation or guaranteed remedy for the RSS bands.

- **Disk-only hierarchy request inputs:** ticket 17 fences reusable items against
  exact open-draft identities and rechecks returned targets with captured disk
  identity. A disk-only item changed before a later hierarchy request has no new
  asynchronous input-validation stage. Preserve that limitation when expanding
  hierarchy support; do not treat a later reread as proof of old coordinates.

## Reader saved comparison (2026-10-24)

- The shared line-diff engine has no internal cancellation checkpoint. A superseded comparison cannot publish,
  but its running job retains its executor credit until the engine returns. Evaluate finer cancellation if
  pathological comparisons show interference in the next batch; do not create a separate frontend worker pool.
  The [comparison report](research/code-saved-comparison.md) records current proof and unverified limits.

- **Composition allocation cost:** ticket22's transient persistent snapshot shares
  source chunks but rebuilds their entry index and copies the pending replacement
  text. Include repeated preedit at several source sizes in the planned batch
  measurements; this is not a constant-time update claim. Keep input/render
  Document-conversion counters separate from these allocations, and preserve the
  single captured-version commit and bounded worker/context guards if optimizing.

- **Native screenshot synchronization:** the foundation driver captures screenshots
  immediately after its state observer reports preedit, without a completed-paint
  acknowledgement. Keep them illustrative alongside the callback/authority/Undo
  assertions. If native visual assertions are added, acknowledge the corresponding
  painted presentation before capturing; a state notification can precede paint.
  The existing TestWindow render checks remain separate evidence.

- **Editor history-growth cost:** the 14–23 batch's matched raw editing windows show
  +0.048–0.319 ms median dispatch at equal cycle/content lengths and 5.8% fewer 180-second
  workload cycles. Profile the additional per-edit work before optimizing; retain
  anchor/source/permission fences and complete Undo/Redo. TUI startup RSS rises
  about 0.676 MiB across eight launches. See the [batch report](research/code-editor-foundation-batch-14-23.md).
  Memory phase differences require equal-history/allocator attribution before any
  efficiency claim; no new stress or optimization is authorized by this note.

- **Editing-preference resolution cost (ticket25):** `Session::editing_preferences_details` walks
  `.editorconfig` files to the filesystem root and probes formatter markers and `PATH` entries on the UI
  thread each time the dialog opens. Menu toggle states read only the remembered project choice. When
  indentation commands (27) or format-on-save (42) consult these preferences per keystroke or Save, cache
  per directory with invalidation or move resolution off the UI thread; measure in the scheduled batch.
  Not measured for this ticket.
- **EditorConfig coverage (ticket25):** `{n..m}` numeric ranges never match, and only `indent_style`,
  `indent_size` and `tab_width` are resolved. `end_of_line`, `charset`, `insert_final_newline` and
  `trim_trailing_whitespace` are not shown or applied; the exact-ending writer already preserves existing
  endings. Indentation is not detected from file contents.
- **Git status refresh cost (ticket65):** one `git status --porcelain=v2 -z --untracked-files=normal` runs per
  watcher burst under the repository, per index/HEAD-log time change seen on the 5 s tick, and per repository
  lookup; changes during a read coalesce into one rerun. A superseded read stops git between entries; at most
  `STATUS_LIMIT` (20,000) entries are kept and git is stopped beyond it. Suspected costs: a large worktree's
  `git status` on every save burst, and the per-row `row_dirty` tab scan while painting the explorer.
  Staging/commits from a terminal appear only on the next tick (up to 5 s). Any burst under the repository
  triggers a read, including the reader's own state/cache writes when the open folder contains them (a home
  or dotfiles repository). A failed read clears every badge until the next trigger, and a panicking read would
  leave reads stopped until the repository changes. Not measured for this ticket.
- **Git status presentation gaps (ticket65):** folders do not aggregate their children's statuses; paths are
  matched as `/`-separated (Windows separators unhandled); git's stderr is read only after its output ends.
  A restored draft marks its tab as recovered (`⟲`, ticket 32); explorer rows still show only their own dirty state
  and Git status, so a recovered file's row has no recovered badge.

## New File and Save As (ticket 31, 2026-10-24)

- **Suspected cost, unmeasured:** Save As validates its destination with `canonicalize`, `symlink_metadata`
  and `metadata` on the UI thread before submitting the write (cheap local stats, but blocking on slow or
  network filesystems). The write itself uses the existing write worker. Include a Save As interaction in the
  scheduled batch measurements before moving validation off the UI thread.
- **Suspected cost, unmeasured:** `Tab::untitled()` checks the tab's source prefix and now participates in
  the repeated `is_file()` filters (watch set, workspace checkpoint, LSP sync, places). This is per-tab string
  prefix work; measure with the many-tab case before caching it as a field.
- **Close prompt Save for untitled:** the dirty-close dialog keeps Save disabled with "Use Save As…"; the user
  cancels, uses Save As, then closes. A chained "Save As then close" continuation is not implemented.
- **Native GUI save dialog overwrite prompt:** some platform save dialogs ask about overwriting an existing
  file themselves; Lenz still requires its own Replace decision. Consider passing the platform's answer only
  if the dialog's confirmation can be verified; GPUI does not report it today.
- **Untitled recovery and identity:** untitled documents are not restored by workspace checkpoints and are
  not recovered after a crash (recovery is a later ticket). Their language stays unset until Save As detects
  it from the chosen name.
- **Save As copies mode:** a new file gets the default umask mode, not the original file's mode (an
  executable script saved elsewhere is not executable). Replacing an existing file keeps that file's mode.
- **Terminal Ctrl-Shift-S:** reported only by terminals with extended keyboard protocols; the palette and
  Save on an untitled document remain available everywhere.
- **Save As while uncertain:** Save As stays available while the original file's Save is uncertain, so the
  draft can be written elsewhere; success moves the buffer to the new file and leaves the old file's
  uncertain state behind. Review whether the user should be told explicitly that the old file was not
  rechecked.
- **Observed test flakes (not caused by ticket 31):** `current_cached_rename_reactivation_updates_path_and_preserves_held_snapshot`
  timed out in 2 of 10 isolated runs on the unmodified integration tip `f9c812a` (watcher-driven rename
  detection). `double_and_triple_click_select_word_and_group` failed once under the parallel code-tui run
  and passed 3 of 3 isolated runs (click timing). No cause or fix is claimed.

## Reader code completion (2026-10-24)

- **Per-keystroke completion requests:** while typing a name with the automatic popup closed, each word
  character sends a new `textDocument/completion` (cancelling the previous one) after the edited version is
  admitted for analysis and synchronized. While the popup is open, only `isIncomplete` lists are asked again.
  No debounce exists; measure request/cancel churn and admission cost at the end-of-epic batch before adding one.
- **Popup filtering and drawing cost:** each continuing keystroke re-filters every received item (lower-cased
  subsequence match) and the GUI/TUI clone the shown labels per frame. Lists are provider-bounded, not capped
  by the reader; check large lists (for example rust-analyzer flyimport) in the batch measurements.
- **Server commands on items:** the parameter-hint/suggest client commands are ignored rather than run; other
  commands refuse the item, since a command could edit other files. Snippet bodies are expanded since ticket 30
  ([snippets](research/code-snippets.md)); imports and resolve are applied since ticket 39 (below).
- **Outside-click parity:** the GUI closes the popup on any click outside it; the terminal closes it only when
  the click moves the caret (a toolbar click leaves it open). Decide one rule in the core if this matters.

## Draft recovery (ticket 32, 2026-10-24)

- **Suspected cost, unmeasured:** every change burst materializes the draft and its confirmed baseline on the recovery
  worker and writes them as JSON (up to 8 MiB per record). Replacements release their snapshots at once (one pending
  version per document), so a burst holds at most one extra snapshot per open draft; include the copy and the write
  size in the scheduled batch measurements before changing the 500 ms debounce (`recovery::DEBOUNCE`).
- **Suspected cost, unmeasured:** start reads up to 20 records and 32 MiB of text synchronously in `Session::new` and
  compares each with its file (up to 64 MiB per file) to say whether the disk still matches. Many or large records delay
  the first window. Move the comparison off the UI thread and say so in the dialog if this shows up in startup samples.
- **Size limit:** a draft and baseline together over 8 MiB (roughly a 4 MiB document) are not kept, and the previous
  record for that file is removed rather than offered as older work. Recovering large documents needs a cheaper
  representation than two full texts.
- **One record per path:** two windows editing the same file write the same record; the last writer wins, so only one
  window's draft is recovered. Keying file records by session instead would recover both at the price of duplicates.
- **Write order:** tombstones and records are ordered by wall-clock milliseconds so a tombstone outranks records from an
  earlier run. A clock set backwards could let a record written later have a lower number; a restart after such a change
  should be exercised before relying on this for adversarial cases.
- **Cache lifetime:** records live under `$XDG_CACHE_HOME/lenz-code/recovery`, so a system cache clean removes them.
  They are not durable storage and no UI claims otherwise.
- **Prompt delivery:** the GUI syncs the recovery dialog on construction and on every dispatch; `ReaderView::poll` does
  not sync prompts, so a prompt created while another modal is open appears when that modal closes or the next command
  runs. The TUI syncs it in `clamp()` each frame.
- **Unverified:** native macOS and Wayland editing remain gated; their termination route and the recovery dialog were not
  run there. Wayland GUI and physical terminals were not exercised for this ticket.
- **No content in diagnostics:** records and prompts never carry source text; `Record`/`Version` `Debug` show identity and
  sizes only. Keep it that way when adding fields (for example a diff summary) to the dialog.
- **Observed core-suite flakes (not caused by this ticket):** with draft recovery present, the full `code-core`
  suite also timed out once in `semantic_requests_wait_for_the_current_admitted_snapshot_and_preserve_saved_text`
  (3 of 3 isolated runs passed, and two full runs afterwards passed). The rename watcher test above did the same
  3 times in 6 full runs while passing 5 of 5 isolated runs. No cause or fix is claimed; the recovery worker adds one
  idle thread per session, which is not shown to matter.

## Keymaps (ticket 26, 2026-10-24)

- **Rebinding commands without a chord:** the Keymap dialog lists commands that have a chord or an override, so a
  command that has never had a shortcut (for example Show Commit) can only gain one by hand-editing the settings
  file's `keybindings` (by command slug). Listing all commands needs a filter in the GUI dialog and a decision about
  making 140 rows navigable; not implemented here.
- **Editing keys are not rebindable:** Copy, Cut, Paste, Delete, Select All, Undo and Redo stay the fixed keys the
  adapters map as text input (`CommandId::input_hint`), because they run before the keymap in both adapters. A
  keymap that drives them would have to keep the draft-context guards (a preview must not be undone through the
  keymap path).
- **Terminal delivery is a static guess:** `Chord::terminal_blocked` marks Ctrl-Q/Ctrl-S (flow control),
  Ctrl-H/M/I (Backspace/Enter/Tab) and Ctrl+Shift+letter (enhanced protocol) as unreliable. It cannot see the actual
  terminal, keyboard protocol or multiplexer, so the TUI only warns on a captured chord and hides nothing; the
  GUI ignores the notion because it delivers every chord.
- **Preset shadowing only appears on a preset change:** `Keymap::bind` refuses a chord another command runs, so
  overrides cannot displace preset bindings while rebinding. Switching to the other preset can still leave an
  override holding a chord the new preset gives another command; the switch reports it and the displaced command
  keeps its menu entry.
- **Slugs follow variant names:** remembered overrides are keyed by a slug derived from `CommandId`'s Rust variant
  name (`SearchEverywhere` → `search_everywhere`). Renaming a variant drops that command's override on the next load
  (silently, like an unknown key); there is no migration or warning yet.
- **Per-keystroke resolution:** `Session::key_command` scans the resolved binding list (about 50 chords) with string
  comparisons on every key. Expected to be negligible, but it is new per-keystroke work in both adapters; include
  fast typing in the scheduled measurements rather than assuming it.
- **Dialog cost:** the Keymap dialog rebuilds its rows (scanning `CommandId::ALL` for the bound ones, then formatting
  labels, chords and notes) each time it is drawn or a key moves the selection. The list is bounded and small; no
  caching was added.
- **Observed test flakes (not caused by ticket 26):** the full `code-tui` suite failed
  `double_and_triple_click_select_word_and_group` once and the full `code-gui` suite failed
  `outline_and_declaration_search_through_keys` once under parallel load; both passed 3 of 3 isolated runs, matching
  the earlier records above. No cause or fix is claimed.
- **Keymap overrides on another platform:** VS Code's macOS chords (for example Cmd-based ones) are reported through
  the same logical Ctrl chords the adapters already normalize; native macOS and Wayland GUI editing remain gated and
  unverified.

## SQL dialect (ticket 60, 2026-10-24)

- **No assistance is wired yet:** the core already compiles the offline PostgreSQL and DuckDB modules
  (`postgres_assistance`, `duckdb_assistance`), and the SQL Dialect details say the editor does not run them yet.
  Tickets 61/62 must capture `Session::sql_identity()` before a request and publish only while
  `sql_identity_current` accepts it; this ticket tests the fence (dialect switch, project change, file switch and
  edit all reject an earlier identity) but no real provider reply exercises it, so SQL dialect-switch cancellation
  remains unverified. Update the details wording when assistance lands.
- **"No metadata connection" is structural, not yet observed:** choosing a dialect only writes the Reader settings
  file and no metadata path exists, so the focused tests assert no document/project write, no project-directory
  change and no assistance run. There is no server/CLI trace to observe because nothing connects; ticket 63 must
  add the real boundary observation.
- **Suspected cost, unmeasured:** each settings save serializes the `sql_dialects` path→dialect map. Every file with
  its own choice adds an entry, and there is no eviction or pruning, so the map can grow across years of standalone
  files. Measure settings write size and load time only if it becomes visible; no benchmark run was done for this
  ticket.
- **Rename/move leaves stale entries:** file overrides are keyed by the tab's absolute source path. A rename or move
  (ticket 36) does not yet carry or drop the old path's entry, so the moved file falls back to the project default
  and the old key lingers in settings.
- **Untitled and non-file views:** an untitled document cannot hold a per-file dialect until Save As gives it a path;
  comparison sides and server-retrieved sources report "Only an opened file has a SQL dialect". A file whose
  extension is not detected as SQL (for example `.ddl`) can still be given a dialect, but the status bar hides the
  label for it; the details dialog and menus still show it. Decide whether status should follow the recorded choice
  rather than the detected language.
- **No preset chords:** the SQL commands are menu/Find Action/palette only. Adding IntelliJ or VS Code chords needs
  conflict decisions (most Ctrl/Alt letters are taken) and a terminal-deliverability check.
- **Narrow layouts:** the SQL label shares the footer with the LSP label, position and branch and can be ellipsized
  below 900 px in the GUI and at small terminal widths. The details dialog remains reachable from the label, the
  palette and the SQL menu; verify the exact cut points if the footer changes.

## Find and replace (ticket 28, 2026-10-24)

- **No preset chord for Replace:** `ReplaceMatch` and `ReplaceAll` are in the Find menu, the command list (F10 /
  Find Action) and both find UIs, but neither keymap preset binds them. IntelliJ gives Ctrl+R to Replace (Reload
  here) and Ctrl+H to Type Hierarchy; VS Code's Ctrl+H is `terminal_blocked` (terminals send Backspace) and a global
  Replace would run with an empty replacement text, deleting the current match. The Keymap dialog only lists
  commands that have a chord or an override, so giving Replace a chord needs the hand-edited settings file or the
  "rebinding commands without a chord" follow-up above.
- **Empty replacement deletes:** the replacement text is literal and may be empty, which deletes each match. This is
  intended (replacing text with nothing) but a first-time user pressing Replace before typing a replacement removes
  the current match; the box shows an empty `Replace` row, and Undo takes it back.
- **Literal replacement only:** regular-expression groups, `$1`, case modifiers and escape sequences are not
  expanded; the replacement text is inserted byte for byte.
- **Replace waits for the search:** each edit restarts the find, so the next Replace is refused ("Wait for the search
  to finish") until the new matches arrive. Repeated replace-current therefore depends on the analysis worker's
  turnaround rather than being a single synchronous walk; include a repeated-replace walk in the scheduled batch
  measurements before changing it.
- **Capped searches:** at 10,000 matches Replace All refuses rather than replacing a prefix; Replace still changes
  the current one. The list is capped before replace, so a document with more matches needs a narrower query.
- **Replacement containing the query:** a replacement that contains the searched text is found again, including
  after wrapping, so repeated Replace can keep matching text it just wrote. This follows the wrapped search the
  find box already had; no special skip was added.
- **Line endings:** matches never cross a line break, so replacing a query that only matches a line's text leaves
  that line's ending behind (deleting "two" leaves the empty line). Matching a trailing newline is not supported.
- **Session-only state:** the replacement text is remembered like the find options while the reader runs and is not
  saved to settings or the workspace, so it is empty after a restart. Decide whether it should join the workspace
  checkpoint.
- **Suspected cost, unmeasured:** Replace All builds one text edit per listed match (at most 10,000), each carrying
  the replacement text, in one transaction; a large replacement on a large document therefore allocates edits and
  the resulting snapshot at once. Measure it before changing the cap or streaming the edits.
- **Suspected cost, unmeasured:** the GUI's find box is now two rows tall whenever the pane has room, so it covers
  more text and a click that used to reach the text can land on the replacement row. Both adapters rebuild the box
  or prompt text each frame; check narrow panes and clicks near the box in the batch measurements.
- **Several cursors:** Replace All carries every caret through the one transaction (a caret inside a replaced match
  moves to the end of its replacement). Replace current re-selects the next match and drops the other cursors, as a
  find step does; if that is unwanted, fanning replace across selections needs its own command.
- **Unverified:** native macOS and Wayland GUI editing remain gated, so their find-box clicks and key delivery were
  not exercised. Physical terminals were not driven for this ticket; the TUI paths were replayed with Ratatui events
  and the cross-adapter parity test, not a real terminal emulator.

## Several selections (ticket 29, 2026-10-24)

- **Only one range shape can be created:** Add Cursor Above/Below adds carets at the same column of neighbouring
  lines, and Select Next Occurrence adds matches of the primary selection's text. There is no command that adds a
  selection of a different length (or extends one cursor into a selection), so differing-length selections exist in
  the model and in editing only after an edit collapses them; a shift-click or a second cursor's own extension
  would need its own command. Until then the model's merge rules for arbitrary overlaps are exercised through
  occurrence selection and cursor absorption.
- **Caret motions act on the primary selection only:** arrows, Home/End, Extend/Shrink Selection, clicks and jumps
  collapse the set (the same rule as Escape), and Up/Down ignore the other carets. Moving every caret, and extending
  every selection with Shift, is the natural next step; it needs per-caret layout queries for wrapped lines.
- **Typing merges per caret, deletions do not:** repeated typing at several carets joins one Undo step
  (`Draft::merge_carets`), as one caret's typing does. Repeated Delete/Backspace at several carets is one transaction
  per keystroke, unlike the joined single-caret deletion; joining it needs the same per-caret offset bookkeeping in
  the other direction. Paste, Cut and every explicit edit end the group, as before.
- **Input-method composition is single-cursor:** the GUI's composition capture places a pending replacement at the
  primary selection, and its commit is one explicit-range edit, so a composed character changes only that place while
  the other carets shift with it. Completion acceptance is likewise one explicit edit. Decide whether composition
  should fan out or drop the extra carets before claiming parity for IME with several cursors.
- **Multi-cursor chords in the VS Code preset:** VS Code's Ctrl+D selects the next occurrence, but it is Compare
  Selected here, so Select Next Occurrence keeps Alt+J in both presets instead of displacing an existing action; the
  add-cursor commands take VS Code's Ctrl+Alt+Down/Up and keep Alt+Shift+Down/Up as aliases so the documented keys
  work under either preset. Revisit if ticket 27 wants Alt+Shift+Down for duplicate line.
- **Select Next Occurrence cost, unmeasured:** each press scans the draft one bounded 64 KiB window at a time
  (`selection::find_from`) from the primary selection to the end and then from the start, so a large file is never
  assembled whole but the scan is linear in the file per press. It is an explicit user action, like Find; include a
  large-file occurrence walk in the scheduled batch measurements before optimizing (for example, reusing the find
  match list).
- **Several cursors are not persisted:** only the primary selection is saved in the workspace and history, so
  reopening a project, Back/Forward and a restored view keep one cursor. A split pane copies the set when the view is
  duplicated, then follows its own.
- **Terminal modifier delivery:** the add-cursor chords (Alt+Shift+Down/Up in both presets, plus VS Code's
  Ctrl+Alt+Down/Up) reach Lenz only from terminals that report modified keys (the enhanced keyboard protocol).
  Otherwise the terminal sends another key; the palette and Selection menu keep both commands reachable, the in-app
  help says so, and **Keymap…** can move them. Collapse to Single Cursor stays on Escape as a fixed editing key
  (`CommandId::input_hint`), so it is not rebindable, like Delete and Undo. Native macOS GUI and Wayland GUI editing
  remain gated by ticket 23/24; the multi-cursor behaviour was verified on the GPUI test platform and the Linux
  terminal only.
- **Rendering cost:** the GUI builds one extra selection list per visible row (`projected_row`) and the terminal
  checks each cell against every selection. Both are proportional to the visible rows times the carets, not the
  document; measure the many-caret case (for example 50 carets) in the batch before caching anything.
## Explorer creation (ticket 35, 2026-10-24)

- **Suspected cost, unmeasured:** Enter in either name prompt checks the folder and the target with `symlink_metadata`
  and `metadata` on the UI thread, and a new file is then written there synchronously with the exact writer
  (`create_exact`: a temp file, `fsync`, a no-clobber rename and a directory sync) before it is opened. Cheap locally,
  blocking on slow or network file systems. Include a create in the scheduled batch measurements before moving the
  write to a worker; Save As already submits its write to the write worker.
- **Only empty folders this session made:** the project inventory is built from files, so a folder that holds no
  file is kept in the tree only when this session created it (`Project::add_folder`), and it is dropped when a later
  full rescan finds it gone. A folder made elsewhere (`mkdir` in a terminal, another program) appears once it holds
  an inventoried file. The knowledge is per session: after a restart an empty folder is not listed until it holds a
  file, which also means it cannot be highlighted as a create target.
- **Ignored folders:** a file created inside an ignored folder (for example `build/`) is added to the inventory and
  tree at once, even though a rescan will not list it; the entry then disappears from the tree while the file stays.
  Refusing creation in ignored folders would need the ignore matcher in the session, which it does not hold.
- **Race after the check:** the prompt validates the name when Enter is pressed and the session validates it again
  immediately before creating, so an entry that appeared meanwhile is never replaced (the exact writer refuses it).
  That refusal, and an I/O failure the mode bits did not predict, close the prompt and are reported in the status
  bar rather than in the dialog: the arm's error field is only used for the check at Enter.
- **Read-only detection:** the folder is called read-only when its own mode bits allow nobody to write; a read-only
  mount, a full disk or an ACL that denies the user is not predicted and fails at the create instead. Only Unix
  permissions are inspected.
- **Names:** one path segment, no `\` or `/`, no control characters, at most 255 bytes, not `.` or `..`. Nothing else
  is refused, so a name the file system rejects for another reason (for example a Windows reserved name, if that
  target ever runs) is reported by the failed create. Trailing spaces and dots are not trimmed.
- **Reveal does not scroll to the row:** the session asks both adapters to select the created entry
  (`State::tree_reveal`), which sets the highlight and opens the explorer. The GUI only clamps its scroll to the entry
  and the terminal keeps its top row, so an entry below the visible rows is selected but not scrolled into view.
- **Finder and dependent views:** a created file is added to the inventory and the finder's results are marked stale
  rather than re-ranked, so it can be missing from an open file finder until the query changes. The declaration
  index and Git status pick it up on their next refresh; a created file is untracked until git sees it.
- **Unverified:** native macOS and Wayland GUI creation were not run (editing remains gated there); the GUI checks
  ran on the GPUI test platform and the terminal checks in a test backend, so physical terminal layouts and real
  desktops are unverified for this ticket.
## Draft change markers (ticket 66, 2026-10-24)

- **Suspected cost, unmeasured:** a settled draft is compared with its baseline in one background pass over the whole
  document (`lenz_engine::compare_lines` allocates its prepared line tables each time), and a dirty draft also
  materializes once through the shared analysis conversion. Typing stops at the three-hundred-millisecond rest, so a
  burst costs one comparison, but a user who pauses often on a 100,000-line file compares that file on every pause.
  Measure the many-pause case on the readiness corpus before deciding whether to reuse the previous blocks
  incrementally (mapping them through the edit ranges and re-diffing only the touched region) or to reduce marker
  coverage for large files; no benchmark was run for this ticket. The diff engine still has no internal cancellation
  checkpoint (see the saved-comparison note): a superseded comparison stops publishing but runs to its end.
- **Markers lag the text while catching up:** the last published blocks stay drawn while a newer version is compared,
  so a line inserted or deleted during that fraction of a second can show a marker one line off until the newer result
  arrives. Adapters label the state (`updating…` in Status details) but do not transform the blocks through the edit;
  the alternative — clearing them on every change — was judged worse than a brief imprecision. Revisit with the
  incremental idea above.
- **Pairing inside a replaced block:** additions are recognised inside a both-sided block only by pairing its lines by
  place (`n` lines against `m`), so a line inserted at the start of a rewritten run is marked **M**, not **A**. A
  membership-based or LCS-based split would need a per-line rule that also explains a genuinely deleted line; keep the
  current conservative letter until a real example asks for more.
- **Ignored-but-untracked files:** Git's status does not list ignored files, so a file inside `.gitignore` is read as
  `new file (not in HEAD)` (it has no HEAD blob and no status). It is never shown as modified; whether an ignored file
  should instead show no markers at all is a presentation question.
- **Files of another repository:** the markers use the session's repository (the project's, else the open file's
  folder), as ticket 65's statuses do. A file from a different repository while a project is open therefore compares
  with its last saved version and is labelled `last saved`, not with that other repository's HEAD. Same limit as the
  explorer badges.
- **Where the markers sit:** the terminal reserves two extra gutter cells for every text file (whether or not it has a
  baseline), so opening a file does not shift its text when the first comparison arrives; that costs two of the
  terminal's columns on every row. The GUI keeps its gutter width — widening it re-wraps every wrapped row, which
  perturbs click mapping and wrapping tests and costs the text two cells — and draws the letters in the gutter's left
  margin: `M`/`A` at the left edge and `D` one cell right of it, with bookmarks a cell further right. On a file of
  five digits or more the right-aligned number reaches into that margin, so a letter can sit over the number's first
  digit (bookmarks already did). A gutter that grows with the markers' column, as the terminal's does, would fix it;
  measure the text-width cost and re-baseline the wrap and click tests before changing it.
- **One more thread per session:** the rest before a comparison is timed by a `reader-markers` thread per session,
  like the tick's and the places saver's. It only waits and wakes; a session with no draft text still holds it until
  the session drops.
- **Unreadable statuses leave markers pending:** when `git status` fails for the session's repository, `State::git_status`
  stays empty, which the markers cannot tell apart from "still loading": the file shows no letters and no reason. A
  failed read could carry its own state (as the blame column does) if this turns out to matter.
- **Unverified:** native macOS and Wayland GUI drawing were not run (editing remains gated there); the GUI checks ran
  on the GPUI test platform and the terminal checks in a test backend. The marker look with exotic fonts, at very
  narrow widths and in physical terminals is unverified, and the marker column's cost per frame was not measured
  (the lookup is a binary search per drawn line).

## Structural edits (ticket 27, 2026-10-24)

- **Line endings stay at their position, not with the line:** moving a line rotates only line *contents*, so every
  terminator stays where it was written. That keeps CRLF, LF, mixed endings and a missing final newline byte-exact
  and never invents a separator, but in a mixed-ending file the moved line takes the terminator of the position it
  lands in instead of carrying its own. Duplicating a block that ends the file without a newline takes the previous
  line's terminator as the separator. Decide whether a per-line terminator model (with an explicit last-line rule)
  is worth the extra cases if mixed-ending moves are reported as surprising.
- **Comments do not parse strings or nesting:** comment toggling uses the language's prefix or block pair and the
  lines' own indentation only, so a selection inside a multi-line string is commented like code, an existing
  `/* … */` is not nested, and a line already commented with a different prefix (`#` in a `//` language) is
  completed rather than recognized. Using the tab's parsed syntax to skip strings would need the parse to be
  current for the edited version.
- **Pairs are character rules, not syntax:** `(`, `[`, `{` and the three quotes pair before whitespace, a closer,
  `;`, `,`, `.` or `:`; a quote after a word stays an apostrophe; Markdown, YAML, TOML and plain text never pair.
  No string/comment context is consulted, so typing `(` inside a comment also opens its pair, and an already
  balanced line can gain a pair when the caret sits before a closer of another kind. Escape does not "unpair" a
  pair typed together.
- **Ctrl+/ in terminals:** many terminals send Ctrl+/ as `0x1F`, which Lenz reads as Ctrl+Shift+- (Collapse All).
  Toggle Comment also has Alt+/ in both presets and stays in the Selection menu and F10; no terminal test could
  make a real emulator deliver Ctrl+/ (see the "unverified" note).
- **Structural edits create no new selection shapes:** a block is the union of the lines the existing selections
  touch, so several blocks move or duplicate only when several non-adjacent selections already exist (adjacent ones
  merge). The ticket-29 limits on creating selections still apply.
- **Suspected cost, unmeasured:** indentation resolves `.editorconfig` files to the filesystem root on every Tab,
  Shift+Tab and indent command (ticket 25's note). That is an explicit action rather than a keystroke, and typing
  delimiters never resolves it, so no cache was added—changing `.editorconfig` applies at once. Comment toggling
  copies each touched line and move/duplicate build one span string per block; neither materializes the document.
  Include a repeated-Tab walk in an editor batch if it becomes visible.
- **Anchors inside a moved block collapse:** bookmarks, history entries and find matches are transformed by the
  buffer's edit transform, which maps a byte inside a replaced span to the end of its replacement. A bookmark inside
  a moved line therefore lands at the block's new end rather than on its line. Transforming anchors per line would
  need the move to publish a line mapping through `publish_edit`.
- **Unverified:** native macOS and Wayland GUI editing remain gated; the GUI checks ran on the GPUI test platform and
  the terminal checks in a Ratatui test backend, so physical terminals and real desktops were not exercised. No
  performance measurement or benchmark was run for this ticket (per the current verification policy).

## External change reconciliation (ticket 34, 2026-10-24)

- **A conflict is only known when it was observed:** `Resolve External Change…` needs a recorded conflict — the
  watcher's change, a rejected Save or a recheck that found another version — so without a watcher the command
  reports `NO_CONFLICT` while the file differs. Save still refuses that write through the writer's own check. A
  session-level "Disk changed" state that survives a missing watcher would need its own checked read on every
  activation; not implemented.
- **Keeping the draft does not block Save:** `Keep Draft` changes no save state, so Save is still offered and the
  writer rejects it again (with a message) instead of the command explaining itself first. Whether a kept conflict
  should disable Save with "Resolve the external change first" is a presentation decision for the next pass.
- **A take releases the recovery copy:** `Take Disk Version` edits the draft and confirms the taken version, which
  forgets its recovery record. Undo restores the previous draft and re-registers it, but a crash between the take and
  the Undo loses the old text; keeping the record while the buffer is clean would restore a draft identical to the
  file. No change is claimed here.
- **The dialog's disk side is a captured read:** the comparison and the dialog show the version read when the dialog
  opened (identified by size and by how it relates to the baseline). A further external change while they are open is
  only noticed by a decision's own re-read, which then refuses and refreshes the sides. Refreshing the open
  comparison from disk would need another read and a new request; not implemented.
- **Three comparison sides in two panes:** the conflict comparison puts the disk version and the confirmed baseline
  as two tabs in the first pane and the draft in the second. The labels are long (`Confirmed baseline (read-only)`),
  so a narrow pane or terminal can show only some tab titles; the labels are not shortened because they are also the
  identity the ticket asks for. Splitting the third side into its own pane would need a three-pane layout.
- **Suspected cost, unmeasured:** `Take Disk Version` copies the whole disk text once to build its single edit, and the
  undo record keeps the previous draft text as one inverse edit, so a take on a large file holds two full texts plus
  the snapshot — the shape of one Save. A decision reads and decodes the file twice (the dialog's read and the
  decision's check), both off the UI thread; no cache was added. Include a take and a save-over on the readiness
  corpus in the scheduled batch before optimizing.
- **Suspected cost, unmeasured:** `reconcile_prompt()` rebuilds the dialog text on each call; the GUI asks for it
  twice per draw while the dialog is open and the terminal once per frame (`clamp`). The text is a few hundred bytes,
  but it is new per-frame work; measure with the other dialog costs before changing it.
- **Matching the shown version:** a decision compares content and the file's device/inode. A bare `touch` (same
  content, same inode) therefore counts as the same version and the writer's own stamp check decides the write.
- **Observed integration-tip test failure (not caused by this ticket, corrected here):**
  `an_empty_session_offers_open_and_explains_the_rest` expected a command-label list that lacked ticket 27's
  Indent/Outdent/Comment/Move/Duplicate commands at `daefa89`, so it failed before this work. The expected list was
  brought back in line with `CommandId::ALL`, including the new `Resolve External Change…` entry.
- **Unverified:** native macOS and Wayland GUI editing remain gated, so their dialog keys, tiny-window wrapping and
  comparison tab bar were not exercised; the GUI checks ran on the GPUI test platform under an owned display. The
  terminal checks used a Ratatui test backend, not a physical terminal, so real tab-bar truncation and key delivery
  are unverified. No performance measurement, stress run or benchmark was made for this ticket.

## Snippets (ticket 30, 2026-10-24)

- **Only completion items supply bodies:** the client now declares `snippetSupport: true`, so language servers send
  snippet text for their own triggers. There is no snippet library, trigger-word expansion or user-defined snippet,
  and `Command::InsertSnippet` (the shared entry point) is dispatched only by tests today; a menu that offers named
  snippets would need a source for the bodies and a decision about where they live (settings, project file).
- **What is refused rather than approximated:** snippet variables (`${TM_FILENAME}` and any all-capitals name),
  transforms (`${1/re/format/}`), nested placeholders, an unterminated placeholder or choice, an empty `${}`, and
  `$0` with text. Each is listed with its reason and inserts nothing. Nested defaults are common in a few servers'
  refactor snippets; supporting them needs a nested stop model where visiting the outer stop leaves the inner one
  still to visit.
- **`${1|one,two|}` picks the first choice silently:** there is no chooser, and the remaining choices are lost. A
  chooser would be a popup or a status-bar toggle with its own modal rules.
- **Mirroring is only at stop boundaries:** a repeated placeholder inserts the same text at every occurrence (the
  first default written for its key) and one stop selects all occurrences, so typing replaces them in one
  transaction. Typing in one occurrence *before* Tab does not mirror to the others, unlike VS Code's linked mode;
  that needs per-occurrence link state through the edit transform and history.
- **A placeholder typed over collapses:** after typing inside a stop, the stop's range follows the edit exactly as
  the selection does, so it becomes a caret after the typed text; Shift+Tab back to it selects nothing. Keeping the
  typed text selected on return would need the stop to remember the text it now holds, separate from the selection.
- **Undo ends the snippet instead of remapping it:** placeholder positions are mapped through the edits of the live
  tab (the same transform the selections take), and Undo/Redo, Reload, another tab, a caret move, read-only mode or a
  preview simply end it. That is why stale positions cannot be visited, but it also means Undo of a typing step does
  not return to the placeholder it was typed in.
- **One snippet at a time:** inserting a body with several carets is refused with a reason, because one Tab cannot
  visit two sets of placeholders. Fanning one body across selections with independent stops would need a stop per
  insertion and a way to say which Tab moves.
- **Suspected cost, unmeasured:** every snippet suggestion is parsed once when the reply arrives (so the list can
  say whether it is expandable), and that parsed body is kept in the suggestion until it is accepted or replaced;
  acceptance expands it into one `String` plus its stops. A large `isIncomplete` list therefore holds one small node
  tree per item until the next reply. Include a completion reply with many snippet items in the scheduled batch
  measurements before changing that lifetime or streaming the expansion.
- **Terminal delivery of Tab is the terminal's:** Tab is a fixed editing key in both adapters (not a keymap chord),
  so the snippet path is reachable wherever Tab reaches Lenz; a terminal that sends Tab only as focus movement
  never gets here, and Ctrl+Space for completion is often NUL. The palette's **Code Completion** and the Tab key in
  source input are the routes; there is no separate "next placeholder" command in the menus.
- **Observed test flake (not attributed):** the `semantics` completion filter reported one failure twice while the
  GUI and terminal suites and their nested `fake_lsp` build were running concurrently (16.6 s for the filter rather
  than ~2.3 s), but the failing test's name was not captured. It passed 6 of 6 runs alone or with four `semantics`
  filters in parallel. A likely suspect is one of the tests that waits 700 ms for a late reply
  (`dismissed_or_left_completion_is_not_reopened_by_its_late_reply`), which was not confirmed. No cause or fix is
  claimed and the test was not changed.
- **Unverified:** native macOS and Wayland GUI editing remain gated, so their key delivery was not exercised; the
  GUI checks ran on the GPUI test platform under an owned display and the terminal checks in a Ratatui test backend,
  not a physical terminal. No performance measurement, stress run or benchmark was made for this ticket.

## Session restore around recovered work (ticket 33, 2026-10-24)

- **Suspected cost, unmeasured:** the workspace (up to 200 tabs) is rebuilt and compared on every places change and
  after each tab or layout command, and a changed one rides the debounced places write. Caret moves and scrolling alone
  do not checkpoint, so an interruption reopens each tab where it was at the last tab, layout or navigation change.
  Include the places write size and the per-command rebuild in the scheduled batch measurements.
- **Suspected cost, unmeasured:** each session claims an owner lock under `recovery/owners/` at start, and start checks
  one lock file per record that names an owner. Locks are `flock`-style advisory locks through `std::fs::File::try_lock`.
  Network or unusual file systems that do not honour them make every owner look ended, so a second window there would
  offer the first window's live drafts, as before this ticket.
- **Undecided drafts block edits:** by the user's decision, a file whose recovered draft is undecided (dialog open, or
  Later) is not editable until Restore or Discard. Its record is also never replaced or forgotten meanwhile (Reload,
  Save or undo of the disk version cannot reach it). `RESTORE_DIRTY` stays as a guard but ordinary edits can no longer
  produce a live draft to conflict with.
- **Dialog timing:** the recovery dialog waits while a project's last tabs are restored (normally until the first
  places read finishes), so a restored draft lands in its restored tabs. A New File created before that read keeps its
  tab and the saved workspace is not applied, as for a file opened first.
- **Not restored:** New File documents still are not part of the workspace (only through recovery), and a recovered
  file that is gone is restored into a new tab rather than the place its old tab had.
- **Unverified:** native macOS and Wayland GUI, physical terminals, and filesystems without advisory locks were not
  exercised for this ticket.

## Explorer rename and move (ticket 36, 2026-10-24)

- **Suspected cost, unmeasured:** Enter in the prompt checks the source, the destination and its folder with
  `symlink_metadata`/`metadata` on the UI thread, and the move itself (one `renameat2`/`renamex_np`) and the follow-up
  `canonicalize`/`metadata` of each affected open file run there too. Cheap locally, blocking on slow or network file
  systems; include a rename of a folder holding many open tabs in the scheduled batch measurements.
- **New audited FFI:** `lenz_content::write::rename_noclobber` is the content crate's third audited `unsafe` site
  (listed in its `lib.rs`): Linux `renameat2(RENAME_NOREPLACE)`, macOS `renamex_np(RENAME_EXCL)`. A file system without
  them (some network and FUSE file systems return `EINVAL`) refuses the move with that error rather than checking and
  racing; other platforms refuse outright. The macOS route is compiled for but was not run.
- **What follows a move:** open tabs (shown, released and read-only comparison sides), their drafts and undo history,
  the inactive cache entry of each followed tab, recovery records, Back/Forward entries, bookmarks and recent places of
  everything under the moved path, the project inventory, empty-folder and expanded-folder state, and watches. Cached
  text of files with no tab, and buffer aliases of paths with no tab, are left to age out; reopening reads the new path.
- **Save target:** a moved document's confirmed save target follows only when the file now at the new path has the same
  device and inode as the confirmed one; otherwise Save stays closed until Recheck, as after a move made elsewhere.
- **Refusals:** a running or unconfirmed save inside the moved path, and an undecided recovered draft there, refuse the
  move. A dirty draft does not: it moves with its file and is never written by the move.
- **No semantic updates:** references, imports and `require`s in other files are not changed. A move marks open
  documents for the next language-server sync, as a watcher-followed rename does; no `workspace/willRenameFiles` or
  `didRenameFiles` is sent, and what a running server does with the old document's URI was not verified with a real
  provider.
- **No shortcut:** Rename and Move are reachable from the explorer's row menu and F10 → View in both adapters; IntelliJ's
  Shift+F6/F6 were not bound because the presets' conflict review is the keymap ticket's decision.
- **Unverified:** native macOS and Wayland GUI, physical terminals, case-only renames on case-insensitive file systems,
  and moves across file systems (`EXDEV` is reported, not emulated by copy) were not exercised.

## Explorer delete (ticket 37, 2026-10-24)

- **Trash choice (user decision):** Lenz implements the freedesktop.org Trash specification itself, with no new
  dependency: the home trash (`$XDG_DATA_HOME/Trash`) when the entry is on the same device, else the mount's
  `.Trash/$uid` (only a real sticky folder) or `.Trash-$uid`. Other platforms, macOS included, refuse recoverable
  deletion and offer Delete Permanently; Finder's trash is not used.
- **Deletion date in UTC:** `.trashinfo` `DeletionDate` is written in UTC, not local time as the specification asks,
  because the core has no time-zone database. Restoring does not read it; desktop trash views show the time offset by
  the local zone. Directory size caches (`directorysizes`) are not updated.
- **Suspected cost, unmeasured:** the checks, the `.trashinfo` write with `fsync`, the rename and a permanent
  `remove_dir_all` run on the UI thread; a large folder deleted permanently blocks until it is removed. Restore starts a
  project rescan. Include both in the scheduled batch measurements.
- **Unsaved work refuses:** a document with a draft, a running or unconfirmed save, or an undecided recovered draft
  inside the entry refuses both deletes rather than asking Save/Discard; the user closes or saves first. Closed tabs'
  recovery keys are tombstoned so a later start cannot offer them.
- **Restore scope:** Restore Last Deleted Entry offers only this session's trash moves, newest first, and forgets them
  at exit; older items are restored from the desktop's trash. A restore refused because something now occupies the
  path keeps the item offered.
- **Partial permanent deletion:** what was removed is reconciled (tabs closed, tree updated) and the rest stays listed
  after a rescan, with the error reported.
- **Unverified:** a mount-level `.Trash`/`.Trash-$uid` route (no second file system in the test environment), native
  macOS and Wayland GUI, and physical terminals.

## Completion imports and resolve (ticket 39, 2026-10-25)

- **One transaction:** a suggestion's name, its `additionalTextEdits` and a snippet body apply as one edit of one file
  (one Undo); malformed or overlapping extra edits, extra edits overlapping the name, and server commands refuse the
  whole item with the reason. Server-initiated `workspace/applyEdit` stays declined.
- **Resolve round trip:** the client now declares `resolveSupport` for `additionalTextEdits`, so servers may defer
  imports to `completionItem/resolve`. Accepting such an item waits for the reply (the popup closes at once); an edit
  meanwhile makes the late answer insert nothing, with a message. Suspected cost, unmeasured: one extra request per
  accepted item on resolving servers; include accept-to-insert latency with a real server in the batch measurements.
- **Range carrying:** extra edits are converted against the version the popup was asked about and carried through the
  name-continuation typing since; a typed edit touching an import range refuses. Positions are assumed to refer to
  that version even if the server synced a newer one before answering a resolve; edits on other lines are unaffected
  either way, but a server answering against a version with lines inserted above would misplace an import. Verify
  with real providers (rust-analyzer flyimport, typescript-language-server auto-imports, jdtls) in tickets 44–48.
- **Several carets:** a plain suggestion applies at every caret whose surrounding name text equals the primary's, with
  imports once; any other caret refuses. A snippet still needs one caret.
- **Environmental test failures:** the Ruby LSP tests matching `completion` fail in this cloud container because the
  `ruby-lsp` gem is not installed (`bundle lock --local` cannot find it); they are unrelated to this ticket.
- **Unverified:** real providers (only the fake server and its scripted resolve were run), native macOS, Wayland and
  physical terminals.

## Suggestion documentation and parameter information (ticket 40, 2026-10-25)

- **Parameter Info:** `textDocument/signatureHelp` is asked by the new Parameter Info command and by typing a server
  trigger character (also when pairing inserts it, or typing steps over a closing pair), and asked again by typing
  while it shows; the answer is shown only for the exact tab, version, caret and server session it was asked about,
  and a dismissed one never comes back from a late answer. Active-parameter label offsets are converted with the
  negotiated position encoding; offsets inside a character are not placed.
- **Shortcut:** Ctrl+Shift+Space in both presets, because this IntelliJ preset already gives Ctrl+P to Find File.
  Terminals usually cannot tell Ctrl+Shift+Space from Ctrl+Space; there Parameter Info is reached through F10 or Find
  Action, and typing `(` opens it anyway.
- **Suggestion documentation:** shown beside the completion list in both adapters; when the list does not carry it and
  the server resolves suggestions, it is asked once per browsed suggestion on its own lane (the client now declares
  `documentation` and `detail` in `resolveSupport`). Markdown is shown as source text in both adapters, not rendered.
- **Suspected cost, unmeasured:** one signature request per typed character while the box shows, and one resolve per
  newly browsed suggestion; no debounce. Measure request churn with real servers in the batch measurements.
- **Unverified:** real providers (only the fake server's scripted overloads and documentation were run), native macOS,
  Wayland and physical terminals.

## Diagnostics and quick fixes (ticket 41, 2026-10-25)

- **Version fence:** published diagnostics are kept per server and path only when their `version` matches text that
  server was sent (a frame still being written counts), and shown only while they describe the exact draft version on
  screen. After an edit nothing is drawn until the server publishes for the new version; no stale range is shifted
  onto new text.
- **Quick fixes:** `textDocument/codeAction` at the caret, with the diagnostics there, in its own lane. An action
  applies only when its whole edit is text edits of this file at this version, as one undoable transaction; another
  file, a create/rename/delete, any server command, a `disabled` reason or a missing edit lists the action with its
  reason and never applies part of it. Preferred fixes come first. `codeAction/resolve` is not advertised, so servers
  that only fill edits on resolve show those actions as offering no change.
- **Shortcuts:** Show Quick Fixes is Alt+Enter (IntelliJ) and Ctrl+. or Alt+Enter (VS Code; Alt+Enter is the
  terminal route); Next/Previous Problem are F8/Shift+F8 in both presets, since this IntelliJ preset gives F2 to the
  language-server menu; Problems… is Ctrl+Shift+M in the VS Code preset and reachable through F10 or Find Action.
- **Presentation:** both adapters underline problem ranges (errors in the error colour, warnings in the accent colour,
  information and hints dimmed in the GUI, plain underline in the terminal), the status line names the problem at the
  caret, and Problems… lists severity counts and every problem with its line and column. There is no gutter mark and
  no project-wide problems view.
- **Found and fixed:** terminal text hits record a byte within their line; the completion, parameter-information and
  quick-fix popups and the problem marks compared them with document bytes, which misplaced the popups on every line
  after the first. They now convert first.
- **Debug output:** ticket 40's merge carried three `eprintln!` traces in parameter information; removed.
- **Suspected cost, unmeasured:** the GUI looks up the active tab's diagnostics once per drawn row, and the terminal
  scans the problems near the shown bytes per drawn cell; both are bounded by the viewport but not measured with
  thousands of diagnostics.
- **Unverified:** real providers (only the fake server's scripted problems and fixes were run), native macOS, Wayland
  and physical terminals.

## Formatting and organize imports (ticket 42, 2026-10-25)

- **Route:** `textDocument/formatting` (options from the file's resolved indentation: `tabSize` is the indentation
  width, `insertSpaces` its style) and a `codeAction` with `only: ["source.organizeImports"]`, both on their own lane
  and fenced to the tab, version and server session asked. Results apply as one undo transaction of the draft;
  overlapping or misplaced edits, an answer for changed text and an organize action touching any other file are
  refused whole. An answer that changes nothing adds no undo step.
- **Format on save (decision taken from the ticket text, open to review):** only the explicit Save command formats.
  When the server cannot be asked at all (no provider, provider without formatting, not started, large file) Save
  writes unformatted and says so; when a sent request fails, is cancelled with Escape, or answers for changed text,
  nothing is written and the draft stays dirty. The new **Save Without Formatting** writes the draft as it is.
  Save on close prompts, Save As and other internal saves never format.
- **Configured tools:** Lenz formats only through the language server; the formatter Editing Preferences reports
  (ticket 25) is not run directly. Servers that call the project's formatter (rust-analyzer with rustfmt) honour
  its configuration; servers that do not (TypeScript's server and Prettier) need per-language work in tickets 44–54.
- **Not done:** range formatting, format on type, `codeAction/resolve` for organize actions that send no edit, and
  EditorConfig `trim_trailing_whitespace` / `insert_final_newline` as formatting options.
- **Suspected cost, unmeasured:** one whole-document request per Format/Save; the fake server answers with a single
  replace-all edit, which keeps the undo record as large as the document. Measure with real servers.
- **Unverified:** real providers (only the fake server's formatter and organize action were run), native macOS,
  Wayland and physical terminals.

## Tool and capability explanations (ticket 43, 2026-10-25)

- **Shown in Server Details (both adapters, same core text):** status and readiness, workspace, executable and
  arguments, the Java/Node.js runtime chosen or found on PATH, the names of provider-settings environment variables
  (values are never shown or copied), version context, setup guidance, and the assistance the server declared versus
  what it lacks, from its capabilities. Before the server starts the list says it is not known yet.
- **Not done:** arguments are shown as configured, so a secret passed as an argument would appear; the runtime row
  covers only Java and Pyright's Node.js; capabilities registered dynamically after start are not reflected; there is
  no per-language environment (Python virtualenv, Ruby bundle) row beyond what each provider's limits text says,
  left to tickets 44–54.
- **Unverified:** real providers' capability lists (only the fake server's), native macOS, Wayland and physical
  terminals.

## Java editing with JDT LS (ticket 44, 2026-10-25)

- **Found and fixed:** JDT LS attaches `java.completion.onDidSelect` to every suggestion, so Lenz refused all Java
  completions as "running a language-server command"; it is now skipped like the parameter-hint trigger (no text
  changes; recent-choice ranking is lost). JDT LS answers `signatureHelp` only when `java.signatureHelp.enabled` is
  set; Lenz's defaults now set it.
- **Pre-existing, not chased:** `real_jdtls` (navigation, ticket-era fixture) fails here at its JDK step: Go to
  Definition on `println` opens `PrintStream` with the whole declaration line selected instead of the name, with
  OpenJDK 21.0.12's sources. The editing tests do not depend on it.
- **Environment:** JDT LS 1.61.0 was installed for this ticket from the Eclipse milestone tarball (checksum
  verified); the tests stay `#[ignore]`d and skip without `jdtls`. Each JDT LS instance used about 1.4 GiB; the
  adapter replay runs two at once.
- **Unverified:** Organize Imports choices for ambiguous types, Kotlin/mixed projects (ticket 45), Gradle import
  side effects in the project folder, native macOS, Wayland and physical terminals.

## Kotlin editing and server commands (ticket 45, 2026-10-25)

- **Decision (user, 2026-10-25):** server commands run only from a per-provider allowlist
  (`semantics::provider::runs_command`; today Kotlin LSP's `jetbrains.kotlin.completion.apply`, `applyModCommand`,
  `kotlin.organize.imports`) and only for the item the user picked; the server must also declare the command. The
  edit it sends back (`workspace/applyEdit`, now advertised) is applied only for the open file at the version the
  command was sent for, as one Undo step; otherwise it is refused whole and answered `applied: false`. Any other
  `workspace/applyEdit` is still declined.
- **Pull diagnostics:** servers declaring `diagnosticProvider` are asked `textDocument/diagnostic` behind every text
  update (no `previousResultId`, so every answer is full); answers are kept for exactly the text asked about.
- **Found:** Kotlin LSP's `--version` took 16.7 s on a cold start after a restart, past Lenz's 3-second version
  probe, so the server showed as missing until restarted. Not changed here; the probe timeout or a retry needs a
  decision of its own.
- **Suspected cost, unmeasured:** one diagnostic request per text update to such a server, without debounce.
- **Unverified:** Gradle Kotlin projects, completion items whose command also needs a snippet (refused), commands of
  other servers, native macOS, Wayland and physical terminals.

## Rust editing (ticket 46, 2026-10-25)

- **Found and fixed:** Lenz advertised pull diagnostics, so rust-analyzer never published problems; Lenz now asks
  `textDocument/diagnostic` per text update (ticket 45) and also advertises and answers `workspace/diagnostic/refresh`
  by asking again about every open text (rust-analyzer answers empty while loading, then sends a refresh).
- **Decision needed (not taken):** rust-analyzer runs with `checkOnSave: false`, a reader-era choice (no builds), so
  only its own diagnostics show; compiler errors (`cargo check`, E0433 for an undeclared type) do not. Turning it on
  would run `cargo check` after each Save: a build cost and a project side effect (`target/`) to agree with the user.
- **Older tests updated:** `real_rust_completion` asserted acceptance synchronously and the plain name; since tickets
  39 and 30 acceptance waits for `completionItem/resolve` and rust-analyzer's snippet inserts `helper_function()`.
- **Unverified:** cargo check problems (off), workspaces with proc macros or build scripts while editing, native
  macOS, Wayland and physical terminals.

## Go editing (ticket 47, 2026-10-26)

- **New provider:** gopls (Go files; root at the nearest `go.work`, else `go.mod`; version from `gopls version`;
  tested v0.24.0). Installed for this ticket with `go install golang.org/x/tools/gopls@latest`.
- **Found, not Lenz's:** gopls v0.24.0 resolves an unimported `uuid` to a bare `"uuid"` import with Go 1.24.7 (also
  from `gopls imports` on the command line), apparently from a newer standard-library index; the fixture imports
  the dependency explicitly instead.
- **Unverified:** multi-module `go.work` workspaces, cgo, build tags, vendored modules, native macOS, Wayland and
  physical terminals.

## TypeScript, JavaScript and TSX editing (ticket 48, 2026-10-26)

- **Found and fixed:** every typescript-language-server fix came as `_typescript.applyCodeActionCommand`, so Lenz
  refused all of them. Lenz now reads the tsserver action the command carries (1-based lines, UTF-16 offsets) and
  applies its changes when they are for this file only and name no further command; the command itself is never
  run, so no server command is added to the allowlist.
- **Behaviour, not a Lenz gap:** its Organize Imports sorts and combines but keeps unused imports (it uses tsserver's
  SortAndCombine mode); removing unused imports is a separate source action Lenz does not offer.
- **Environment:** typescript-language-server 6.0.1 was installed for this ticket with npm; the tests point it at the
  global TypeScript (`READER_TSSERVER` or `npm root -g`) and use a local JSX namespace instead of `@types/react`, so
  React's own types are unverified.
- **Unverified:** React projects with `@types/react`, monorepos, typescript-go editing (ticket 48 covers
  typescript-language-server), unsaved-target navigation beyond completion, native macOS, Wayland and physical
  terminals.

## Ruby editing and incremental sync (ticket 49, 2026-10-26)

- **Found and fixed (all servers):** Lenz sent every `didChange` as full text without a range. Ruby LSP 0.26.11
  advertises incremental sync and applies only ranged changes, so it silently kept the opened text and every later
  request failed (`InvalidLocationError`) or answered for old text. Servers asking for incremental sync now get one
  change replacing the whole text they hold; rust-analyzer, gopls, typescript-language-server, JDT LS and Kotlin LSP
  were re-run on their real editing tests afterwards and pass. The earlier "UTF-8 positions are wrong" note on Ruby
  LSP may have had the same cause; its index still answers in characters, so UTF-16 stays.
- **Changed default:** Ruby LSP ran with `formatter: none` and no linters (reader era); it now uses `formatter: auto`
  and its default linters, so RuboCop (when the project bundles it) reports problems, fixes and formats, loading the
  project's own `.rubocop.yml`. A project whose RuboCop configuration loads plugins now runs them when a file opens.
- **Also:** an explicit completion request keeps waiting for an identical one typing already started, instead of
  cancelling and asking again.
- **Environment:** installing `ruby-lsp` also makes the Ruby LSP fake-server tests that used to fail here
  (environmental, see the handoff) pass.
- **Unverified:** Rails (ticket 50), Standard/Syntax Tree formatters, Ruby LSP add-ons beyond the existing fixture,
  native macOS, Wayland and physical terminals.

## Rails (ticket 50, 2026-10-26)

- **Changed (all servers):** a suggestion without `detail` shows its `labelDetails` (signature, then description)
  as its detail. Lenz still does not declare `labelDetailsSupport`, so servers that honour the capability are
  unchanged; Ruby LSP and the Rails add-on send them regardless (`Filter Post records by title`, `() post.rb`).
  Declaring the capability would move other servers' signatures out of `detail`; not done.
- **Found and fixed (all servers):** a completion, parameter information, quick fix, Format Document, Quick
  Documentation or occurrence request asked right after an edit or Undo could find its version admitted (converted on
  a worker after the last document sync) while the server still held the previous one, and was refused with "has
  not been sent this version of the file" (seen as Format Document after Ctrl+Z in the terminal replay, 2 of 3 runs).
  Those requests now send the admitted version first when no server holds it yet, as definitions and usages already
  did; the replay then passed 5 runs in a row (and 3 more after narrowing the resend to that case). `a_request_right_after_an_edit_or_undo_sends_that_version_first` covers the sequence but did not
  reproduce the race in the core alone (the worker timing is the adapter replay's).
- **Suspected costs (unmeasured, for the end-of-epic campaign):** each of those requests now checks that a server
  holds its version and, when none does, runs one more document sync pass (a pass over the tabs, plus decoration
  upkeep) before it is sent. The Rails add-on boots the whole
  application in a second Ruby process (in the server's process group, so it counts as the server's helper memory and stops with it);
  a large app's boot time and memory add to Ruby LSP's. A Rails bundle indexes far more gems (the fixture's
  indexing ran about 10 s after "ready" on this machine), during which gem-backed answers are empty. rubocop-rails
  adds cops to every diagnostics pass.
- **Observed (server side):** right after an edit, Ruby LSP 0.26.11 with the Rails add-on sometimes answers a
  completion with nothing (a direct JSON-RPC probe without Lenz: 2 of 6 requests sent immediately after
  `didChange` were empty, none of 6 sent a second later; for the one-letter prefix `t` it sometimes answered keywords
  instead of the column either way). Typing completion then shows nothing; Ctrl+Space asks again. The tests ask
  again. rubocop-rails 2.38.0 fails internally (shown as a server
  notice) on a hand-written `schema.rb` using `t.timestamps`; Rails' own dumps are not affected.
- **Not offered:** code lenses (the add-on's route lenses and test runs), the pending-migrations prompt (needs
  `window/showMessageRequest` with message-action properties) and its Run Migrations; file symbols show the names'
  source text, not the add-on's `has_many :comments` labels.
- **Process check:** after each real Rails run no `ruby-lsp`, add-on server or `rails` process was left behind.
- **Unverified:** ERB views (ticket 58), Spring, engines inside a Rails app, Rails versions other than 8.1.4, native
  macOS, Wayland and physical terminals.

## HTML (ticket 51, 2026-10-26)

- **Found and fixed:** the HTML server reads the `css` section for its `<style>` blocks; Lenz answered it `null`, and
  vscode-langservers-extracted 4.10.0 then failed every validation (`Cannot read properties of null (reading
  'validProperties')`, shown as a server error), so pages had no problems at all, scripts included. Providers can now
  name further sections (`Provider::sections`): `css` and `javascript` are answered `{}` or the user's settings under
  that key; `js/ts` stays `null` unless set, since an empty object fails its script validation the same way.
- **Found and fixed (all servers):** a snippet whose only stop is `$0` (a CSS property's `color: $0;`) left the caret
  after the inserted text instead of at `$0`.
- **Changed (all servers):** a suggestion carrying `editor.action.triggerSuggest` asks for suggestions again once it
  is applied (quietly, and only with typing completion on), as VS Code does; the CSS server uses it to offer a
  property's values. `editor.action.triggerParameterHints` is still ignored.
- **Added:** closing tags through VS Code's own `html/autoInsert` request (`Provider::auto_insert`, a new
  `Question::AutoInsert` in its own lane): asked after `>` or `/` typed at one caret, inserted as its own Undo step
  when the version, caret and server session still match. Formatting is now offered by the HTML server
  (`provideFormatter: true`); the CSS server still has it off.
- **Suspected costs (unmeasured, for the end-of-epic campaign):** each `>` or `/` typed in an HTML file sends one more
  request (after the conversion of the typed version, like typing completion) and reads the provider's settings once
  (a merge of the settings JSON) to check `autoClosingTags`; every typed character does one more lookup of the server
  holding the file, as the completion and signature triggers already do. A property completion now asks the CSS
  server again for its values. Format Document on a page runs the server's HTML, CSS and JavaScript formatters (whole
  document, one edit). The HTML server's memory with formatting on was not compared with the 242–243 MiB measured
  before.
- **Server side, not fixed:** vscode-langservers-extracted 4.10.0 resolves TypeScript's library declarations one
  folder too high, so embedded JavaScript has no standard library or DOM types (`document.` members are not offered);
  VS Code's built-in server is unaffected but was not available to test editing here. Its formatter drops the final
  newline unless `format.endWithNewline` is set, and that setting also leaves an empty line at the end of each
  `<style>` block.
- **Not offered:** `autoQuote` (Lenz pairs quotes itself), linked editing of a tag pair (`linkedEditingRange`), quick
  fixes and organize imports (the server has neither), CSS file formatting, TypeScript or template languages inside a
  page.
- **Process check:** after each real HTML run no `vscode-html-language-server` process was left behind.
- **Seen, not caused here, fixed (`f6e0987`):** code-tui `snippet_placeholders_are_tabbed_through_and_escape_ends_them`
  failed at its second expansion on the integration branch before this ticket: the fake server's list had gained
  suggestions since it was written, so three Downs no longer reached `snippet_repeat`; it now picks it by label.
- **Unverified:** VS Code's bundled HTML server for editing, the CSS server's editing, native macOS, Wayland and
  physical terminals.

## Python (ticket 52, 2026-10-26)

- **Found (Pyright 1.1.414, not Lenz's):** with a configured `venv` that does not exist, Pyright logs `venv … subdirectory
  not found in venv path` and analyses with the packages of the first `python3` on its PATH, a different environment,
  without telling the client otherwise. Lenz now reads the same configuration at launch and does not start Pyright
  then (status "LSP stopped" with the reason; Server Details says to fix the environment, not to reinstall Pyright).
- **Added:** `semantics::python` resolves the environment (`pyrightconfig.json`, `[tool.pyright]`, `python.venvPath`,
  `python.pythonPath`) and Server Details shows it (`Provider::environment`, `ProviderView::environment`); without
  configuration it names the PATH interpreter Pyright uses and an unconfigured project `.venv`/`venv`. Lenz does not
  pick a project `.venv` on its own (VS Code's Python extension does): that would be selecting an environment the
  project did not configure.
- **Added:** Pyright has no formatter, quick fixes or organize imports (1.1.414 declares `quickfix` and
  `source.organizeImports` code actions but answered none for any problem tried). Python's Format Document and
  Organize Imports now run the project's configured ruff (or black, format only) on the draft through stdin, as one
  Undo step, fenced to the version given; format on save uses it too. The Editing Preferences formatter row now looks
  in the configured environment's `bin` instead of `.venv`/`venv` when one is configured.
- **Decision taken from the ticket text, open to review:** Organize Imports runs ruff's import-sorting rules only
  (`--select I --fix`), whatever else the project selects; ruff's other fixes and its lint problems are not shown (that
  needs ruff's own language server beside Pyright, two servers for one file, which Lenz does not do yet).
- **Suspected costs (unmeasured, for the end-of-epic campaign):** every Pyright launch reads `pyrightconfig.json` or
  `pyproject.toml` (and, unconfigured, runs `python3 --version`, 3 s limit) before the server starts, adding the
  interpreter's start-up time to Pyright's. Each Python Format Document or Organize Imports reads the project's
  configuration files and starts a formatter process (its start-up time unmeasured),
  copies the draft once into the process's stdin and holds its whole output; the edit is reduced to the changed span
  before it reaches the UI thread. Opening Server Details or Editing Preferences for a Python file reads those files
  on the UI thread (once per dialog).
- **Not done:** Escape (or an edit) stops waiting for a project formatter, but the process itself runs to its end
  (or the 20-second limit) and its answer is dropped; it is not killed early.
- **Observed, partly traced:** in the adapter replay, the terminal's Ctrl+End did not move the caret to the end of the
  document; the test places the caret by command instead. Neither adapter's key table maps Ctrl+Home/Ctrl+End to a
  caret motion (End without Shift scrolls to the end in both) and the Session has no document start/end motion, so
  the caret movement seen in the GUI came from elsewhere (not traced). Moving the caret to the document's start and
  end, as other editors do, is a gap in both adapters, not this ticket's.
- **Process check:** after each real run no `pyright-langserver`, `ruff` or `black` process was left behind.
- **Unverified:** conda environments, `executionEnvironments`, `${workspaceFolder}` variables, a configuration changed
  while Pyright runs (Pyright reloads it itself; Lenz checks again on restart), the PyPI wrapper's own
  `pyright-langserver` (its version shows as unknown), native macOS, Windows, Wayland and physical terminals.

## YAML (ticket 53, 2026-10-26)

- **New provider:** yaml-language-server (YAML files; root as HTML's: the browsing root, else the file's folder;
  version from its npm `package.json`; tested 2.0.0, installed for this ticket with npm on Node.js 22).
- **Privacy decision (from the ticket's guidance):** nothing is downloaded by default. SchemaStore and the Kubernetes
  CRD store are turned off in the settings Lenz answers, and the server is told to ask Lenz for remote schemas
  (`yaml/registerContentRequest`), which Lenz refuses (`vscode/content`), so the refusal holds whatever the schema's
  origin (modeline, mapping, `kubernetes`, a `$ref`). `remoteSchemas: true` (Lenz's own key in the provider's
  settings) or `schemaStore.enable: true` lets the server download; both apply on Restart Language Server. Verified
  with a recording HTTP proxy in the server's environment: silent when refused, one GET when allowed. Direct
  connections that ignore the proxy variables could not be observed this way; request-light 0.5.8 (the server's
  HTTP client) reads only those variables, and every remote fetch of 2.0.0 goes through it.
- **Found (yaml-language-server 2.0.0, worked around):** with `format` settings that leave out `enable`, the formatter
  is silently off (Lenz's defaults carry `format.enable: true`); without a `[yaml]` `editor.tabSize` (and an `editor`
  section with `detectIndentation: false`) its "Convert Tab to Spaces" quick fix inserts the text `undefined` (Lenz
  answers the root's `.editorconfig` size); its schema globs are matched as `**/` + glob without dot folders, so a
  project under a hidden folder (`~/.config/app`, a temporary `/tmp/.tmp…`) matched no mapping (Lenz now anchors
  relative globs at the workspace root, escaped).
- **Fixed (not YAML's, found by this ticket's tests):** in languages with paired delimiters, a typed character equal
  to the next one was stepped over instead of typed: Enter at the end of a line only moved the caret, and a letter
  before the same letter was swallowed. Only a closing delimiter or a quote steps over its twin now (as
  docs/code/tui.md already said).
- **Added:** Enter in a YAML draft indents by the caret line's structure (Lenz's own rule, in the core, with or without
  the server); YAML indentation is spaces only (EditorConfig's or the project's tabs overridden, "YAML indents with
  spaces only") with a default size of 2; Editing Preferences names the server's bundled Prettier as YAML's
  formatter (a project `.prettierrc` is not read by it); Server Details says where schemas come from.
- **Suspected costs (unmeasured, for the end-of-epic campaign):** Enter in YAML copies the caret's line up to the caret
  (one line per caret) and, after a key that opens a block, resolves the file's indentation, which reads the
  `.editorconfig` files above it on the UI thread (as Tab already does); a very long line is copied whole.
  Answering yaml-language-server's `[yaml]` section reads the root's `.editorconfig` files (at start and on its
  configuration requests, which 2.0.0 repeats for formatting and validation). The `schemas` globs are rewritten for
  every `yaml` configuration answer (small: the user's mapping). yaml-language-server's resident memory (about
  87 MiB here, with one schema) is not compared with a baseline.
- **Not done:** the server's on-type formatting (`\n`), code lenses (Jump to Schema), document links, rename and its
  `yaml/get/all/jsonSchemas` schema picker are not used; completion indentation follows the root's `.editorconfig`,
  not a per-file one; custom tags need `customTags` in the settings.
- **Process check:** after each real run no `yaml-language-server` process was left behind.
- **Failing before this ticket, fixed (`3ce643a`):** `semantics` tests
  `project_initialization_settings_are_isolated_by_language_server_root` and
  `starts_on_the_first_relevant_source_and_follows_a_unicode_definition` had stale expectations (Ruby LSP's `formatter`
  is `auto` since ticket 49; `applyEdit` is declared since ticket 45); code-gui
  `ctrl_f_finds_as_you_type_with_toggles_f3_and_list_all` was a real regression: since ticket 41 bound Alt+Enter to
  Show Quick Fixes, the GUI's find box no longer received it for List All. Seen once, passed on rerun: code-tui
  `the_status_bar_shows_the_chosen_sql_dialect_and_its_details_open_from_that_label` (its settings file not yet written).
- **Unverified:** real SchemaStore and Kubernetes schemas (exercised only as refused), YAML schemas written in YAML,
  multi-document files, `yaml.schemas` keyed by URL with downloads allowed beyond the one test GET, native macOS,
  Wayland and physical terminals.

## Shell scripts (ticket 54, 2026-10-26)

- **New provider:** bash-language-server (every file Lenz detects as a shell script; root as HTML's: the browsing root,
  else the file's folder; tested 5.8.1 from npm on Node.js 22). ShellCheck 0.9.0 and shfmt 3.8.0 (Ubuntu packages)
  were already installed; Lenz installs none of them.
- **Decision: Lenz runs ShellCheck and shfmt itself.** bash-language-server 5.8.1 is sent empty `shellcheckPath` and
  `shfmt.path`, and its quick-fix and formatting capabilities are ignored. Its own ShellCheck checks a script without a
  shebang as Bash even when the project's `.shellcheckrc` says `shell=sh` (it passes `--shell=bash`, which overrides the
  file), and places ShellCheck's character columns as UTF-16 units, a column short after each character outside the
  BMP; both were seen over its plain protocol before this ticket's code existed. Lenz's own run passes the shell it
  identified, places columns as characters, and also works without the server.
- **Decision: the shell's order** is the file's `# shellcheck shell=` directive, its shebang, a shell file name or
  extension (`.sh` names none), the provider's `shell` setting (per project or global), the nearest `.shellcheckrc`'s
  `shell=`, EditorConfig's `shell_variant`, else Bash. This keeps the shebang over a `.shellcheckrc` `shell=`, where
  ShellCheck run by hand lets the rc file win (verified on 0.9.0); the story asks for the script's actual shell.
- **Open decision (taken provisionally):** the "explicit language choice" is the `shell` key of the provider's settings
  in providers.json (global, or per project under `projects.<workspace root>`). There is no command in either adapter to
  choose a shell per file, as SQL Dialect… does for SQL; adding one means new commands, menus and replays in both
  adapters.
- **Found, not worked around:** bash-language-server 5.8.1 sometimes answers the first completion after an edit inside
  an unterminated double-quoted string (`echo "$gree`) with nothing, and the same request again with the suggestions
  (reproduced over its plain protocol); the tests ask again. It offers no parameter information. ShellCheck 0.9.0 has
  no BusyBox dialect (`--shell=busybox` is "Unknown shell"), so `ash` and BusyBox scripts are checked as dash. shfmt
  3.8.0 formats `[[ ]]` in POSIX mode without complaint but refuses Bash arrays.
- **Added:** an sh or dash script's suggestions leave out Bash's reserved words, builtins and Bash-syntax snippets
  (bash-language-server suggests the same ones for every shell); ShellCheck's fixes and "Disable ShellCheck rule … for the entire file" as Lenz's own quick fixes (local,
  fenced to the version checked); Format Document and format on save through shfmt as the identified shell with the
  file's indentation and its EditorConfig shfmt keys (shfmt ignores EditorConfig once given a dialect); Organize
  Imports disabled for shell scripts; Server Details rows naming the shell (and what named it), ShellCheck and shfmt
  with versions or missing; the problems list and Quick Fix say why nothing is reported (ShellCheck missing, off, zsh).
  Two provider fields for any provider: `withheld` (settings as the server receives them, in `initializationOptions`
  and configuration answers) and `ignores` (server capabilities Lenz serves itself); both are no-ops for the others.
  `Semantics::sync` now also takes every file tab's source. No behaviour of other languages changed.
- **Suspected costs (unmeasured, for the end-of-epic campaign):** one ShellCheck process per open script version once
  edits pause for 300 ms (a full copy of the text into its standard-input writer, off the UI thread, on one worker
  thread per window); each run re-reads the `.shellcheckrc` and `.editorconfig` files above a script that names no
  shell itself. On the UI thread, a script's changed version merges the provider's settings (a small JSON merge) and
  every changed synchronization lists the file tabs' sources and prunes the checks of closed ones (a linear search over
  them); Editing Preferences searches PATH for shfmt when opened. Each tool's `--version` runs once per executable.
  bash-language-server analyses up to 500 workspace files in the background; its memory is not compared with a
  baseline.
- **Not done:** a per-file shell command in the adapters; ShellCheck's "disable for this command" (bash-language-server
  places it with its parse tree); problems for scripts that are not open; zsh beyond the server's Bash-grammar
  assistance; Bats; ShellCheck 0.10's BusyBox dialect and shfmt's zsh (3.10+) untested; macOS, Wayland and physical
  terminals unverified.
- **Process check:** after each real run no `bash-language-server`, `shellcheck` or `shfmt` process was left behind.

## Environment files (ticket 55, 2026-10-26)

- **Decision: detection by name.** `.env`, any `.env.*` (`.env.local`, `.env.example`, `.env.production.local`),
  `*.env` and `.flaskenv` are environment files (`Language::Env`); a last extension that names a language wins
  (`.env.sh` Bash, `.env.json` JSON, `.env.d.ts` TypeScript). The name is the explicit choice, as for every language:
  Lenz has no per-file language command for any file type (only SQL has a per-file dialect), so a file named otherwise
  cannot be marked as an environment file, and a `.env` cannot be read as Bash, without renaming it. A direnv `.envrc`
  is a script direnv runs, so it stays plain text as before. A shebang does not override the name (as everywhere).
- **Decision: a hand-written lexer, not a grammar** (`crates/content/src/highlight/env.rs`): no maintained
  tree-sitter dotenv grammar is bundled, and a lexer keeps the rules exact. Rules: `#` comment lines; `export`, key,
  `=`, value; double quotes run to the next unescaped `"` across lines and paint `\x` escapes; single quotes run to the
  next `'` with no escapes; unquoted values end at a `#` after a blank; an unclosed quote paints only its own line;
  lines that are not entries stay unpainted. Values are data: `$VAR`, `${VAR}`, `$(…)` and backticks are painted as
  value text, not as references or commands, because nothing interprets them. Backticks are not quotes (Node's dotenv
  allows them; python-dotenv and Docker Compose do not). "Line continuations" are the multi-line quoted values; a
  backslash at the end of an unquoted value is an ordinary character (no common loader joins such lines). Divergence:
  python-dotenv accepts `\'` inside single quotes, which Lenz paints as closing at that quote.
- **Decision: pairing.** Only `"` and `'` pair, only with the caret right after an entry's `=` and nothing after it on
  its line; a selection is wrapped in the quote typed; the typed closer steps over its twin only when that twin ends the
  line and is not escaped; brackets, braces and backticks never pair or step. Toggle Comment uses `#`.
- **Privacy routes checked (a sentinel secret, `env_files` tests and the parity replay):**
  language servers (no provider lists the language, `wants_text` is false, no server's desired set holds the file, and
  format-on-save is skipped for it, so Save never asks a server; ShellCheck runs only for Bash); the saved declaration
  index (`DeclIndex::wants` and `has_declarations` are false); bookmarks and recent locations (their saved line text,
  the "anchor", is now empty for environment files and dropped when an older places file is read); settings and the
  workspace checkpoint (paths only); Language Server Status, Server Details, Copy Server Details and the language
  actions' disabled reasons (fixed text, no file text); notices and status messages after the replay. Every file the
  session left outside the project was scanned for the sentinel: only the recovery record held it. Recovery stores the
  unsaved draft and its last saved text (the `draft` and `baseline` fields), the file name as title, in the Reader's
  recovery folder (0700) as a 0600 file, until Save or Discard; that is what drafts already require.
  Not observable from Lenz: a language server started for another file of the project may read files from disk itself
  (workspace scans); Lenz only sends such servers file-change notifications with paths, never contents. Panic messages:
  the Reader binaries install no crash reporter (a panic prints its message to stderr), the lexer has no panic sites
  and a randomized test checks it never panics; the Diff app (`lenz`), whose crash reporter writes panic messages, now
  highlights environment files with the same lexer. Reader stderr lines (timings, appearance notices) carry no text.
- **Behaviour changes:** environment files were plain text before; they now get highlighting, `#` comments, folds and
  expand-selection steps in the Reader and highlighting in the Diff app. Their bookmarks and recent locations keep only
  a line number, so after an outside change a bookmark stays on its line number (edits made in Lenz still move it with
  its line), and Back/Forward into a changed version lands at the recorded top line without restoring the caret.
  Format on save no longer runs for them. Nothing changed for other languages; `config` in `highlight.rs` now returns
  an `Option` (only environment files have no grammar).
- **Known, not changed (all languages):** when Select Next Occurrence finds nothing, the status bar quotes up to 40
  characters of the selection on screen (never written anywhere). Project text search and Find read environment files
  in the session like any file.
- **Suspected costs, unmeasured (end-of-epic campaign):** each edit relexes the whole environment file on the syntax
  pool (linear, no tree, no incremental reuse; the same scheduling as a tree-sitter parse, which is also whole-file);
  typing a quote in one copies the caret's line twice on the UI thread (a very long line is copied whole); reading a
  places file runs name detection once per saved mark; `anchor`/`anchor_matches` check the snapshot's language first.
- **Not done:** no diagnostics for malformed lines or duplicate keys (they are only left unpainted); no key or variable
  completion; no Docker Compose `${VAR:?error}` / default-value semantics (all text); no per-file language override.
- **Unverified:** native macOS and Wayland GUI editing; physical terminals (the replay draws in a Ratatui test backend
  and on the GPUI test platform). No performance measurement or benchmark was run for this ticket (user decision).

## Markdown authoring (ticket 56, 2026-10-26)

- **Decision: Lenz's own help, no server.** Lenz has no Markdown language server; list continuation, link and heading
  completion and link following are written in the core (`markdown_authoring.rs`, `session/markdown.rs`) and take
  precedence over any server a user might configure for Markdown. Code Completion is enabled for Markdown drafts with a
  caret whatever the provider state.
- **Decision: the live preview is a second view, not a mode.** **Open Markdown Preview to the Side** reuses the split
  panel (Split Right's tab copy, shown as its preview) rather than a new side-by-side widget, so the preview is an
  ordinary read-only view of the same buffer: closing, moving and Unsplit work as for any split, and each pane keeps
  its own scroll. Scroll positions are not linked between source and preview.
- **Decision: the last preview stays while the next is made.** An edit no longer drops a shown preview to "Preparing":
  the tab holds the last preview with the edit sets since its version (`HeldPreview`, at most 1,024 sets, then it waits
  for the new version as before), and positions in it are carried through those edits (`View::preview_byte`). A render
  of a version edited again before it lands is still dropped by version, so under continuous typing the preview
  updates once typing pauses for one render. The GUI keeps the last layout while the new one is shaped.
- **Decision: Show in Source and Follow Link take the last clicked preview position** (`Command::PointPreview`, checked
  against the shown preview's tab and version at use), the reading line otherwise; Ctrl+click is
  `Command::ActivatePreview`. Keyboard users get the reading line; there is no keyboard caret in the Reader preview.
- **Behaviour changes (Markdown only):** Enter after a list item continues the list (an `EditKind::Replace` step, so it
  is not merged into the typing before it); typing `(`, `/`, `#` and destination characters in a Markdown draft asks for
  Lenz's link suggestions while Completion While Typing is on; the preview's context menu gains Show in Source and
  Follow Link (after Copy), a Markdown source's gains Open Markdown Preview to the Side; preview images decoded for one
  version are reused by the next (a changed image file shows after the preview is reopened); a first preview without
  images is complete in one step (no separate images message). No other language changed.
- **Found and fixed (Markdown previews, failing on the integration branch before this ticket):** a preview whose tab
  had been released (another file shown in its pane, then closed) stayed "Preparing Markdown preview" for good when
  shown again for the same version: the analysis workers took the new request, which carried the same `renders`
  generation as the delivered one, for a duplicate and dropped it. Each request now has its own identity
  (`render_seq`); the generation still stops work for a preview no longer wanted. Regression test
  `a_preview_shown_again_after_its_tab_was_released_is_made_again` (code-core).
- **Commands:** `CommandId::ALL` 178 → 181 (`PreviewBeside`, `ShowInSource`, `FollowLink`), none bound to a key.
- **Suspected costs (unmeasured, for the end-of-epic campaign):** every character typed in a Markdown draft copies the
  caret's line up to the caret once more to see whether it is in a link destination (a very long line is copied
  whole); typing in a destination re-lists the folder (`Project::children`, linear in the files under it) on `(` and
  `/`, and starts a headings job on `#` and each destination character while no popup shows (a whole-file line walk
  off the UI thread, or a file read of up to 4 MiB, superseded by the next keystroke); Enter in Markdown reads up to
  5,000 lines above the caret for fences and up to 1,000 below for renumbering, on the UI thread. A preview beside the
  source renders every edited version off the UI thread (the whole document is parsed again, as a reload did before),
  and the GUI shapes the whole layout again per landed version; neither is incremental. Measure typing latency and
  render churn with a large Markdown file and a preview beside it.
- **Not done:** scroll sync between source and preview; outdenting a nested empty item on Enter; renumbering after a
  list item is ended or deleted; reference-style links, `[[wiki link]]` completion and Obsidian's vault-wide note
  lookup; headings inside lists, quotes or HTML; anchors exactly as GitHub renders them for HTML, `_emphasis_` and
  entities; file suggestions without an open folder; a keyboard caret in the preview for Follow Link.
- **Unverified:** native macOS and Wayland GUI editing; physical terminals (Ctrl+click needs the terminal to report
  modifiers with mouse buttons; the menus work without it). The checks ran on the GPUI test platform and a Ratatui test
  backend. No performance measurement or benchmark was run for this ticket (user decision).

## Template source maps (ticket 57, 2026-10-26)

- **Decision: one projection per embedded language, never the template itself.** An ERB or Haml template
  (`crate::template`) is sent to a language server only as a projection under the template's own URI and version:
  its Ruby (each ERB code/output tag's code, or each mapped Haml script and `#{…}`, copied verbatim, a newline after
  each) to the Ruby provider as `languageId: ruby`, and, for an HTML ERB template (`.html.erb`, `.htm.erb`,
  `.rhtml`), its markup (same bytes, every tag including delimiters replaced by `_`, line ends kept) to the HTML
  provider as `languageId: html`. Ruby LSP's own ERB documents (`languageId: erb`, with the native replacement-range
  defect from the readiness work) are never used. Haml markup is never sent as HTML.
- **Decision: positions convert through the projection; a template itself never converts.** `semantics::lsp`
  `position`/`offset`/`offset_at` map a view's template bytes to and from its projection in the negotiated encoding;
  a position in the projection's scaffolding maps to nothing (the edit or answer is refused, never placed elsewhere),
  and a template's own document refuses conversion outright, so a reply path that forgot to use its server's view
  fails closed. Session reply handlers convert against `Semantics::served(doc, session)`.
- **Decision: routing by caret.** A question about a template is asked of the view holding the caret (its Ruby,
  else an HTML template's markup); Format Document, Organize Imports and file symbols ask its Ruby. A caret on a
  tag's delimiters or in a comment, in Haml markup or in a non-HTML template's text asks nothing and says why.
- **Decision: whole-transaction edit check, local.** Every provider edit (completion and its extra edits, snippets,
  quick fixes, formatting, organize imports, server-command edits, closing tags) must lie inside one mapped region,
  and reading each touched construct again (the ERB tag from `<%` to `%>`, the stretch of markup, the Haml line) must
  find the same tags, indicators, trim markers and line kind (`Projection::check`); otherwise nothing is applied and
  the reason is shown. Completion's typing since the popup is carried through. Only the touched constructs are
  copied, so the check does not copy the document on the UI thread.
- **Decision: Erubi's reading of ERB.** `<%-`, a `-`/`=` before `%>`, `<%==`, `<%#` and `<%%` follow Rails' Erubi
  (cross-checked against Action View 8.1.4's handler by `real_erb_regions_are_the_ruby_rails_compiles`); `%%>` in a
  code tag (an escaped `%>` to stdlib ERB, the tag's end to Erubi) leaves the template's Ruby unmapped with its line.
  A tag still being typed (no `%>` yet) runs to the next `%>`, as Erubi reads it (to the end when there is none), so
  while `<%` is half typed above other tags the Ruby problems describe the host text it swallows until `%>` is typed;
  likewise a Haml template whose Ruby stops being mappable mid-edit (typing `- if`) is closed in Ruby LSP and reopened
  once it maps again, the status bar saying why meanwhile.
- **Decision: Haml contexts are explicit.** Tags (`%name`, `.class`/`#id` chains, static `(…)` attributes, `/`,
  `<`/`>`), `=`/`&=`/`!=` and `-` scripts that open no block, `#{…}` in text, `|` continuations, `-#` comments (their
  nested lines skipped) and `!!!` are mapped; a filter, `{…}` or dynamic attributes, a Ruby block (`- if`, `do`),
  lines nested under content or Ruby, a comma-continued script, `/` comments, `~`, `==`, `\` lines with
  interpolation, tab indentation or nested/unfinished interpolation leave the whole template's Ruby unmapped, with the
  line and the construct in the refusal, the status bar ("LSP unavailable": not used for this template) and Server
  Details. Haml 7.5.1 compiles every mapped region verbatim (`real_haml_regions_are_the_ruby_haml_compiles`). Lenz
  does not check Haml syntax itself.
- **Decision: projection noise is dropped, not shown.** For Ruby views, problems from RuboCop or Standard Ruby (they
  would lint the projection's layout) and Ruby's "possibly useless use … in void context" warning on an output
  (`<%= x %>`, Haml `=`/`#{}`) are not shown; a problem whose end falls in scaffolding is drawn up to the nearest mapped
  byte. With Ruby LSP 0.26.11 RuboCop neither lints nor formats a `.erb`/`.haml` URI anyway (only Prism's problems
  arrive; Format Document says "nothing to change").
- **Behaviour changes (all languages, no effect when one server holds a file):** the completion, parameter-information
  and closing-tag trigger characters are those of every initialized server holding the file (a template's two views),
  and diagnostics from several servers for one file are shown together (the newest version they describe); semantic
  tokens and inlay hints are not requested for templates; a type hierarchy built from syntax (for a server without one) refuses templates. Server Details
  gains one row per template language. `CommandId::ALL` unchanged (181).
- **Found and fixed while testing (templates only):** when a typed version was not materialized yet, the session synced
  the documents servers already held, which for a template was one of its views: only that view's server kept the
  template, the other closed and reopened it a moment later, and a closing tag typed meanwhile was lost (seen once in
  six real runs). `Semantics::desired` now returns the template itself; guarded by the "never closed while shown"
  assertion in `an_html_erb_templates_markup_goes_to_the_html_server_with_its_ruby_hidden` (fails without the fix).
  Also made `markdown_lists_link_completion_and_a_live_preview_match_in_both_adapters` wait for the project listing
  before typing `](` (its first link popup was empty in about one loaded run in eight on this branch; not reproduced
  in six on the integration branch; test-only).
- **Left to tickets 58 and 59:** Rails view context (helpers, `@ivars` typed from controllers, partial locals) through
  ruby-lsp-rails, which the projection does not provide; markup assistance for Haml; configured template formatting
  (erb-formatter, htmlbeautifier, haml-lint); ERB percent-lines and stdlib-ERB trim modes; Ruby blocks and the other
  unmapped Haml constructs above; parameter information right after a freshly typed `(` (Ruby LSP answers nothing
  there; ticket 04's private preload is not ported); completion of methods without a receiver (Ruby LSP 0.26.11 does
  not offer top-level methods there, in a template or a `.rb` file alike).
- **Suspected costs (unmeasured, for the end-of-epic campaign):** every materialized template version builds its
  projections on the worker (one scan; the Ruby projection holds the tags' code, an HTML template's markup projection a
  full copy of the text, so a template held by both servers costs about twice its size per version kept), and sends the
  projection whole per version as for any file. The merged problem list of a template is rebuilt per publication. An
  accepted completion in a template copies the touched tag or line from the snapshot. `Kind::of` runs on every
  position conversion (an extension comparison, no allocation).

## ERB (ticket 58, 2026-10-26)

- **Decision: an ERB template's Ruby runs as it is compiled.** The Ruby projection wraps the tags' code in a method
  (as Action View, Tilt and `ERB#def_method` compile templates): a layout's `yield` was reported "Invalid yield"
  under ticket 57's top-level projection. A Rails view (`app/views/…`) gets a class of its own,
  `LenzTemplate::View_<path>` (named as Action View names its compiled method), subclassing `ActionView::Base` and
  including `ApplicationHelper` (mailer views: `ActionMailer::MailHelper`) and the folder's own helper, with Action
  View's strict locals as the method's parameters. Ticket 59 made the same choice for Haml views independently
  (`LenzHamlView…`); both are now treated alike below.
- **Decision: a per-view class, indexed on open, rather than reopening `ActionView::Base`.** Ruby LSP learns a
  projection's declarations only when it indexes the document (on a document-symbol request), and its ancestor cache is
  not cleared when a URI is indexed the first time, so includes added to an already-linearized `ActionView::Base` would
  be missed for every view opened later. A class per view is first linearized after its indexing; Lenz asks for the
  document's symbols right behind its `didOpen` (before any edit or question). Probe evidence: before that request
  `link_to` and the app's helpers complete to nothing; after it, both complete.
- **Decision: helpers by path only.** Rails includes every helper in every view by default; Lenz includes only the
  application's and the folder's own, so it never offers what a view may not have, at the cost of other helper
  modules. Listing `app/helpers` would mean file reads on each projected version (or a cache with invalidation).
- **Decision: formatting is erb_lint's, configured by the project.** erb_lint (`.erb_lint.yml`) is the configured
  ERB linter with autocorrect in Rails projects; htmlbeautifier and erb-formatter have no project configuration, and
  Herb's formatter is experimental. Run as the project runs it (binstub, else `bundle exec` when locked, else PATH).
  Without one, Format Document says so; no language server is asked for an ERB template's formatting any more (Ruby
  LSP formatted nothing for templates; with ticket 59 Haml is haml-lint's, so no template's formatting reaches a
  server).
- **Behaviour changes (all languages):** a project-formatter run (Python, shell, SQL, ERB, Haml) whose version changed
  before it could start now says "the file changed before the project's formatter could run" instead of naming the
  language server. Suggestions, file symbols and workspace symbols named as template scaffolding (`LenzTemplate…`,
  `LenzHamlView…`, `__lenz_haml_render__`) are dropped from every server's answers; a scaffolding symbol's children
  are listed a level up. `runtime::run_with_input_env` added (stdin tools with extra environment).
- **Server side, not fixed (Ruby LSP 0.26.11 with its Rails and RuboCop add-ons):** Go to Definition and hover answer
  nothing when non-ASCII text precedes the caret on its line (`link_to("é", posts_path)`), in a `.rb` file as in a
  template, reproduced with a JSON-RPC probe without Lenz; non-ASCII after the caret is fine, as is parameter
  information on such a line. Running Ruby LSP's `Definition` request class in-process on the same text answers
  correctly, so the cause is in the running server's setup, not located further.
- **Tool side, not fixed (erb_lint 0.9.0):** TrailingWhitespace fails on CRLF files; any correction turns CRLF into
  LF (parser's source buffer; Lenz restores CRLF for files that use it throughout); `--stdin` needs the file to exist;
  the result gains a final newline. Recorded in providers.md.
- **Not offered:** controller instance variables and route-helper suggestions in views (Ruby LSP's index has neither;
  the Rails add-on 0.4.8 adds no view context); partial locals passed by `render` without strict locals; erb_lint's
  remaining problems as problems; ViewComponent sidecar templates' component context; stdlib-ERB percent lines.
- **Suspected costs (unmeasured, for the end-of-epic campaign):** each Rails view (ERB or Haml) opened in Ruby LSP
  costs one more document-symbol request, which makes Ruby LSP run its combined requests and index the projection;
  the per-view classes stay in Ruby LSP's index until it restarts. Each projected ERB version scans the whole text
  once more for strict locals (`# locals:`), and carries a few scaffolding lines. Each completion item and symbol of
  any server is checked against the scaffolding names (a prefix comparison). Format Document on ERB starts erb_lint
  (Ruby, RuboCop, and `bundle exec` when locked: about 1 s on the fixture) per request.
- **Seen, not caused here:** code-tui
  `app::tests::sql_dialect::the_status_bar_shows_the_chosen_sql_dialect_and_its_details_open_from_that_label` fails
  now and then when run in parallel with the other `details` tests (`settings.json` not found at
  `input_tests/sql_dialect.rs:66`): the session writes settings on a background job and the test reads the file
  without waiting. Passes alone and serially; it passed 4 of 4 runs on the integration branch and failed 1 of 3
  (earlier 3 of 3 inside a larger filter) on this branch, whose changes do not touch settings.
- **Process check:** after each real run no `ruby-lsp`, add-on, `rails` or HTML server process was left behind.
- **Historical ticket-57 bug, reviewed during ticket 68:** code-tui
  `template_assistance_routes_through_its_projections_alike_in_both_adapters` fails intermittently at its wait for both
  servers' problems after two Ctrl+Z return the template to its first text (`parity_tests.rs`, the `problems(state) == 2`
  step before the quick fix): the GUI's session then holds no diagnostics for the tab at all while the terminal's holds
  both, and none arrive within 15 s. It failed 2 of 6 warm runs at ticket 57's merge (`fec6f27`), 2 of 6 on this
  ticket's branch and about 1 in 3 after merging tickets 62–64; it passes alone more often (10 of 10 once with
  `--exact`). The later review repaired diagnostic identity/publication ordering and verified targeted parity repeats;
  the original run is retained without claiming every old timeout had the same cause.
- **Unverified:** Rails versions other than 8.1.4, erb_lint versions other than 0.9.0, engines, mailer views with real
  servers (unit-tested only), native macOS, Wayland and physical terminals.

## Tests that time out under load (2026-10-24)

The rates below describe the pre-review source; see the current review above for repaired fixture
races, reproduced product fixes and unreproduced failures.

- **Historical: seven tests failed at random, each waiting out its `settle` deadline:** the event they wait for never
  arrives, so a failing run takes the full timeout rather than running slowly. Run alone and back to back, as
  `scripts/ci-local.sh` runs them, the suites were clean in 16 of 20 (`code-core` lib), 35 of 40 (`code-gui`
  lib) and 18 of 20 (`code-tui` with `gui-test`) runs, so the full gate fails roughly one run in three:

  | Test | Failed |
  |---|---|
  | `code-gui` `tests::outline_and_declaration_search_through_keys` | 5 of 40 |
  | `code-core` `session::tests::semantic_requests_wait_for_the_current_admitted_snapshot_and_preserve_saved_text` | 2 of 20 |
  | `code-core` `semantics::tests::request_target_contexts_replace_per_lane_and_retire_on_cancel_reply_and_exit` | 1 of 20 |
  | `code-core` `session::tests::save_as::save_as_moves_every_view_only_after_the_new_file_is_confirmed` | 1 of 20 |
  | `code-tui` `app::tests::sql_dialect::the_status_bar_shows_the_chosen_sql_dialect_and_its_details_open_from_that_label` | 1 of 20 |
  | `code-tui` `app::tests::followed_rename_invalidates_source_context_hits_without_changing_text_version` | 1 of 20 |
  | `code-core --test semantics` `dismissed_or_left_completion_is_not_reopened_by_its_late_reply` | 18 of 80 alone, 1 of 15 in its binary |

  Running 64 copies of one test at once (`<test binary> --exact <name>` in parallel) raises the rate enough to
  trace it.
- **The completion late-reply test's third request is sometimes never sent:** after the tab switch and `Back`,
  `Command::Complete` at the same position leaves the fake server's log at two completion requests, whether or not
  the caret then moves (checked by waiting for the request before moving it). That may be a real gap (completion not
  requested after returning to a tab) rather than a test race; it fails more often run alone than with the rest of
  its binary.
- **Two diff-app Markdown tests also time out at random (seen 2026-10-25, during ticket 41's gate):**
  `lenz` `ui::markdown::tests::gui_markdown_final_markup_marker_is_independently_visible` and
  `gui_markdown_final_navigation_reveals_hunks_and_markup` both waited out their deadline in a 259-test run; run alone,
  the first passed and the second failed again. Neither touches the reader crates. Not traced yet.
- **Two causes already found in this class were product bugs, not slow tests:** a check that ran after a rename
  but before the watcher reported it dropped the file identity the move needed (`42e7fc7`), and a prompt that a
  render-time layout command settled left both adapters' dialogs stale (`9c4e9de`). Every core dispatch ends by
  accounting saves and advancing pending work, so an adapter or a test that reads state only after its own
  dispatches can miss a change. Trace each remaining test before raising a timeout.


## Ticket 59: Haml authoring costs and remaining contexts

Haml projection now tracks indentation-controlled Ruby blocks, attributes and nested interpolation;
Rails view projections add private per-URI lexical helper context. Maps are still constructed once per
admitted version on the materialization worker. Tag/attribute suggestions and Enter inspect a bounded
line window (16 KiB); explicit indentation resolves project preferences. Formatting adds a bounded
20-second Ruby/haml-lint subprocess on the existing session worker path, with source on stdin and
current-version result checks. Paired release latency, peak/settled memory and sustained stress remain
for the agreed end-of-epic campaign; no preservation claim is made from correctness tests.

Filters, object references, multiline attribute lists, tab indentation and Ruby regex/percent literals
inside mapped interpolation/attributes remain unmapped. Haml-native syntax highlighting and live Haml
grammar problems are absent. Rails helper scope is static and convention-based; controller-variable
types, partial locals and namespaced/custom helper inference are unverified. See the public
[provider capability description](code/providers.md#haml-editing) for exact tested behavior.

## Ticket 61: offline SQL worker costs and inference limits

PostgreSQL editor requests reuse the bounded analysis scheduler, with consumers for syntax, completion,
parameter help and formatting. Each source version is admitted/materialized once; superseded requests
are coalesced or cancelled and responses check document, dialect, project and source identity. Syntax
checks run when the active SQL version changes. Costs needing the agreed end-of-epic campaign include
first admission, parser work while typing, quoted-name/token recovery, CTE/derived-scope traversal and
formatting. No latency or memory-preservation claim is made from correctness checks.

The resolver now handles set-output names, explicit recursive CTE columns, qualified wildcard outputs,
known builtin table-function outputs, non-LATERAL isolation, ambiguous-column omission and incomplete
quoted identifiers. General malformed-query recovery, complete LATERAL/CTE visibility semantics,
unknown function result schemas/types and exhaustive builtin overloads remain limited. Native macOS
and Wayland are unverified. Metadata acquisition remains an independent explicit workflow.

## Ticket 62: native DuckDB request costs

DuckDB editor assistance shares the SQL admitted-version scheduler and response fences. Native parsing,
format validation and overload help launch bounded CLI requests with one engine thread and 128 MB engine
limit; cancellation kills the owned process group. Output capture is capped and drains pipes. Source goes
only on single-line encoded stdin, never argv. Repeated process startup and the case-insensitive scope
resolver's name normalization are costs for the end-of-epic campaign. No performance-preservation claim
is made here. External metadata acquisition remains separate; unknown result types, complete visibility
and arbitrary malformed lexical recovery remain unverified/unsupported as documented in providers.md.

## Ticket 63: explicit metadata snapshot costs

Connect/Refresh submit a bounded worker process; PostgreSQL reads fixed catalog operations only,
with a five-second deadline, 1 MiB capture and 512-entry limits. Snapshots are immutable and reused
by admitted-version completion/signature workers for up to five minutes. Disconnect, scope changes,
refresh and expiry cancel/reject obsolete replies. The real PostgreSQL 17.11 fixture and real GUI/TUI
replays pass with server statement observation, unchanged rows and no document execution. Process
startup, catalog decoding/cloning and retained snapshot memory need the agreed end-of-epic campaign;
no latency or memory-preservation claim is made here. Native macOS/Wayland remain unverified.

## Ticket 64: explicit file metadata costs

DuckDB metadata adds a bounded in-memory engine and a read-only attachment only on Connect/Refresh.
The catalog is capped at 1 MiB/512 relations/functions. File/WAL metadata is checked on demand for
assistance and publication, plus at most once per second during session synchronization; this adds
filesystem calls only while a file snapshot is retained. Catalog identifier validation and qualified
function suffix resolution add bounded parser/name work. Real native tests and actual GUI/TUI controls
pass; syscall observation shows read-only database opens and zero connect calls, with unchanged
fixture bytes/rows and no export/sentinel. Process startup, retained catalog memory and filesystem
polling costs need the agreed end-of-epic campaign. No performance-preservation/native-platform pass
is claimed; macro inference and unqualified search-path resolution remain limited.

## Ticket 67: change inspection costs and remaining checks

The existing settled marker comparison now retains exact byte hunks and an indexed baseline, reusing
its line diff instead of comparing again on click. HEAD snapshots share decoded text with the cached
blob; creating their indexed wrappers and retaining a hunk vector add per-published-version metadata.
Opening/refreshing the inspector copies at most 64 KiB for its preview; an explicit revert copies at
most 4 MiB of prior text into one normal draft edit. Repository reverts add an off-thread HEAD/absence
check and reject an intervening draft/epoch change. The comparison reuses the existing draft/history
and holds its inspected baseline until closed. These costs need the agreed end-of-epic latency/memory
campaign, alongside sustained inspect/compare/revert/undo stress. No preservation claim is made from
correctness tests. Core and actual GUI/TUI replays cover HEAD and last-save input/render paths,
CRLF/Unicode, EOF deletion, file/index preservation and modal input ordering. Native macOS/Wayland
and physical terminal behavior remain unverified.

Historical ticket-67 limit, repaired during the Linux review: plain Home/End now moves the caret
to line bounds in both adapters; document bounds and Shift selection share core motions.
