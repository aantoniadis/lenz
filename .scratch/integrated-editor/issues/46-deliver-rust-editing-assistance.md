# 46: Deliver Rust editing assistance

**What to build:** Use type-aware Rust completion/imports, diagnostics, quick fixes, signatures and formatting.

**Blocked by:** 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 43: Explain active tools, environments and missing capabilities.

**Status:** resolved

- [x] Verify an actual Rust project and configured toolchain with real provider fixtures.
- [x] Show member/type-aware suggestions and apply safe current-file imports/fixes/formatting with one-action undo.
- [x] Replay through both adapters, including missing/indexing/error states and cancellation during editing.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 77.

## Answer

Merged in df67744 (branch work/editor-46-rust). Verified with real rust-analyzer 1.97.0 (rustup component, rust-src, rustfmt) on a Cargo package: rust-analyzer's own problems for the shown version, the "Import `std::collections::HashMap`" assist as one Undo step, type-aware members of a `HashMap<u32, u32>`, completing `HashSet` with its merged `use` line as one Undo step, parameter information for `insert` with its active parameter, rustfmt through Format Document, and a format answered after an edit refused; the file on disk unchanged. Fixed on the way: Lenz advertised pull diagnostics, so rust-analyzer never published problems; Lenz now asks per text update (ticket 45) and answers `workspace/diagnostic/refresh` by asking again about every open text. Missing, indexing and error states and their editing behaviour are covered by tickets 43/41 (shared core and both adapters); cancellation during editing by the refused stale format.
Evidence: ignored real tests `real_rust_editing` (5.7 s) and the updated `real_rust_completion` in crates/code-core/tests/providers.rs, and `real_rust_analyzer_fix_and_format_match_in_both_adapters` (Alt+Enter, Down, Enter, Ctrl+Alt+L, Ctrl+Z in both adapters) in crates/code-tui/src/parity_tests.rs, all passing here. Filtered gate: core lib 50, semantics 13, TUI 7 and reader GUI 3 passed; clippy clean; panic-site ratchets at baseline. Open decision recorded in docs/editor-follow-ups.md (ticket 46 section): rust-analyzer runs with `checkOnSave: false`, so `cargo check` errors are not shown. Native macOS, Wayland and physical terminals unverified.
