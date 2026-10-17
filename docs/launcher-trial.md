# Lenz command-family trial

This branch tests one workspace and a small launcher over three independent applications.
It adds the Reader sources to the existing Lenz workspace.

```sh
scripts/try-launcher.sh
target/launcher-trial/lenz path/to/source.rs
target/launcher-trial/lenz diff --no-index left.txt right.txt
target/launcher-trial/lenz diff --tui --no-index left.txt right.txt
```

The script prints the actual staging directory, including when Cargo uses a custom target directory.
For a shell-local trial, prepend that directory to `PATH`; nothing is installed globally.

The staged names are `lenz`, `lenz-code`, `lenz-diff` and `lenz-diff-tui`. Each application is a
separate executable and can also be launched directly. The launcher has no dependencies beyond
Rust's standard library. It finds its siblings beside its own executable, preserves the working
directory, environment and arguments, and replaces itself with the selected app using Unix `exec`.
The terminal app builds separately without GUI dependencies.

`--tui` selects the terminal app when placed immediately after `lenz diff`. The remaining
arguments are forwarded unchanged, including option values and paths named `--tui`.
For example, `lenz diff -- --tui` compares that path in the desktop app.
`lenz --help` describes the command family; `lenz diff --help` reaches the existing comparison help.
Bare `lenz` opens the reader in the current directory; a path opens that file or folder.
Only `diff` is a reserved application selector. Paths named `code`, `log` and `merge` go to
the reader like other paths; use `lenz ./diff` or an absolute path to read something named `diff`.

## Migrating existing comparison commands

The staged launcher now defaults to the reader. Existing comparison commands need the `diff`
prefix, or the separate `lenz-diff` executable:

| Previous command | Staged launcher command |
| --- | --- |
| `lenz left right` | `lenz diff left right` |
| `lenz log` | `lenz diff log` |
| `lenz merge ...` | `lenz diff merge ...` |

Git difftool and mergetool command strings likewise become
`lenz diff "$LOCAL" "$REMOTE"` and
`lenz diff merge "$BASE" "$LOCAL" "$REMOTE" "$MERGED"`.
This trial changes no installed Git configuration. Reader arguments are forwarded to its existing
executable; use the explicit Diff command for comparisons instead of supplying two paths to the reader.

Reader's code is under `crates/reader-core` and `crates/reader-gui`, with local dependencies on
`lenz-content` and `lenz-gpui`. Its documentation is [here](code/README.md).
The existing comparison GUI/TUI parity rules apply to the two Diff frontends; this trial adds
no terminal Code frontend. Application behavior remains in the existing respective cores.

This is a local command/workspace experiment, not a release migration. Existing Cargo package
and build-artifact names, window branding, configuration directories, desktop entries and release
packaging remain as they were. In particular, `target/release/lenz` remains the original GUI;
use the **staged** `target/launcher-trial/lenz` to try the command family. The staging script
creates aliases to the built applications and copies the tiny launcher; it does not combine
their dependencies or retain an extra launcher process.

## Initial trial validation (`c030df9`)

The full `scripts/ci-local.sh` gate passed, including workspace/core tests, the Diff GUI tests,
Reader's GUI tests, both panic-site budgets, collector tests and the original release size check.
The launcher tests went red before implementation and now verify app selection, literal/empty/raw
arguments, working directory, environment, unchanged PID, exit status, help and missing executables.
Reader's imported code has its own panic-site baseline so the existing Diff budget is unchanged.

Real-app checks on Linux used disposable files, an owned private Xvfb display and an owned PTY.
They verified readable Code and Diff windows, Code's Find input, terminal input/resize/idle output,
Ctrl-C/error cleanup, and the executable reached after `exec`. Fixture bytes remained unchanged.
Three alternating baseline/current runs measured Diff first paint at 128 → 129 ms and terminal
first content at 6.70 → 6.54 ms. Code window creation was 41.03 → 40.84 ms; this measures window
creation, not readable content or physical display latency. Sampled process memory stayed in
overlapping ranges. A 10-second terminal scroll/input run held near 12.2 MiB RSS; it is not a
long-session leak test or a measurement of GPU memory.

Commands, raw samples, executable/source identity and screenshots are retained locally under
`target/launcher-trial-evidence`; the full gate log is `target/launcher-trial-ci.log`.
The first Diff screenshot OCR check missed colored changed rows; visual inspection confirmed them,
and the focused rerun verified readable source text and the correct GUI executable.

Native macOS, real language-server integration, the full performance matrix and release/install
migration need separate validation before adopting this experiment. Existing reader provider
integration tests remain opt-in; this import does not claim to reverify their documented results.

## Default-reader update

The staged launcher now uses the reader as its default. Updated dispatch tests went red on the
old default and passed after the change; the full local gate passed again. Private-display checks
verified bare `lenz`, a file path, a folder path and a literal file named `code` all reach the
reader, with unchanged arguments and fixture bytes. The reader's Find input and explicit desktop
Diff launch also passed; both explicit Diff help routes still select the correct frontend.
Three alternating runs compared the same Reader executable directly and through the launcher;
window creation and sampled process memory stayed comparable. This is window creation, not a
new measurement of readable content or physical presentation.

The rebuilt trial is ready in `target/launcher-trial`. Updated evidence is under
`target/launcher-default-reader-evidence`, with gate output in `target/launcher-default-reader-ci.log`.
The initial trial's evidence above is retained separately. Native macOS and installed Git tool
configurations were not changed or revalidated.
