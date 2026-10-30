# Reader snippets and placeholder navigation (ticket 30)

A snippet is shared Session behavior: the core expands it, owns the live placeholder state and decides what Tab
does; the window and the terminal translate keys and paint the shared status text.

## Where snippets come from

The only source of snippet text today is a language server's completion item with `insertTextFormat: 2`, whose
`textEdit.newText` or `insertText` is the body. The client now declares `snippetSupport: true`, so servers send
these bodies; accepting one inserts the expansion instead of the raw text. Items that also carry
`additionalTextEdits` stay refused (ticket 39 applies imports atomically). `Command::InsertSnippet { tab, expected,
range, body }` is the same core entry point for any future snippet source and for tests; no adapter dispatches it
directly — Tab, Enter, Escape and typing reach the shared code through their existing keys.

## Supported body grammar

`$1`, `${1}`, `${1:text}`, `${1|one,two|}` (the first choice), the same forms for a name (`$name`, `${name:text}`,
`${name|a,b|}`), and `$0` for the final caret. Escapes `\$`, `\}`, `\\` are literal, and `}` inside a default must
be escaped (`\}`) because the first unescaped one closes the placeholder. Line breaks in a body become the
document's own line ending, so a CRLF file never gains LF lines.

A body this editor will not expand is refused whole — nothing is inserted — and the reason is shown beside the
suggestion and again if accepted: a placeholder inside a default, an unterminated placeholder or choice, a
placeholder transform (`${1/re/format/}`), a snippet variable (`${TM_FILENAME}`, an all-capitals name), an empty
`${}`, or a `$0` that carries text. Named placeholders are ordinary stops; they are not substituted from the
environment.

## Placeholder order and stops

Numbered placeholders are visited in ascending numeric order, then named ones in the order they first appear,
then the final caret (`$0`, or an implicit stop after the inserted text when a body writes none). One placeholder
written more than once is one stop that selects every occurrence together, and every occurrence is inserted with
the same text (the first default written for that key). A body with no placeholders places the caret after the
inserted text and starts no snippet state.

## Indentation

Inserted lines follow the file's resolved indentation — `.editorconfig` first, then the project's choice, then the
language default (ticket 25's resolution, computed at insertion). The body's leading tabs and spaces are counted
in steps (a tab is one step, `indent_size` spaces are one step) and written in the file's step; spaces that do not
fill a step stay for alignment. Every line after the first starts at the leading whitespace of the line the snippet
begins on, and a whitespace-only line stays empty rather than gaining indentation.

## Tab, Escape and ending a snippet

While a snippet is live, Tab visits the next stop and Shift+Tab the previous one; past either end — including
after the final caret — the snippet ends and that key press changes nothing. Without a live snippet, Tab and
Shift+Tab keep ticket 27's meaning: indent the lines the selections touch by one step, or insert the step at a
caret, and outdent respectively. A completion popup still takes Tab first (ticket 38), so Tab accepts a suggestion
rather than moving a placeholder while the popup is open.

Escape ends a live snippet (it stays a fixed editing key, and is offered even when only one selection is active),
then collapses several selections as before. The snippet also ends whenever it is no longer about the text it was
inserted into or the view showing it: a caret move, a click, a selection change, Undo/Redo, Reload, another tab,
a split-pane change, read-only mode or a preview. Ending never edits text. The state is validated on every read,
so a stale position can never be visited; the core drops it at the next command.

## Several selections

Typing, paste and delete at a repeated placeholder change every occurrence in one transaction, and each caret
follows its own edit — the ordinary multi-selection path. Inserting a snippet with several carets is refused with
a stated reason, because one Tab cannot visit the placeholders of two insertions. A placeholder typed over
collapses to a caret at the end of the typed text, so revisiting that stop selects nothing.

## Presentation

`Session::snippet_hint()` is the status text both adapters show while a snippet is live:
`Snippet: placeholder 2 of 3 · Tab next · Esc done`, or `Tab ends` on the last stop. It appears in the shared
status message after file notices, so the window's status bar and the terminal's footer show the same words.

## Verification

- Pure module (`crates/code-core/src/snippet.rs`): ordering with `$0` first in the text but last in the visit
  order, repeated placeholders, named placeholders, choices, escapes, indentation conversion for space and tab
  files, alignment spaces, CRLF, multi-line defaults, and every refusal message.
- Session commands (`crates/code-core/src/session/tests/snippet.rs`): insertion, first-stop selection, the
  status wording at each stop, Tab/Shift+Tab, indent again after the end, Escape, a caret move, a tab change and
  return, Undo of a typing step and of the whole insertion, a repeated placeholder typed at every occurrence,
  project indentation with CRLF, and the refusals (malformed body, several carets, read-only). Disk bytes are
  asserted unchanged.
- Completion acceptance with the real fake server (`crates/code-core/tests/semantics.rs`): a snippet item
  expands in order, Tab visits its stops, one Undo restores the typed name, a repeated placeholder is typed at
  once, and a refused body is listed with its reason.
- GUI (`code-gui` `a_snippet_completion_expands_in_order_and_tab_visits_its_placeholders`): real keystrokes for
  typing, Tab acceptance, Tab through the stops, the painted status bar, the last Tab, Undo, a repeated
  placeholder typed at three selections, Escape, and Tab indenting afterwards.
- Terminal (`code-tui` `snippet_placeholders_are_tabbed_through_and_escape_ends_them`): the same replay with
  Ratatui events, the footer drawn with the hint, and the file bytes unchanged.

## Limits

- Snippet variables and transforms are refused rather than approximated; nested placeholders are refused.
- Only the first choice of `${1|a,b|}` is inserted; there is no chooser.
- Mirrored editing is not implemented: a repeated placeholder is linked when visited (one selection per
  occurrence) and by its inserted text, not while typing in one occurrence before Tab.
- No snippet library, trigger-word expansion or Tab-stop persistence; a snippet does not survive a restart,
  navigation or a split change.
- Native macOS and Wayland GUI editing remain gated, so their key delivery is unverified; the terminal checks
  used a Ratatui test backend and the GUI checks the GPUI test platform under an owned display. No performance
  measurement or benchmark was run for this ticket (per the current verification policy).
