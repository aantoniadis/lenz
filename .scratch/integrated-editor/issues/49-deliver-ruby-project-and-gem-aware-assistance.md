# 49: Deliver Ruby project and gem-aware assistance

**What to build:** Use Ruby constants/methods, signatures and configured diagnostics/formatting in the project’s gem context.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** resolved

- [x] Verify real bundled/non-bundled project discovery with installed gems and stable indexing startup.
- [x] Resolve provider encoding/signature behavior for exact current-file edits and demonstrate configured lint/format tools.
- [x] Expose the same supported actions/readiness in both adapters without installing tools or hiding missing capabilities.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 78.

## Answer

Merged in 6e47efd (branch work/editor-49-ruby). Verified with real Ruby LSP 0.26.11 and RuboCop 1.92.1 (installed for this ticket with `gem install`) on Ruby 3.3.6: a bundled project with RuboCop gets RuboCop's problems, its autocorrect fix and its formatting, each one Undo step, in both adapters; project constants complete on a line after a non-BMP character (UTF-16); parameter information shows `new()`; a folder without a Gemfile starts in Ruby LSP's own bundle, completes the project's constants and gets nothing written into it. Fixed: Lenz sent full-text `didChange` to servers asking for incremental sync, which Ruby LSP cannot apply (it kept the opened text); such servers now get one ranged change replacing the whole text, and every earlier real-server test was re-run and passes. Changed: Ruby LSP now uses `formatter: auto` and its default linters (the project's configured RuboCop). Also: an explicit completion waits for an identical request typing already started. Ruby LSP stays on UTF-16 (its index answers in characters); its parameter information finds nothing on a line holding an astral character (recorded).
Evidence: ignored real tests `real_ruby_editing`, `real_ruby_without_a_gemfile` and the re-run `real_ruby` (crates/code-core/tests/providers.rs), `real_ruby_lsp_fix_and_format_match_in_both_adapters` (crates/code-tui/src/parity_tests.rs); fake-server test `a_server_that_applies_only_ranged_changes_follows_edits`. Semantics filter 10 passed, the completion parity replay passed 5 runs in a row; clippy clean; panic-site ratchets at baseline. Gaps in docs/code/providers.md (Ruby LSP) and docs/editor-follow-ups.md (ticket 49). Native macOS, Wayland and physical terminals unverified.
