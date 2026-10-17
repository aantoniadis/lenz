# Shared Reader and Diff Features Implementation Plan

> **For agentic workers:** Use test-driven implementation of each task and review the integrated branch.

**Goal:** Close the agreed appearance, find and GUI-control sharing gaps while preserving separate application cores.

**Architecture:** Common non-GUI behavior belongs in lenz-content, common GPUI presentation in lenz-gpui. Reader and Diff retain document/session models; Diff GUI/TUI consume the same core behavior. No new dependencies or installed configuration changes.

**Authority:** User approved sharing appearance preferences, find behavior and reusable GUI controls. Routine compatibility and implementation choices below are agent decisions.

## Tasks

- [x] Appearance: share lenz theme selection/user themes and font preferences. Preserve legacy Reader theme selection/themes as read-only fallbacks when canonical Lenz values are absent; canonical values win. Read LENZ_THEME/LENZ_APPEARANCE in both, with legacy Reader environment variables supported as fallback. Keep product-specific settings, recents, providers and documents independent. Font changes take effect next startup; open document state remains independent.
- [x] Find behavior: share bounded collection (10,000 results), next/previous and anchor navigation, counter formatting and option descriptions in lenz-content. Keep cancellable Reader workers and Diff's existing work orchestration. Test empty/invalid queries, exact cap versus truncation, cancellation, Unicode and wrapping; exercise both real adapters and Reader.
- [x] GUI control: share the in-document find bar's geometry, painting and hit testing in lenz-gpui. Keep input routed through each core's command interface, preserve modal behavior and keep the bar usable at narrow widths. TUI uses the same find data/status with its own cell layout.
- [x] Integration: run workspace/core tests, real Diff GUI tests, Reader GUI tests, TUI event/render tests and scripts/ci-local.sh. Preserve baseline release binaries and compare equivalent startup, find/input, memory and repeated workloads on private fixtures/displays/PTYs. Record evidence and limits; update relevant docs and performance notes.

## Review focus

- Conflicting canonical/legacy appearance files: canonical wins without rewriting legacy files.
- Invalid canonical settings: show notice/default, do not conceal corruption with legacy fallback.
- Shared result limits: distinguish exactly 10,000 matches from a truncated result set.
- Stale/cancelled Reader searches cannot update a different document.
- Narrow controls and modal clicks cannot modify documents or underlying controls.

Baseline: experiment/lenz-launcher at 9dedeb8; preserved executables and evidence under target/shared-features-evidence. Existing gate passed at this revision. Native macOS and real language servers remain outside this change's verified scope.

Completed and verified locally. See [verification and measurements](../../shared-features-performance.md); no release or installed settings migration was performed.
