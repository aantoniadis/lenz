<p align="center">
  <img src="docs/images/lenz-logo.png" alt="Lenz logo" width="128" height="128">
</p>

<h1 align="center">Lenz</h1>

<p align="center">Side-by-side diffs, repository history and three-way merges, in a native window or your terminal.</p>

![Lenz GUI comparing working-tree changes in a Rust project, with the changed files on the left and word-level highlights between the two sides](docs/images/gui-comparison.png)

A fast, native side-by-side diff viewer, git client and merge tool for Linux and macOS, built in Rust. Lenz has two frontends over one shared core:

- **`lenz`**: a GPU-rendered desktop app built on [gpui](https://github.com/zed-industries/zed).
- **`lenz-tui`**: a terminal app built on [Ratatui](https://ratatui.rs), without any GUI dependencies.

Run either with the arguments you'd give `git diff`, or give it two files or two directories, and it opens the Change Set. `lenz log` browses the repository and pull requests, and `lenz merge` (or `git mergetool`) resolves conflicts.

**Status:** beta (0.0.1). GUI and TUI workflows are implemented and tested on Linux and macOS (Apple Silicon); automated GUI interaction coverage is Linux-only. See the [macOS figures and known costs](docs/performance.md#macos).

## Screenshots

| | GUI (`lenz`) | Terminal (`lenz-tui`) |
|---|---|---|
| **Repository** | [![Repository window with branch graph, commit log and the selected Change Set](docs/images/gui-repository.png)](docs/images/gui-repository.png) | [![Terminal repository view with the same log and Change Set](docs/images/tui-repository.png)](docs/images/tui-repository.png) |
| **Merge** | [![Three-way Merge View with Ours, Result and Theirs](docs/images/gui-merge.png)](docs/images/gui-merge.png) | [![Terminal Merge View](docs/images/tui-merge.png)](docs/images/tui-merge.png) |
| **Compare** | [![Markdown Preview comparison](docs/images/gui-markdown.png)](docs/images/gui-markdown.png) | [![Terminal comparison with syntax highlighting and files list](docs/images/tui-comparison.png)](docs/images/tui-comparison.png) |

## Features

- **Compare** files, directories, revisions, the index or the working tree, with word/character/line highlighting, whitespace and case modes, three scroll modes, syntax highlighting and blame annotations.
- **Apply changes**: move Change Blocks with ≫ / ≪ and revert hunks. Changes stay as a draft until you save. Every applied change stays visible as Applied or Undone, with selective Undo.
- **Browse** the repository: commit log with branch graph, file history (`--follow`), Change Set search and read-only GitHub pull requests with descriptions and review threads.
- **Merge**: a three-way Merge View, a Conflict List for whole merges/rebases/cherry-picks, and a Version Picker for binary files. The Merge View applies every non-conflicting change as soon as it opens and never drops a conflict.
- **Images**: two-up, swipe, onion skin and difference modes, with zoom and opacity.
- **Markdown Preview**: an Obsidian-like reading view (callouts, properties, highlights, tags, wiki links, footnotes, local images), Inline with removed words struck in place or Side by side, with change bars beside each changed block; every source change stays reachable. The terminal app shows it too, with images as real pixels in kitty and Ghostty.
- **Themes**: Darcula, Light, Catppuccin, Tokyo Night and Gruvbox, system/Omarchy following and custom TOML themes.
- **Safe writes**: atomic, keeping encoding, BOM, line endings and file mode, and never over a file that changed on disk since Lenz read it.

## Install

Builds for macOS (Apple Silicon) and Linux (x86_64, aarch64) are on the [GitHub Releases page](https://github.com/aantoniadis/lenz/releases); the current one is a beta.

- **Homebrew** (macOS arm64, Linux): `brew install aantoniadis/lenz/lenz`, from the tap's formula (`packaging/homebrew/lenz.rb`).
- **Arch Linux**: the AUR package `lenz-bin` (`packaging/aur/PKGBUILD`, not published yet), e.g. `paru -S lenz-bin`.
- **Tarball**: `lenz-<version>-<target>.tar.gz` from the [releases](https://github.com/aantoniadis/lenz/releases) holds `lenz`, `lenz-tui`, this README and the licences; put them on your `PATH`. On macOS the GUI is inside `Lenz.app` (so the Dock shows its icon) and `lenz` is a symlink into it: keep the two together, or link to `Lenz.app/Contents/MacOS/lenz`.
- **From source**: `cargo install --locked --git https://github.com/aantoniadis/lenz lenz` (or `lenz-tui`); see [Build](#build) for the system libraries.

Every package installs both `lenz` and `lenz-tui`. Linux binaries need glibc 2.35 or later.

Linux tarballs include the logo and `lenz.desktop`; the AUR and Linux Homebrew packages install them. The launcher opens the repository chooser described below.

## Build

Needs a stable Rust toolchain and git 2.30 or later at runtime.

```sh
cargo build --release --locked -p lenz       # GUI: target/release/lenz
cargo build --release --locked -p lenz-tui   # TUI: target/release/lenz-tui
```

On Linux, gpui needs: `libxkbcommon-x11-dev libwayland-dev libfontconfig-dev libx11-xcb-dev libvulkan1 libglib2.0-dev libzstd-dev` (Debian/Ubuntu names). On macOS, full Xcode with the Metal toolchain (`xcodebuild -downloadComponent MetalToolchain`).

### macOS app bundle

`cargo build` makes a bare `target/release/lenz`. It works, but the Dock shows a generic icon while it starts and quits, and the Lenz logo only once the window is up. For the logo throughout, build the release package, which puts the GUI in `Lenz.app`:

```sh
packaging/package.sh 0.0.1
```

The bundle is at `target/package/lenz-0.0.1-aarch64-apple-darwin/Lenz.app`, next to a `lenz` symlink into it; the same folder is packed as `target/package/lenz-0.0.1-aarch64-apple-darwin.tar.gz`. `cargo build` does not update it: run the script again after changes. The argument is the bundle's version (Finder's Get Info) and goes into the file names; About shows the workspace version.

```sh
open target/package/lenz-0.0.1-aarch64-apple-darwin/Lenz.app                     # no files, as from Finder
target/package/lenz-0.0.1-aarch64-apple-darwin/lenz path/to/left path/to/right  # a comparison
```

To install it, copy `Lenz.app` to `/Applications` and put a `lenz` symlink to `/Applications/Lenz.app/Contents/MacOS/lenz` on your `PATH` (as the Homebrew formula does); launched through that symlink, from a terminal or as a git difftool, it still shows the logo.

## Usage

`lenz` and `lenz-tui` take the same arguments. The git grammar mirrors `git diff`:

| Command | Shows |
|---|---|
| `lenz` | working tree vs index (unstaged changes), plus untracked files |
| `lenz --staged` (or `--cached`) | index vs HEAD |
| `lenz <rev>` | working tree vs `<rev>`, plus untracked files |
| `lenz A B`, `lenz A..B` | revision `A` vs revision `B` |
| `lenz A...B` | merge base of `A` and `B` vs `B` |
| `lenz ... -- <path>...` | only these paths (relative to the current directory) |
| `lenz <left> <right>` | two files, or two directories paired by relative path |
| `lenz --no-index <left> <right>` | the same, even if an argument names a revision |

Browsing the repository:

| Command | Opens |
|---|---|
| `lenz log [<rev>...] [-- <path>...]` | the Repository Window: sources sidebar, commit log with branch graph, the picked Change Set below |
| `lenz log --follow <file>` | one file's history |
| `lenz pr [<number>\|<url>]` | a GitHub pull request, read-only (needs `gh`, run inside a clone) |
| `lenz` in a repository without changes | the Repository Window on HEAD |

Resolving conflicts:

| Command | Opens |
|---|---|
| `lenz merge [<path>...]` | the Repository Window on the **Conflict List** of the merge, rebase, cherry-pick, revert or stash pop in progress (plain `lenz` does too while one is) |
| `lenz merge <base> <local> <remote> <merged>` | one file in the **Merge View** (what `git mergetool` runs); works outside git too |

Diffs given on the command line (and `git difftool`) open with the branches and history hidden (Files shows when there is more than one file); Esc shows the picker. **Focus** (the toolbar button, `f`, Enter in the log, or a double click on a commit) hides Branches and History, leaving Files and the diff; Focus again or Esc restores them. Two arguments are compared as plain files when both exist on disk and neither names a commit. In directory mode, byte-identical files at different paths are shown as renames.

Options:

- `--no-untracked`: hide untracked files.
- `--name <label>`: label for a single Comparison (may be empty).
- `-w`, `--ignore-all-space`; `--whitespace=default|trim|ignore|ignore-chunks`: starting Whitespace Mode.
- `-i`, `--ignore-case`: start with Ignore Case on.
- `--highlight=word|char|line`: starting Highlight Level; `--scroll=shared|aligned|padded|independent`: starting Scroll Mode (Aligned keeps matching lines level while each side scrolls its own lines; Independent never moves the other side).
- `--blame`: start with Blame Annotations on; `--line <n>`: open the first file at line `n` of the right Side.
- `--about`: show version, platform and licence information without opening a comparison.

**Outside a repository**, `lenz`, `lenz log`, `lenz merge` and `lenz pr` (and the same in `lenz-tui`) open a repository chooser instead of failing: recent repositories (those Lenz has saved a place for), a path field with Tab completion, and in the GUI **Browse…** (Ctrl+O) for the system folder dialog. Choosing one reruns the same command inside it; Esc quits with exit code `0`. Two files or folders still compare anywhere.

Lenz stays in the foreground until its window closes. Exit codes: `0` on a normal close, or when there is nothing to show (it prints `no differences` and opens no window); `2` on bad arguments or startup errors (git missing or older than 2.30, `lenz-tui` outside a repository without an interactive terminal). `lenz merge <base> <local> <remote> <merged>` exits `0` only after **Done**, `1` when cancelled or closed, so `git mergetool` restores its backup.

### Settings and last position

An optional `~/.config/lenz/settings.toml` (or `$XDG_CONFIG_HOME/lenz/settings.toml`) sets defaults for both frontends. Lenz reads it at startup; **☰ → Settings…** opens it in your editor, first creating it with every option commented out when there is none. A bad value falls back to its default with a notice. Command-line flags and environment variables (`LENZ_EDITOR`, `VISUAL`/`EDITOR`, `LENZ_ICONS`, `LENZ_LOG_DIR`) win over it.

```toml
[gui]
font_family = "JetBrains Mono"   # default: first installed of a built-in monospace list
font_size = 13                   # 8–32

[view]                           # starting options of each new comparison
whitespace = "default"           # default | trim | ignore | ignore-chunks
ignore_case = false
highlight = "word"               # word | char | line
scroll = "shared"                # shared | aligned | padded | independent
blame = false
markdown = "source"              # source | preview
markdown_layout = "inline"       # inline | side-by-side (Preview)

[tools]
editor = "nvim --clean"
icons = "nerd"                   # plain | nerd (TUI)
crash_dir = "~/lenz-crashes"

[session]
restore_place = true
```

Plain `lenz` and `lenz log` in a repository reopen where you left off: the selected commit, the file and the line. Anything that no longer exists falls back to the normal start, and a merge or rebase in progress always opens its Conflict List. Explicit comparisons, `lenz pr`, `lenz merge`, difftool and mergetool always open what they name. Positions are kept privately in `~/.local/state/lenz/places/`; set `restore_place = false` to turn this off.

### In the window

- `?` lists every key binding. The command palette (Ctrl/Cmd+Shift+P, F10, `:` in the diff, or the **Commands** hint at the bottom right; `⋯` in the TUI) lists every action; type to filter, and an unavailable one shows why.
- The toolbar keeps the frequent actions, with diff options under **⚙** and app actions under **☰**; Branches, History and Files toggle from the bottom bar; the diff header switches Markdown and image modes. Right-click shows only actions for what is under the pointer.
- Tab/Shift-Tab move focus between panes. Typing in the Files or Commits list searches it; Ctrl/Cmd+F searches the focused text side, Ctrl/Cmd+Shift+F the whole Change Set.
- **Back / Forward** (◀ ▶, Alt+Left/Right, mouse back/forward buttons) return to the previous commit, branch, filter or file history and reopen its file and line; `[` / `]` move between files.
- Drag pane dividers to resize; **Reset pane sizes** restores the layout.
- `o` opens the file in your editor at the line (`$VISUAL`/`$EDITOR`, or `LENZ_EDITOR` when those are wrappers), `shift-o` in the default app.
- Long histories load in steps of about 50,000 commits beyond what you can see; scrolling, End, searches and filters load more.
- In a pull request, **PR details** (`d`) shows the description, discussion, review summaries and inline review threads.

The terminal frontend has the same toolbar, menus and palette, with terminal-specific keys (`:` or F10 commands, Alt+0 Focus, F8 switches independent views, F6 moves between panes, Ctrl-Q quits). Clipboard uses OSC 52 and images render as terminal cells. See [terminal controls and capability limits](docs/tui.md).

### Editing and saving

Editable text comparisons keep **Undo**, **Redo** and **Save** directly accessible, with **Discard unsaved changes** in the Edit menu. Hunk moves stay pending until **Ctrl/Cmd+S** (Ctrl-S in the TUI comparison pane). **Revert hunk** restores the left version on the right working-tree side. Drafts survive file switches and repository refresh, and Lenz asks before closing or changing comparisons with unsaved drafts.

**Discard file** is a separate confirmed Git action: it restores the index version, preserves staged changes, restores tracked deletions, and offers **Undo Git discard**.

### Merging

In the **Merge View** the Result starts from the Base with every non-conflicting change applied. Resolve each red **Conflict** with ≫ (Ours), ≪ (Theirs) — both, in the order clicked — or ✕ (keep the Base), or the keys `alt-shift-→ ← ↓ backspace`. **Resolve simple conflicts** (in the command palette) resolves Conflicts whose edits don't overlap word by word. `ctrl-s` saves; Conflicts left are written as git conflict markers in your `merge.conflictStyle`, with both versions kept, so saving early never loses a change. `ctrl-enter` is **Done**. Binary files open in the **Version Picker**: take one version whole. **Reopen conflict** resets an individual resolution.

git runs the merge tool once per file; `lenz merge` instead opens one window on every conflict (text, binary, deleted, symlink, submodule) with **Mark resolved** for each file.

### What Lenz writes

Only the files you act on: a Merge View's Result when you save, a whole version taken in the Version Picker, a text draft when you save it. Lenz stages only when you press **Mark resolved** (`git add`, or `git rm` for a deletion; **Re-open** restores the conflict markers), and never commits, checks out, continues or aborts a merge or rebase. Besides that: a pull request's objects under `refs/lenz/pr/*` (removed on exit) and, if you choose Speed up history, git's commit-graph file.

### Crash reports

A panic saves a local diagnostic report. **Report last error** in either command menu, or `lenz report --last` / `lenz-tui report --last`, opens an editable GitHub issue draft; `--print` shows the report location and link instead. The draft holds metadata only and uploads nothing. See [crash reporting](docs/crash-reporting.md).

## git difftool

```sh
git config --global diff.tool lenz
git config --global difftool.lenz.cmd 'lenz --name "$MERGED" "$LOCAL" "$REMOTE"'
git config --global difftool.prompt false
```

Prefer `git difftool -d` (for example `git difftool -d main...HEAD`): it opens one window for the whole Change Set instead of one window per file. Use `lenz-tui` in place of `lenz` for the terminal frontend.

## git mergetool

```sh
git config --global merge.tool lenz
git config --global mergetool.lenz.cmd 'lenz merge "$BASE" "$LOCAL" "$REMOTE" "$MERGED"'
git config --global mergetool.lenz.trustExitCode true
git config --global mergetool.prompt false
git config --global mergetool.keepBackup false   # optional: Lenz writes atomically, so the .orig files are just clutter
```

## Performance

Linux desktop, release builds, medians of three warm launches; the GUI on a software-rendered private X display, the TUI in a 120×36 pseudo-terminal:

| Operation | GUI | TUI |
|---|---:|---:|
| Open a small text comparison | 105 ms · 186 MiB | 8 ms · 10 MiB |
| Open a generated 100k-line comparison | 161 ms · 240 MiB | 28 ms · 55 MiB |
| Open two 4096×4096 PNGs | 187 ms · 706 MiB | 51 ms · 139 MiB |
| Open a 100k-line three-way merge | 249 ms · 307 MiB | 153 ms · 115 MiB |
| Open the repository log (500 commits) | 115 ms · 187 MiB | 7 ms · 10 MiB |
| Page Down in a 100k-line diff | 15.6 ms | 0.6 ms |
| Move through the repository log | 8.6 ms | 0.3 ms |

Opening times run to the GUI's first paint and the TUI's first comparison text; memory is RSS three seconds later (on macOS, the physical footprint). Interaction times are first feedback after the key, not a completed frame on a physical display. Syntax highlighting finishes after first content (0.5–0.9 s on 100k-line inputs in the TUI). On an Apple M2 Mac, GUI first paint is 231–401 ms; see the [macOS results](docs/performance.md#macos).

See the [performance overview](docs/performance.md) for method and accepted costs, and the [benchmark guide](docs/benchmarking.md) to run the suite yourself.

## Documentation

| Guide | Covers |
|---|---|
| [Terminal frontend](docs/tui.md) | TUI controls, architecture and capability limits |
| [Markdown Preview](docs/markdown.md) | Source, Inline and Side by side views and source fidelity |
| [Themes](docs/themes.md) | Built-in themes, system following and the custom theme format |
| [Crash reporting](docs/crash-reporting.md) | Report locations, retention and issue drafts |
| [Performance](docs/performance.md) | Current GUI/TUI results, method and accepted costs |
| [Benchmark guide](docs/benchmarking.md) | Running and comparing the performance suite |
| [Optimization notebook](docs/optimization-notebook.md) | Techniques kept, rejected and still untried |
| [Release checklist](docs/release-checklist.md) | Packaging and the manual release walkthrough |
| [Design research](docs/research/README.md) | Initial toolkit, git, diff/merge, encoding and packaging research |
| [All docs](docs/README.md) | Index of the guides and design records |

## Contributing

Checks run locally: `scripts/ci-local.sh` runs clippy with warnings as errors, the panic-site ratchet, every test suite and the release size check (`--old-git` adds the git 2.30 tests in podman or docker). Enable it before every push with `git config core.hooksPath .githooks`. `python3 scripts/capture-screenshots.py --xvfb /path/to/Xvfb` regenerates the screenshots (Catppuccin Mocha) on a private display.

## Licence

The app (`crates/app`) is MIT; the diff engine (`crates/engine`) is Apache-2.0 and contains ported IntelliJ Community code (see its NOTICE). Darcula and Light follow IntelliJ's schemes (JetBrains, Apache-2.0); additional base colors are adapted from Omarchy's MIT palettes. See [theme attribution](docs/themes.md).
