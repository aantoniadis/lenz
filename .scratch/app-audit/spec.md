# Code branch picker and GUI/TUI workflow audit
Status: needs-info

Audit current 0.0.14 GUI and terminal workflows using disposable fixtures. Fix
reproducible integration problems and make Code's Git picker responsive with
distinct local branch, remote branch and worktree groups, using shared behavior.
Keep raw native-input evidence, failures, release measurements, identities and
process snapshots in perf-results/app-audit/. Preserve normal Cargo profiles and
standalone TUI builds. Keep merging and release publication outside this task.

Implementation tickets 01–03 are resolved; complete CI and native replays pass.
Ticket 04 awaits explicit acceptance of the measured feedback tradeoff required
by AGENTS.md. The experiment was reverted; validated runtime files/binaries are
unchanged. See the [audit report](../../docs/research/code-branch-picker-audit.md).
