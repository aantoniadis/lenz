# Crash reports and GitHub issues

Lenz saves a local report when a Rust thread panics, including in shipped
`panic = abort` builds. Startup, accepted comparison/repository/pull-request load
failures and terminal input/output failures also record the failed operation,
underlying error and a suggested next step. Validation notices such as an
unavailable command continue to appear in the interface.

The report starts with version/build, platform/architecture, frontend, UTC Unix
timestamp, thread and source location. Its **Cause** section preserves the first
panic in the process: a later worker-disconnection or join panic cannot replace
that original failure. Subsequent reports also point to the first report, which
has the original backtrace. **Recent operations** lists up to 32 fixed semantic
labels with elapsed milliseconds; it collects no document text, query text or
file paths. **Backtrace** captures the stack without requiring `RUST_BACKTRACE`.
Ordinary operational errors omit the stack and show their next step instead.
A disconnected document worker retains its original panic cause and backtrace.

Reports are written atomically. The directory is `$LENZ_LOG_DIR` if set, else `[tools] crash_dir` from the [settings file](../README.md#settings-and-last-position), else the platform default:

| Platform | Directory |
| --- | --- |
| Linux | `$XDG_STATE_HOME/lenz/crashes`, or `~/.local/state/lenz/crashes` |
| macOS | `~/Library/Logs/Lenz` |

Only the latest ten completed Lenz reports are retained. Each report is limited
to 256 KiB, with long error/stack fields truncated. New directories are private
and files have mode `0600` on Unix. An existing override directory keeps its
permissions; choose a private directory. If writing fails, stderr retains the
original panic output and explains how to select a writable directory.

## Open an issue draft

The report and stderr footer contain a **Report on GitHub** link. You can also
choose **Report last error** in the GUI or terminal command menu, including
repository, merge and Version Picker views. Missing reports produce an
explanation. No comparison needs to be loaded.

```sh
lenz diff report --last
lenz diff --tui report --last

# Works without a browser, display, interactive terminal or repository:
lenz diff report --last --print
lenz diff --tui report --last --print
```

The draft has a title, build/platform/frontend metadata, report identifier and
space for reproduction steps. It excludes raw error text, stack traces, local
paths and operation history. Nothing is submitted automatically. Review the
local report for private data, then attach it or paste the relevant cause and
backtrace into the issue and submit it yourself. Panic payloads and underlying
tool errors can contain private paths or data even though Lenz does not collect
document contents, command arguments, credentials or the full environment.
Home-directory strings are replaced with `$HOME`; this is not complete redaction.

GUI drafts use the platform URL opener; CLI/TUI drafts use `xdg-open` on Linux or
`open` on macOS. If the browser launcher cannot be started, use the printed link.
The browser handles GitHub sign-in; `gh` is not required for reporting.

## Coverage and verification

This covers Rust panics and the operational failures described above. Segfaults,
allocator aborts, stack overflow, forced termination and OS kills can bypass
the panic hook. They need the OS crash/core report; a saved Lenz report is not
guaranteed for those failures. An unwinding build may keep running after a worker
panic; saving its diagnostics does not repair the worker. Shipped function
symbols are retained, but full source line information for every stack frame is
not shipped. Local builds without `LENZ_BUILD_ID` explicitly report that their
revision is unavailable. Packaging sets that identifier at build time, without
startup subprocesses.

Core tests run isolated main and worker panics, report retention and CLI
retrieval. GUI tests execute the real report menu and check the opened URL and
the missing-report notice; terminal event and render checks cover the command
before loading and at narrow sizes. A smoke check uses the real terminal setup,
owned pseudo-terminals and a fake browser; it never touches the active desktop
or submits an issue:

```sh
cargo test --workspace --locked
cargo build --locked --release -p lenz
cargo build --locked --release -p lenz-tui
cargo build --locked --profile dist -p lenz --no-default-features --example crash_probe
cargo test -p lenz-tui --no-run
python3 scripts/crash-reporting-check.py \
  --panic-probe target/dist/examples/crash_probe \
  --tui-test-binary target/debug/deps/lenz_tui-<hash> \
  --output target/crash-check
```

Use the terminal test executable path printed by Cargo. The probe deliberately
aborts in disposable processes and disables core dumps. Native macOS execution
is unverified.
