# 02: Share exact-ending writes with explicit save outcomes

**What to build:** Make the existing safe-write behavior reusable by the Reader without changing established Diff/merge saving.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Existing Diff dominant-ending and hard-link behavior passes its regression suite after the narrow extraction.
- [x] Reader writes preserve encoding/BOM/mixed endings/mode, validate captured canonical plus device/inode identity, reject same-path same-content hard-link substitution, and refresh identity after replacement.
- [x] Real disposable-file fault tests distinguish pre-write rejection, failed rollback and post-replacement uncertainty without claiming every error leaves disk unchanged.

**Spec coverage:** user stories 26, 27, 28, 29, 118.

## Answer

Implemented in `99ca712`, merged to `integration/integrated-editor` by `ebc9354` with no conflicts. Shared content writer preserves the existing Diff API and exposes exact-ending reads/writes with captured target identity and Saved/Rejected/Uncertain outcomes. The independent merge pass confirmed 7 exact-save tests, 25 writer regressions, 15 EditSession tests and 24 merge-session tests (one ignored), plus no test-feature leakage into production dependencies. Implementer also ran all 617 app tests and strict Clippy.

Evidence: `target/perf/editor-implementation/02/` and `target/perf/editor-implementation/merge-02/` in the integration worktree. This closes reusable writer behavior only; Reader draft retention/async save and actual GUI/TUI saving remain ticket13. Writes retain existing Unix/in-place compatibility; no filesystem-wide compare-and-swap or universal rollback guarantee is claimed.

Full integration gate passed on `177c167`: workspace/core, actual GUI/TUI adapters, panic ratchets, collector checks and separate release/size checks. This establishes the current read-only application integration; full editor performance remains the later contract/foundation acceptance work.
