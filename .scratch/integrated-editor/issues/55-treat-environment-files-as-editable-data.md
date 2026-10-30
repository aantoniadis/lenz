# 55: Treat environment files as editable data

**What to build:** Edit environment key/value files with predictable highlighting and quoting without executing their values.

**Blocked by:** 27: Indent, pair, comment, move and duplicate lines.

**Status:** resolved

- [x] Recognize environment-file syntax and quote/comment/value boundaries in both adapters.
- [x] Typing, paste, selection and undo preserve literal values, Unicode and line endings; no shell execution or interpolation side effect occurs.
- [x] Do not expose environment contents in diagnostics, shared settings or capability logs.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 42, 89.

## Answer

Merged in b941da6 (branch work/editor-55-env). New language `Language::Env`, detected by name only: `.env`, `.env.*`, `*.env` and `.flaskenv`, unless the last extension names another language (`.env.sh` is Bash, `.env.json` JSON); `.envrc` stays plain text; the name wins over a shebang as for every language. A hand-written lexer (`crates/content/src/highlight/env.rs`) paints `#` comments, `export`, keys, `=`, single-quoted (no escapes), double-quoted (escapes painted, multi-line) and unquoted values (ending at a `#` after a blank); `$VAR`, `${VAR}`, `$(…)` and backticks are plain value text; an unclosed quote paints only its line; comment runs and multi-line values fold; Expand Selection grows quotes, value, entry. Editing is literal: Toggle Comment uses `#`; only `"` and `'` pair, and only right after an entry's `=` with nothing after it; brackets and backticks never pair; nothing expands or runs; Unicode and CRLF are kept.
Privacy: environment files go to no language server (no provider lists the language; language actions say why), the declaration index skips them, format-on-save skips them, bookmarks and recent places keep no line text (older saved text dropped when read), Server Details and Copy Server Details show fixed text; a sentinel secret was found in no file the session writes outside the project except the recovery record (the draft and last saved text, as for any file: 0600 in a 0700 folder, its name carrying no text). A server started for another file may read project files from disk itself; Lenz sends such servers paths only.
Evidence: parity test `environment_files_highlight_edit_and_save_alike_in_both_adapters` (crates/code-tui/src/parity_tests.rs: the same 12 tokens in the expected colours in the GUI's rows and the terminal's cells; a pair after `=`, typed `${X} $(id) [a] `b`` with nothing added, Alt+/, Ctrl+Z, a CRLF/Unicode paste, Server Details without file text, Ctrl+S writing the draft's bytes; nothing ran; fails with detection disabled), core tests in `session/tests/env_files.rs` (routes above), lexer tests (exact spans, CRLF, Unicode, escapes, folds, selection steps, linearity, 2,000 random inputs). Whole `lenz-content` lib set 257 passed (once more before merging); filtered core, GUI, TUI and Diff-terminal sets pass; clippy clean; panic-site ratchets at baseline. Performance deferred to the end-of-epic campaign (user decision): the whole file is re-lexed on each edit. Decisions taken: the explicit choice is the file name (Lenz has no per-file language command); backticks are not quotes; no unquoted line continuation; single quotes have no escapes (python-dotenv accepts `\'`). Limits in docs/code/README.md (Environment files) and docs/editor-follow-ups.md (ticket 55): no diagnostics for malformed lines or duplicate keys, no completion, no Compose `${VAR:?}` semantics, no per-file language override; native macOS, Wayland and physical terminals unverified.
