# Native macOS acceptance (ticket 20)

Linux results are not native evidence. Everything here must run on a Mac; until it has, macOS support is
**unverified** and ticket 20 stays open. Nothing below has been run yet.

## 1. Prepare

- Xcode command-line tools (`xcode-select --install`), Rust (rustup), Python 3 (from the command-line tools), Git.
- A logged-in desktop session: launch and frame-pacing workloads open real windows. Close other heavy apps; keep
  the Mac on power.
- Language servers, installed by you (the reader never installs one), named for the tests. GUI apps on macOS do
  not inherit your shell's `PATH`: give the reader explicit `executable`/`env.PATH` in
  `~/.config/lenz-code/providers.json` (`$XDG_CONFIG_HOME/lenz-code/providers.json` when that is set; the same
  path on macOS as on Linux) and record what was needed.

  | Variable | Example |
  | --- | --- |
  | `READER_RUST_ANALYZER` | `$(which rust-analyzer)` (`rustup component add rust-analyzer rust-src`) |
  | `READER_PYRIGHT`, `READER_PYRIGHT_PATH` | `pyright-langserver` from npm, `PATH` holding `node` |
  | `READER_TYPESCRIPT`, `READER_TYPESCRIPT_PATH`, `READER_TSSERVER` | `typescript-language-server`; a TypeScript `lib/tsserver.js` |
  | `READER_JDTLS`, `READER_JDTLS_PATH` | `jdtls` (Homebrew), `PATH` holding Java 21+ and Python 3 |
  | `READER_RUBY_LSP`, `READER_RUBY_LSP_PATH` | `gem install --user-install ruby-lsp` (a non-system Ruby if Apple's is too old) |
  | `READER_KOTLIN` | the standalone Kotlin LSP's `bin/intellij-server` (macOS archive) |
  | `READER_NODE_PATH` | a `PATH` holding `node` (VS Code's HTML/CSS servers from `/Applications/Visual Studio Code.app`) |

  Kotlin's real tests also need Kotlin 2.2.21 in `~/.m2` (they skip otherwise, rather than download).

## 2. Run

```sh
scripts/code-macos-acceptance.sh ~/reader-macos-evidence
```

It records identity (macOS, model, CPU, memory, displays/GPU, toolchain, revisions), builds `--locked`, checks that
`code-core` has no GPUI dependency, runs the suites three times, every real-provider and real GUI test that is
configured, the pinned corpora workloads, launch and memory (`launch-bench.py --native`: physical footprint with
resident size beside it), the scroll frame-pacing probe, and the 15-minute stress (physical footprint through
`proc_pid_rusage`). Raw output for each step is kept in the evidence folder; `run.log` summarizes.

## 3. Check by hand (record each as pass, fail or not done)

- Ordinary click selects; **Cmd-click** follows a definition; neither modifies the file.
- Shortcuts with **Cmd** (Cmd+O, Cmd+P, Cmd+F, Cmd+Shift+F, Cmd+Shift+O, Cmd+T, Cmd+W, Cmd+[ / Cmd+], Cmd+Shift+P),
  **F12 / Alt+F12 / Shift+F12** with and without **Fn**, and the same commands from the command menu (F10).
- Focus and **Escape**: menus and modals take keys first; Escape closes them and cancels a pending definition.
- Preview tab reuse, keeping tabs (double-click), closing, Back/Forward, reopening a closed source from history.
- Narrow and tiny windows (drag to the smallest size); light and dark appearance (System Settings), switching while
  open.
- Text with Unicode and CRLF, a Markdown preview and its source toggle (Cmd+Shift+V), PNG/JPEG/SVG images.
- A new window (Cmd+Shift+N) has its own session.
- External changes: edit, replace and delete an open file from another editor; the reader follows, keeps the last
  content when deleted, and never writes.
- Quitting (Cmd+Q) and closing the last window stop the language servers (no leftover processes: `ps` afterwards).
- Each language: Cmd-click definition, Shift+F12 usages, a dependency opening with its origin; Ruby's launch writes
  no `.ruby-lsp/`; Kotlin to Java works and Java to Kotlin is reported as not followed.

## 4. Lenz on the Mac

The shared crates changed (`e6218d2`): run Lenz's own gate and its native UI workloads on the same Mac.

```sh
git clone https://github.com/aantoniadis/lenz && cd lenz
scripts/ci-local.sh
cargo test -p lenz --features gui-test gui_
python3 scripts/perf-suite.py --profile full --ui native --output perf-results/mac-native
```

There is no earlier native macOS baseline for Lenz, so these are absolute results; compare a before/after pair on
the same Mac before accepting a later shared-code change.

## 5. Record

Copy the evidence folder next to the Linux record and write `acceptance.md` in the same shape: one row per
ticket-20 criterion with pass, fail or unverified and its evidence. Report GPU memory as not measured unless
measured, and do not compare Mac latency numbers with the Linux reference machine.
