# Retain syntax feedback, simplify language-server setup and accelerate search
Status: resolved

Fix syntax feedback disappearing after editing an error, switching tabs and returning.
Make missing language-server installation discoverable and straightforward in GUI
and TUI, extending the existing explicit, cancellable managed-install workflow.
Measure and speed up Find and relevant shared search paths without changing match,
Unicode, cancellation, replacement, ordering or truncation semantics.

## Acceptance

Regression checks reproduce and protect error feedback across dirty tab switches,
including pending analysis and subsequent edits. Both adapters expose the improved
setup path and installer state. Exact-byte and protocol guards remain intact.
Search improvements compare equivalent normal-release workloads with correctness
assertions, retained identities/raw evidence, startup/interaction/memory and a
sustained workload. Full local CI passes. Keep all evidence under
`perf-results/editor-diagnostics-install-search/` and report unverified platforms.


## Answer

All three implementation tickets are resolved. Linux core/GUI/TUI regressions,
the complete local gate and normal-release component/native frontend checks pass.
See the [validation report](../../docs/research/editor-feedback-setup-search-validation.md)
for exact endpoints, source/executable identities, retained failures, actual
sustained duration, memory modes, blocked live-registry checks and unverified
platforms. No merge or release is included.
