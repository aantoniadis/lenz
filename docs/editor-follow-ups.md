# Editor follow-ups

Record findings here instead of expanding the current editor implementation. Deal
with these after the approved work is done. A finding that blocks an approved
ticket's acceptance remains in that ticket; this file does not waive its checks.

## Deferred observations

- **TUI overhead:** the final snapshot-window comparison measured startup RSS
  +0.592 MiB and completed Go To median +0.085 ms, with substantial timing
  variation. Settled RSS decreased. The user accepted the documented batch costs
  on 2026-10-24; retain this as a later optimization opportunity.
  Evidence: `docs/research/reader-snapshot-tui-performance.md` on the TUI branch
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
  `target/perf/editor-implementation/merge-11-18/reader-gui-*.log`.
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
  limits are in the [batch performance report](research/reader-editor-batch-11-18-performance.md).

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
  The [comparison report](research/reader-saved-comparison.md) records current proof and unverified limits.

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
  about 0.676 MiB across eight launches. See the [batch report](research/reader-editor-foundation-batch-14-23.md).
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
  detection). `double_and_triple_click_select_word_and_group` failed once under the parallel reader-tui run
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
  commands refuse the item. Revisit with ticket 40 (signature help) and 39 (resolve/additional edits).
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
- **Cache lifetime:** records live under `$XDG_CACHE_HOME/lenz-reader/recovery`, so a system cache clean removes them.
  They are not durable storage and no UI claims otherwise.
- **Prompt delivery:** the GUI syncs the recovery dialog on construction and on every dispatch; `ReaderView::poll` does
  not sync prompts, so a prompt created while another modal is open appears when that modal closes or the next command
  runs. The TUI syncs it in `clamp()` each frame.
- **Unverified:** native macOS and Wayland editing remain gated; their termination route and the recovery dialog were not
  run there. Wayland GUI and physical terminals were not exercised for this ticket.
- **No content in diagnostics:** records and prompts never carry source text; `Record`/`Version` `Debug` show identity and
  sizes only. Keep it that way when adding fields (for example a diff summary) to the dialog.
- **Observed core-suite flakes (not caused by this ticket):** with draft recovery present, the full `reader-core`
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
- **Observed test flakes (not caused by ticket 26):** the full `reader-tui` suite failed
  `double_and_triple_click_select_word_and_group` once and the full `reader-gui` suite failed
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
