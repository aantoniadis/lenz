# 27: Indent, pair, comment, move and duplicate lines

**What to build:** Perform common structured text edits in either adapter using project conventions.

**Blocked by:** 25: Respect project editing and formatting preferences.

**Status:** resolved

- [x] Indent/unindent, paired delimiters and comment toggling respect language/context and project settings.
- [x] Move/duplicate lines and selections preserve line endings, positions and meaningful one-action undo.
- [x] Actual GUI/TUI mappings agree for empty files, boundaries, multiline Unicode selections and read-only/modal states.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 12, 13, 14.

## Answer

Merged in daefa89 (branch work/editor-27-structural-edits, 39f7e74, from 569e94c; clean). One core implementation behind `Command::{Indent,ToggleComment,MoveLines,DuplicateLines}` (ALL = 164, Selection group, shared disabled reasons): indentation follows EditorConfig and the project's ticket-25 preference (re-read, not cached); comment syntax is chosen by language (with an honest refusal where a file type has none); pairs wrap selections, step over closers and stay literal in Markdown/YAML/TOML/plain text; move and duplicate rotate contents while leaving each line ending at its position, so CRLF, LF, mixed endings and a missing final newline survive byte-for-byte; each action is one undo step and never writes the file. Routes: Tab/Shift+Tab (fixed editing keys), Toggle Comment `Ctrl+/` and `Alt+/` (terminal-safe), Move `Ctrl+Shift+Up/Down` (Alt+Up/Down under VS Code), Duplicate `Alt+Shift+Up/Down`, all in the ☰/F10 menu and palette. **Key decision:** `Alt+Shift+Up/Down` now duplicates, so Add Cursor Above/Below moved to `Ctrl+Alt+Up/Down` in both presets (ticket 29's follow-up anticipated this clash); Tab also now inserts the resolved indent step instead of a literal tab. Filtered core (15 structural) / GUI / TUI + parity replay passing; clippy clean; panic-site ratchets ok. Limits: comment toggling and pairs do not parse strings, no un-pair, endings-at-position rather than per-line terminators — see "Structural edits (ticket 27)" in docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.
