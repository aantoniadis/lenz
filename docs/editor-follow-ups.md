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
  Draft recovery has no "recovered" state yet, so only dirty and Git status are shown.

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
