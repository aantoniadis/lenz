# 30: Insert snippets and navigate placeholders

**What to build:** Expand basic snippets and use Tab to visit their placeholders without losing coherent multi-cursor history.

**Blocked by:** 29: Edit multiple selections coherently; 25: Respect project editing and formatting preferences.

**Status:** resolved

- [x] Expand supported snippet text with project indentation and a defined placeholder order, including repeated placeholders.
- [x] Tab visits active placeholders or indents otherwise; Escape/focus changes end snippet state safely.
- [x] Multi-cursor compatible cases work as one transaction and incompatible contexts refuse safely; real adapter tests cover undo and stale positions.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 11, 16, 66.

## Answer

Merged in 1ef0f11 (branch work/editor-30-snippets, 5ec889d, from 94bd935; clean). Completion items declaring `insertTextFormat: 2` are parsed by `crates/reader-core/src/snippet.rs` (`$1`, `${1}`, `${1:text}`, `${1|one,two|}`, `$name`, `$0`; refused whole with a reason for variables, transforms, nesting or unterminated parts) and inserted by `Session::insert_snippet` as one `EditKind::Replace` transaction with project indentation and each line's own ending. Stops visit in numbered order, then named placeholders, then the final caret; a repeated placeholder is one stop typed at every occurrence. Tab/Shift+Tab visit stops while a snippet is live and otherwise behave exactly as ticket 27's indent; Escape, a caret move, a tab switch, reload, read-only or Undo ends it, and the live snippet is revalidated against the active tab, snapshot key, editability and stop on every read, so stale positions cannot survive. Refused (change nothing, stated reason) for malformed bodies, several carets and read-only. Shared `Session::snippet_hint()` gives GUI and terminal the same `Snippet: placeholder 2 of 3 · Tab next · Esc done`; no new command. Filtered core 17 snippet + 15 structural + 12 multi-selection + 18 keymap, semantics 3 + completion 7, GUI/TUI replays passing; clippy clean; panic-site ratchets ok. Ticket 39 seam: `accept_completion` funnels into `insert_snippet`, so additional edits can join the same single transaction. Limits: first choice only, no mirrored typing between stops, no snippet library, several carets refuse insertion — see "Snippets (ticket 30)" in docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.
