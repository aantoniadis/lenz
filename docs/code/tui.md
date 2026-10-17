# Terminal reader

Lenz Code's terminal reader presents the same `reader-core` session as the GUI. In the
[command-family trial](../launcher-trial.md), use `lenz --tui path/to/file.rs` or the `lenz-code-tui` alias.
Build the adapter separately so its executable
does not include GPUI, GPU or windowing dependencies:

```sh
cargo build --locked --release -p reader-tui
./target/release/lenz-reader-tui path/to/file.rs
./target/release/lenz-reader-tui path/to/folder
```

With no argument it opens the starting folder, except home and `/`, as the GUI does. `--help` and `--version`
work without a terminal; ordinary reading needs an interactive terminal on stdin and stdout. `--` permits a
path beginning with a dash. Opening a file alone does not enumerate its parent; Open Folder starts project
browsing and project search.

## Controls

F10 or `:` opens the command palette. Type to filter, use the arrows and Enter to run. It includes all 82 shared
commands, their setting states and disabled reasons, including provider controls and features without a terminal
shortcut. F1 opens wrapped, scrollable help; F2 shows full notices. Esc dismisses a modal before underlying
controls can receive input. View switching with F8 and quitting remain global through modals. Ctrl-Q closes the current independent view; F8 and Shift-F8 switch views. New Window
in the palette opens an independent terminal view. Ctrl-Shift-Q quits all views in terminals that report it.

| Operation | Control |
| --- | --- |
| Open file / find project file / go to line | Ctrl-O / Ctrl-P / Ctrl-G |
| Find in file / next / previous | Ctrl-F or `/` / F3 / Shift-F3 |
| Find options | Alt-C / Alt-W / Alt-R in find or search |
| Search project / file pattern | Ctrl-Shift-F / Tab in search |
| Reload / close tab | F5 or Ctrl-R / Ctrl-F4 |
| Explorer visibility / focus | Ctrl-B / Ctrl-Shift-E |
| Focus tree, reading area and results | Tab / Shift-Tab; click a pane |
| Navigate tree or results | Arrows, PageUp/PageDown, Home/End; Enter opens |
| Scroll text | Arrows, j/k, PageUp/PageDown, Home/End; wheel |
| Select / copy | Click or drag, Shift-arrows, Ctrl-A / Ctrl-C |
| Extend / shrink syntax selection | Ctrl-W / Ctrl-Shift-W |
| Next / previous tab | Ctrl-Tab / Ctrl-Shift-Tab |
| Split right / down; focus pane | Ctrl-\ / Ctrl-Shift-\; Ctrl-1 / Ctrl-2 |
| Back / forward | Alt-Left / Alt-Right |
| Horizontal scroll / reorder tab | Ctrl-Alt-Left/Right / Alt-Shift-Left/Right |
| Soft wrap / Markdown source-preview | Alt-Z / Ctrl-Shift-V |
| Collapse / expand; all folds | Ctrl-minus / Ctrl-equals; add Shift for all |
| Bookmark / key / list | F11 / Ctrl-F11 / Shift-F11 |
| Set / visit numbered bookmark | Ctrl-Shift-digit / Alt-digit |
| Recent files / locations | Ctrl-E / Ctrl-Alt-E |
| Outline / declarations | Ctrl-Shift-O / Ctrl-T |
| Definition / peek / usages | F12 / Alt-F12 / Shift-F12 |
| Type definition / implementations | Ctrl-Shift-B / Ctrl-Alt-B |
| Call / type hierarchy | Ctrl-Alt-H / Ctrl-H |

The finder accepts `name:line[:column]`. Go to line accepts `line`, `line:column` or `line,column` and reports
invalid input. Find updates while typing; Enter leaves its matches available for F3, while Esc closes find.
Project search supports the same file globs, options, streamed results and cancellation as the GUI. Result
selection previews a snapshot; Enter validates and opens it. Left/Right fold result groups and hierarchy levels;
the palette changes hierarchy direction. Mouse and keyboard selection copy source bytes, including hidden folded
lines, without changing the source. A caret without a selection copies its source line.

## Presentation and terminal limits

Syntax and semantic colors, inlay hints, find and occurrence highlights, folds, sticky lines, breadcrumbs,
bookmarks, split panes, image rendering and Markdown previews consume the shared state. Markdown uses the shared
parsed blocks, inline styles and validated local images. Its rendered selection copies rendered text through the
core's Copy Text command. An SVG can switch between image and source. Images use true-color half-block cells; Markdown images fit the width and at most 20 terminal rows, without upscaling.
The terminal controls fonts, cell aspect ratio and support for italic, underline and strike. These are display
limits rather than different document behavior.

Themes use the shared Lenz configuration: `lenz/appearance.json` and `lenz/themes` under
`$XDG_CONFIG_HOME`, with `LENZ_THEME` taking precedence over `LENZ_APPEARANCE` and the legacy
Reader environment variables. Explorer order, wrap, sticky scroll, semantic settings, split orientation and saved
places use the GUI's `lenz-reader` configuration and state directories. See [Themes](../themes.md)
for precedence and compatibility fallbacks. Follow system falls back to a dark palette
when the shared theme service cannot detect appearance. There is no native file picker: Open File, Open Folder
and Workspace Root accept paths. Opening another view creates another session with its own documents and places;
it does not mirror a live document from a different view.

Clipboard transport is OSC 52: the terminal or multiplexer must permit it. Source text is base64 encoded before
entering the escape sequence. Legacy terminals cannot distinguish every Ctrl/Shift combination; use the palette
for any shortcut your terminal does not deliver. Pane dimensions are automatic, and mouse tab dragging and divider resizing are not implemented; tabs can be reordered
with the keys or moved between panes through the palette. Native GUI text shaping and operating-system primary selection
are not available in the terminal. The terminal restores raw mode, mouse/focus/paste reporting and the alternate
screen when it exits normally or returns an I/O error; an uncatchable process kill cannot run cleanup.

## Verification

```sh
scripts/ci-local.sh
cargo test --locked -p reader-tui --features gui-test parity_tests
python3 scripts/code-tui-bench.py --binary target/release/lenz-reader-tui \
  --out target/perf/reader-tui-check --lines 100000 --rounds 30 --stress-seconds 60
```

The local gate runs core tests, actual GPUI input tests, terminal event/render tests and a replay comparing both
adapters' semantic state and source bytes. The `gui-test` feature is for combined tests; leave it off the
standalone TUI build. The PTY collector generates disposable documents, isolates settings, checks normal/error
cleanup, input and modal behavior, copy, refresh, independent views and tiny resizes. It retains raw terminal
output, source/executable identity, timing samples, memory samples and actual stress duration.

PTY timings end at bytes received by the collector; they do not measure a terminal emulator's display latency.
Linux RSS includes allocator residency and excludes child language servers; macOS collection uses resident size,
not physical footprint. Neither memory samples nor one stress run prove the absence of leaks. Native macOS,
physical-terminal appearance, authenticated or installed language-server combinations beyond the existing provider
tests, and long sessions remain separate acceptance checks. The [workspace integration measurements](tui-performance.md)
record current startup, interaction and stress checks; the imported [performance record](performance.md)
describes historical Reader measurements with different workloads.
