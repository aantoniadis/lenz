# 68: Verify the complete integrated editor release

**What to build:** Demonstrate the whole agreed editor experience and publish its verified capabilities, limits and performance evidence.

**Blocked by:** 24: Enable and verify native macOS editor closure; 26: Offer IntelliJ and VS Code keymaps with overrides; 28: Find and replace within the current draft; 33: Restore the project session around recovered work; 34: Reconcile an external file change against a dirty draft; 37: Delete recoverably and confirm permanent removal; 45: Deliver Kotlin and mixed JVM project assistance; 46: Deliver Rust editing assistance; 47: Deliver Go editing assistance; 48: Deliver TypeScript, JavaScript and TSX assistance; 52: Deliver Python environment-aware assistance; 53: Deliver schema-aware YAML editing; 54: Deliver Bash and sh editing assistance; 55: Treat environment files as editable data; 56: Edit Markdown with live preview and authoring help; 58: Deliver first-class ERB editing; 59: Deliver first-class Haml editing; 63: Use an explicit PostgreSQL metadata connection; 64: Use an explicit DuckDB metadata file; 67: Inspect and revert marked changes in the shared draft.

**Status:** claimed

- [ ] All required feature and native-platform tickets are resolved with real evidence; no missing template/SQL capability is silently downgraded to highlighting or generic suggestions.
- [ ] Pass full shared-core/real-GUI/TUI parity, existing Diff/repository/PR/merge regressions, safe-save/recovery/native-quit and actual provider/SQL-boundary scenarios.
- [ ] Repeat equivalent release startup/interaction/child-resource/memory and sustained editing/completion workloads, fix regressions or obtain explicit agreement for deliberate tradeoffs, and report unverified platforms/physical-latency endpoints.

**Spec coverage:** user stories 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118.

## Linux/shared validation — 2026-10-27

The user authorized proceeding with everything possible without macOS after ticket 58 merged.
Review, reproduce/fix shared/Linux issues, run the complete local regression gate and one release
performance/stress campaign. Native tickets 03 and 24 remain unresolved; this ticket must not be
resolved or claimed as a complete release acceptance without their required evidence. Collect
concrete Linux results and explicit remaining limits/costs before seeking any final cost decision.

## Linux/shared checkpoint — 2026-10-27

The [validation report](../../../docs/research/code-integrated-editor-linux-validation.md) records
production repairs through `ff217fc`, fixture-only follow-up `d6fd35b`, a fresh complete full gate,
62 available-provider checks, native SQL/X11/PTY safety, the complete canonical 133-case/391-metric
comparison, final Reader startup/GUI/PTY/shared-history results and passing 900-second reading stress.
Integrated editing stress was interrupted before completion; its full result, template/optional-source
costs, absolute performance checks, focused repeats and final artifact rechecking remain pending.
The validation branch is pushed; these repairs are not merged into integration. The single cost decision
is pending. Native tickets 03/24 and full 68 remain open, with 65 of 68 tickets resolved.

## User-directed integration merge — 2026-10-27

Merged at `422cb9e5337f88750f78f6c22186988da4b0b645` at the user's direction. The merged tree
matches the validated branch exactly and retains ticket 58's ancestry. Full integrated editing
stress now passes (900.110 seconds, 30,980 iterations, -1.00 MiB tail growth),
with exact draft/disk/history/completion/tab-release assertions. Template costs also passed.
Remaining performance follow-ups are running; the full-campaign cost decision and native tickets
03/24 remain open. Ticket 68 stays claimed, with 65 of 68 resolved.

## Completed Linux measurements — 2026-10-27

The [Linux validation report](../../../docs/research/code-integrated-editor-linux-validation.md)
records the review/repairs, fresh complete local gate, available real-provider cohort, native X11/PTY
and read-only SQL boundary checks, and paired release performance/stress campaign. Final Reader
runtime is `ff217fc8f7b1dafbffd1f4abe2ac8456393ea60b`; `d6fd35b8d2d99c88ffc515eeb188d4f9fb4d4380`
changes only a real-provider fixture assertion/comment. Canonical Diff/core/TUI measurements use
`b072d0ea8ac141b260aa37eda5e5266018d663db`, with production subtrees unchanged by later Reader work.
The report and complete canonical metric summary keep measured costs and failed/inconclusive
attempts explicit. The single cost decision remains pending; no tradeoff is silently accepted.

Native macOS tickets 03 and 24 remain unresolved, blocking complete ticket 68. Wayland editing stays
gated. Four fresh Kotlin opt-ins were unavailable from official sources and are excluded from the
passing count; prior ticket 45 evidence remains separate. This Linux result does not tick full native
release acceptance or resolve the spec. There remain 65 resolved tickets out of 68.

The user directed integration merge `422cb9e5337f88750f78f6c22186988da4b0b645`; its code tree matches the validated branch and preserves ticket 58. Subsequent measurement completion is recorded without inferring full native release acceptance.

The unchanged absolute performance gate remains failed: view construction also exceeds its ceiling on the baseline; the original candidate regex search also exceeded 25 ms. Individual remainder checks and five alternating search pairs are reported separately, never as a stitched passing gate.
