# Reader code completion (ticket 38)

Completion is a shared Session behavior; the GUI and terminal translate keys and draw the popup.

## Behavior

- `Command::Complete` (Ctrl+Space, **Code Completion**) asks the active tab's server with
  `textDocument/completion` in its own request lane, so documentation, occurrence and navigation requests
  never cancel it. Typing a word character or a server `triggerCharacters` entry asks automatically unless
  **Completion While Typing** (`auto_completion` setting, default on) is off; the command still works.
- A request records its tab, exact draft snapshot (key and source metadata), caret and trigger. Its reply is
  shown only when it is still the pending request, the tab is active, the snapshot and caret are unchanged and
  the server session can still answer. Dismissal, another tab, a caret move, an unrelated edit or a newer
  request drops it, so a late reply cannot reopen a dismissed popup or edit another document.
- Typing that continues the name at the caret (word characters, or Backspace not past the name's start)
  keeps the popup, moves it to the new version and re-filters it locally (case-insensitive subsequence of
  `filterText`/`label`); an `isIncomplete` list is asked again. Any other edit, punctuation included, closes it
  after typing normally: punctuation never accepts.
- Tab/Enter (or a click) accepts: one `EditKind::Replace` transaction against the current snapshot replacing
  the provider's range, extended to the caret, with plain text (`textEdit.newText`, else `insertText`, else the
  label). The core edit path rejects stale versions; one Undo restores the name as typed.
- Items with `additionalTextEdits`, snippet text (`insertTextFormat` 2), a server command (other than the
  client-only parameter-hint/suggest hints, which are not run), or a range that does not contain the caret on
  its line are listed dim with an explanation and refused whole. The client declares no snippet support.
- Manual requests report why completion is unavailable (no provider, missing/stopped/disabled server, a server
  without `completionProvider`, gated input, read-only, preview) or that there are no suggestions; automatic
  requests stay quiet. Ordinary editing never waits for a reply; an unadmitted version defers the request.

## Verification

- Core, fake provider (`crates/reader-core/tests/semantics.rs`): manual acceptance and single Undo with disk
  bytes unchanged; Escape, tab switch and caret move before a slow reply; typing open/filter/punctuation,
  trigger character, Backspace, setting off and persisted; refused import/snippet items; no provider and a
  server without completion.
- GUI (`reader-gui` `completion_popup_opens_by_typing_and_ctrl_space_and_takes_only_its_keys`): real
  keystrokes for typing, Down/Enter, Undo, Escape, Tab, punctuation, Ctrl+Space with the setting off, a modal
  taking keys first, a 160×120 window and a click on a row, repeated open/close.
- TUI (`completion_popup_takes_only_its_keys_and_fits_a_tiny_terminal`): the same keys, Ctrl+Space and NUL,
  palette precedence, a 30×6 terminal and a click on a row.
- Parity (`completion_typing_acceptance_and_dismissal_match_in_both_adapters`): one replay through both
  adapters with equal text, selection and popup rows/selection after each step.
- Real provider (opt-in, `real_rust_completion` in `tests/providers.rs`): an installed rust-analyzer build
  (9074e9b4c6 2026-09-06) on Linux that the provider table does not list as tested (the existing `real_rust`
  check rejects it as untested); no run with a tested version exists. It covered manual completion after a
  non-BMP character on a CRLF line, acceptance, a `.` member list and Undo, disk unchanged. Other providers are
  unverified for completion.

## Not covered here

Imports/additional edits, snippets and `completionItem/resolve` (ticket 39); documentation and signature help
(ticket 40); multi-cursor completion; native macOS and Wayland input; performance measurements, which are
deferred to the end of the epic.
