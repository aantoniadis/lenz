# Make missing language servers easy to install in the app
Status: resolved
Type: task

Improve the missing-server entry point and installation presentation in GUI/TUI.
Use explicit install actions, meaningful prerequisite/setup choices, bounded output,
progress, cancellation, exact provider identity and guarded settings persistence.
Extend curated installation where a supported existing runtime permits it. Do not
run installs while merely opening a source or dialog. Preserve project files.


## Answer

Missing-server footer opens Install and F2 initially selects it. The shared core
provides runtime-before-server selection for curated Node recipes, progress,
Cancel, details and retry; both adapters replay the complete flow through actual
input and an owned installer process. Curated Ruby LSP 0.26.11 and gopls v0.24.0
recipes use existing toolchains/private prefixes and guarded provider-only saves.
Custom/unknown/unrelated settings and source bytes remain protected.

Real npm installation passes. A fresh offline Ruby install from official
SHA-verified packages builds native dependencies and installs six gems, then
validates and persists the Ruby LSP launch. Live Ruby index and a Go module
redirect are blocked with HTTP 403, so complete
live registry coverage is unverified, explicitly recorded in the [validation report](../../../docs/research/editor-feedback-setup-search-validation.md).
Fake-manager subprocess tests cover identity, launch and persistence guards;
owned process-tree cancellation, tiny layouts and modal input pass in GUI/TUI.
