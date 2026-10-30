# 48: Deliver TypeScript, JavaScript and TSX assistance

**What to build:** Edit TS, JS and React TSX with imports, diagnostics, formatting and component/prop help.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** resolved

- [x] Verify real project configuration, dependencies, TS/JS language modes and React component/prop completion.
- [x] Apply completion/import/fix/format edits atomically to the current file and reject wider edits.
- [x] Exercise both adapters, unsaved targets, Unicode ranges and tool environment/readiness failures.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 81, 82.

## Answer

Merged in 96508e6 (branch work/editor-48-typescript). Verified with real typescript-language-server 6.0.1 (installed for this ticket with npm) and TypeScript 6.0.2's tsserver on Node.js 22 in a TS/JS/TSX project: completing another module's export with its import after a non-BMP character (UTF-16 ranges) as one Undo step, problems for the shown version, the "Update import from …" fix, parameter information, Format Document, Organize Imports (sorts and combines), JSDoc-typed members in JavaScript, and a component's props in TSX; files on disk unchanged. Fixed: its fixes arrive as `_typescript.applyCodeActionCommand`, which Lenz refused wholesale; Lenz now reads the tsserver action's text changes (this file only, no further commands such as `@types` installs) and applies them as one transaction without running any server command.
Evidence: ignored real tests `real_typescript_editing` (crates/code-core/tests/providers.rs) and `real_typescript_fix_and_format_match_in_both_adapters` (crates/code-tui/src/parity_tests.rs), passing here; unit test `tsserver_changes_are_read_for_this_file_only_and_never_with_further_commands`. Filtered gate: core lib 4, semantics passing except one environmental Ruby LSP test, TUI 6 passed; clippy clean; panic-site ratchets at baseline. Gaps (React's own types, monorepos, typescript-go editing) in docs/code/providers.md and docs/editor-follow-ups.md (ticket 48). Native macOS, Wayland and physical terminals unverified.
