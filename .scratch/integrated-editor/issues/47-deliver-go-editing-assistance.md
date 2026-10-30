# 47: Deliver Go editing assistance

**What to build:** Use module-aware Go completion/imports, diagnostics, quick fixes and formatting.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** resolved

- [x] Resolve the project’s installed/configured Go language and formatting tools without app-driven installation.
- [x] Prove real module/dependency context and current-file edits for the accepted capabilities.
- [x] Both adapters expose readiness, actions and failure behavior; absent tools stay unverified rather than passing acceptance.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 77.

## Answer

Merged in 69d023a (branch work/editor-47-go). Go had no language server in Lenz; gopls is now a provider (Go files, root at the nearest `go.work`, else `go.mod`, version from `gopls version`, tested v0.24.0, setup text naming `go install golang.org/x/tools/gopls@latest`; Lenz never installs it). Verified with real gopls v0.24.0 (installed for this ticket with `go install`) and Go 1.24.7 on a module requiring `github.com/google/uuid`: problems for the shown version, Organize Imports adding `strings` as one Undo step, completion of the dependency's members (module context), parameter information, gofmt through Format Document, and a format answered after an edit refused; the file on disk unchanged. Without gopls, Server Details names the install command and editing goes on.
Evidence: ignored real tests `real_gopls_editing` (crates/code-core/tests/providers.rs) and `real_gopls_organize_and_format_match_in_both_adapters` (Ctrl+Alt+O, Ctrl+Alt+L, Ctrl+Z in both adapters; crates/code-tui/src/parity_tests.rs), and `a_missing_gopls_says_how_to_install_it_and_editing_goes_on`, all passing here. Core lib filter 79, TUI 15 and reader GUI 7 passed; clippy clean; panic-site ratchets at baseline. Found and recorded: gopls v0.24.0 guesses a bare `"uuid"` import with Go 1.24 (also on its command line). Multi-module workspaces, cgo, build tags, native macOS, Wayland and physical terminals unverified (docs/code/providers.md, gopls; docs/editor-follow-ups.md, ticket 47).
