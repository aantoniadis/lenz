# Lenz command family

Lenz 0.0.2 ships one workspace and a small launcher over four independent applications; this page began
as the trial that brought the Reader sources into the Lenz workspace, and keeps that record below.
Release packages install the launcher as `lenz` beside `lenz-code`, `lenz-code-tui`, `lenz-diff` and
`lenz-diff-tui`; from a checkout, `scripts/try-launcher.sh` stages the same names.

```sh
scripts/try-launcher.sh
target/launcher-trial/lenz path/to/source.rs
target/launcher-trial/lenz --tui path/to/source.rs
target/launcher-trial/lenz diff --no-index left.txt right.txt
target/launcher-trial/lenz diff --tui --no-index left.txt right.txt
```

The script prints the actual staging directory, including when Cargo uses a custom target directory.
For a shell-local trial, prepend that directory to `PATH`; nothing is installed globally.

The staged names are `lenz`, `lenz-code`, `lenz-code-tui`, `lenz-diff` and `lenz-diff-tui`. Each application is a
separate executable and can also be launched directly. The launcher has no dependencies beyond
Rust's standard library. It finds its siblings beside its own executable, preserves the working
directory, environment and arguments, and replaces itself with the selected app using Unix `exec`.
The two terminal apps each build separately without GUI dependencies.

`--tui` selects the terminal reader immediately after `lenz`, or the terminal comparison app
immediately after `lenz diff`. Only that first selector is consumed; remaining arguments are
forwarded unchanged, including option values and paths named `--tui`.
For example, `lenz --tui diff` reads a path named `diff` in the terminal;
`lenz -- --tui` reads a literal `--tui` path in the desktop reader, and
`lenz diff -- --tui` compares that path in the desktop app.
`lenz --help` describes the command family; `lenz diff --help` reaches the existing comparison help.
Bare `lenz` opens the reader in the current directory; a path opens that file or folder.
Only `diff` is a reserved application selector. Paths named `code`, `log` and `merge` go to
the reader like other paths; use `lenz ./diff` or an absolute path to read something named `diff`.

## Migrating from 0.0.1

`lenz` now opens the reader. Comparison commands need the `diff` prefix, or the separate
`lenz-diff` executable:

| 0.0.1 command | 0.0.2 command |
| --- | --- |
| `lenz left right` | `lenz diff left right` |
| `lenz log` | `lenz diff log` |
| `lenz merge ...` | `lenz diff merge ...` |
| `lenz pr <n>`, `lenz report --last` | `lenz diff pr <n>`, `lenz diff report --last` |
| `lenz-tui ...` | `lenz diff --tui ...` or `lenz-diff-tui ...` |

Git difftool and mergetool command strings likewise become
`lenz diff "$LOCAL" "$REMOTE"` and
`lenz diff merge "$BASE" "$LOCAL" "$REMOTE" "$MERGED"`.
Lenz changes no installed Git configuration: update yours by hand. Reader arguments are forwarded to its existing
executable; use the explicit Diff command for comparisons instead of supplying two paths to the reader.

Reader's code is under `crates/reader-core`, `crates/reader-gui` and `crates/reader-tui`.
Both reader adapters share the reader core; terminal Code and Diff have no GUI dependencies.
Reader documentation is [here](code/README.md), with terminal controls and capability limits
[in the TUI guide](code/tui.md). Application behavior remains in the existing respective cores.

Cargo package and build-artifact names, window branding and configuration directories remain as
they were; only release packaging installs the new names (with **Lenz Code** and **Lenz Diff**
desktop entries on Linux). In particular, `target/release/lenz` remains the Diff GUI; use the
**staged** `target/launcher-trial/lenz` to try the command family from a checkout. The staging script
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
