# Historical research

These reports were produced during the initial Lenz design work on October 1–2,
2026, and imported on October 5, 2026. Their original text, measurements, source
references and screenshots are preserved unchanged. Version numbers, proposals,
open questions and machine-specific probe paths describe that research period.
Use the root [README](../../README.md) for current behavior and installation.

The chosen designs are implemented in the application. Ticket paths under `.scratch/`
named in these notes refer to planning records retired before release (kept in the private history). Importing this evidence adds no application code,
dependencies or workspace members. In particular, the navigation experiment's flat
file list was dropped after review; it is not a pending implementation requirement.

## Source provenance

Each report was copied from its own research branch. Those branches, and the
standalone prototype applications (two-pane connectors, change-set navigation and
an image decoding/rendering probe), stay in the project's private history and are
not part of this repository. They are historical experiments, not pending work.

Screenshots: [image probe](images-in-gpui-probe.png) and
[standalone Zed editor probe](zed-editor-probe.png).

## Later research

- [Reader snapshot migration measurements](code-snapshot-migration-performance.md) (2026-10-23): isolated release comparisons, optimization experiments, full local gate and the accepted measured performance costs.

- [Native macOS termination prerequisite](native-macos-termination.md) (2026-10-23): pinned GPUI shutdown boundary, candidate consent integration and required native acceptance; implementation remains unresolved.
- [Large-workspace LSP practices](large-workspace-lsp.md) (2026-10-21): VS Code, Zed, Neovim and Helix practices, Ruby LSP indexing boundaries, project scope, reuse and server-specific caches.
- [Rust security and performance practices](rust-security-performance-practices.md) (2026-10-08): primary-source practices for a Rust viewer of untrusted repositories: terminal injection and Unicode, git/gh hardening and repo-local config execution, atomic writes, decoder limits, crash-freedom lints, supply chain, performance and ratatui/crossterm.
