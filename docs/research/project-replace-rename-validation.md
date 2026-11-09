# Project replacement and symbol rename: validation

The implementation is on `feat/project-replace-and-rename`, based on
`b69b3b44c62b755377bd7a3bed4e666a38b9d6f5` after merging PR #50. Acceptance is **incomplete**. The
[remaining validation ticket](../../.scratch/project-replace/issues/01-final-validation.md)
keeps the feature spec open.

The earlier passing evidence below predates this rebase and was collected against the previous
base, `ea2d782ca38924757f5fb28b295e8874c57cc013`. The rebase retains PR #50's
scratch-kind picker, navigation changes and commands alongside the replacement
and rename workflows. It requires fresh compilation and workflow validation;
earlier passing runs do not establish correctness of the combined changes.

Rebase verification: the resolved Rust files parse, the command catalog and its
expected-label list retain all 260 commands (256 from updated main plus four
workspace-edit commands), and `git diff --check` passes. The combined release
`cargo check` for core, GUI and TUI, including all targets and the GUI/TUI replay
feature, failed while compiling `bindgen`: a compiler worker could not be created
(EAGAIN). This is not a passing compilation or workflow test. The attempt is
retained under `perf-results/project-replace/rebase-pr50/`.

The shared core prepares bounded replacement/rename plans, validates all included
sources before applying any changes, and edits normal drafts. Both adapters expose
replacement/new-name inputs, a scrollable preview, file inclusion and Apply/Cancel
controls. Normal Save and per-file Undo retain their existing responsibilities.

## Evidence and limits

Raw logs, failed attempts, executable identities and the final source manifest are
retained in the ignored `perf-results/project-replace/` directory. They are local
evidence, not release artifacts.

| Check | Result |
| --- | --- |
| Initial core/GUI/TUI compilation | Passed before the final safety additions. |
| Strict workspace Clippy, ordinary and adapter-test feature graphs | Passed before the final safety additions; see `final-checks.log`. That script subsequently failed in tests. |
| Earlier focused core checks | Ten cases passed across isolated runs, including a real fake-LSP rename round trip, dirty draft preservation, distinct buffer identities, guarded Apply, Undo and legacy-encoding Save. These results predate the final server-snapshot and expansion guards. |
| Earlier GUI/TUI feature replay | All three new input/parity tests passed in `parity-3.log`, including both appearances and narrow layouts. Later additions to cancellation, scrolling and typing assertions still need a final passing replay. |
| Diff GUI regression checks | 291 ordinary cases passed in isolated processes, with three ignored; `diff-gui-isolated2.log`. |
| Python collectors | 77 tests ran, one skipped; `python-collectors-final.log`. Collector definitions were unchanged. |
| Full local CI | No complete passing run. Several real failures found during development were fixed; subsequent runs failed on process/thread creation. |
| Final core production compilation | Succeeded during `standalone-minimal-processes.log`; the TUI then reached code generation and failed to create a compiler worker. |
| Final focused core test build | Blocked by compiler worker creation in `serde_derive`; `core-minimal-build.log`. |
| Final GUI production build | Blocked by compiler worker creation in font/accessibility dependencies; `gui-release-union.log` and `gui-release-final.log`. |
| Final production Clippy attempt | Blocked by compiler worker creation in the `cexpr` build dependency; `final-production-clippy.log`. |

The environment contains about 32,000 unreaped processes. EAGAIN failures affected
compiler workers, syntax/search workers and `notify` watcher threads. A workspace
reconnect also interrupted a build. Serial Cargo jobs, single-CPU affinity and
fewer supervising processes did not eliminate these failures. Cargo profiles,
compiler flags and production dependencies were not changed to obtain a result.

The `pg_parse` native library built normally, but its upstream pthread stress test
failed at the same process ceiling. Its build cache was reused after checking that
copied sources and the library bytes were unchanged and restoring copied-source
mtimes. That upstream stress test remains unpassed; cache reuse is not a successful
test result. The proof and limitation are in `pg-cache-repair.json`.

## Performance acceptance remains open

An initial standalone production build succeeded before the last changes. A final
production GUI and final test executable could not be completed. Consequently the
planned final native GUI replay, paired GUI/TUI startup/input/memory comparisons
and repeated 40-file/4,000-edit apply/undo probe have **not run**. The retained
baseline is the normal release build under
`perf-results/fix-rigorous-2026-11-09/candidate/`; final candidate performance has
not been established. Existing README metrics remain unchanged, and no performance
preservation or speedup claim is made for this feature.

The prepared collectors retain source/executable hashes, raw samples and actual
stress duration. Their endpoints distinguish PTY bytes, XDamage feedback and a
mapped window from physical display latency or completed analysis; process RSS
does not measure GPU memory or prove leak freedom. Native macOS and Wayland
execution are unverified. Final acceptance requires the outstanding ticket's
checks on a healthy environment.
