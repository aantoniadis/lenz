# Themes

Lenz shares semantic chrome, syntax and diff colors between its GUI, TUI and Reader.
Choose from 16 bundled themes:

| Theme | ID | Appearance |
|---|---|---|
| Darcula | `darcula` | Dark |
| Light | `light` | Light |
| Catppuccin Mocha | `catppuccin` | Dark |
| Catppuccin Latte | `catppuccin-latte` | Light |
| Tokyo Night | `tokyo-night` | Dark |
| Gruvbox | `gruvbox` | Dark |
| Nord | `nord` | Dark |
| Everforest | `everforest` | Dark |
| Kanagawa | `kanagawa` | Dark |
| Dracula | `dracula` | Dark |
| One Dark | `one-dark` | Dark |
| Rosé Pine | `rose-pine` | Dark |
| Rosé Pine Dawn | `rose-pine-dawn` | Light |
| Flexoki Light | `flexoki-light` | Light |
| Solarized Light | `solarized-light` | Light |
| GitHub Light | `github-light` | Light |

## Attribution

Darcula and Light follow IntelliJ's color schemes (JetBrains, Apache-2.0). The
Catppuccin, Tokyo Night, Gruvbox, Nord, Everforest, Kanagawa, Rosé Pine Dawn and
Flexoki Light adapt [Omarchy's stock palettes](https://github.com/omacom/omarchy/tree/quattro/themes).
The other additions use the upstream [Dracula ANSI palette](https://draculatheme.com/spec),
[One Dark](https://github.com/atom/one-dark-syntax/blob/master/styles/colors.less),
[Rosé Pine](https://github.com/rose-pine/alacritty/blob/main/dist/rose-pine.toml),
[Solarized](https://ethanschoonover.com/solarized/), and
[GitHub Primer](https://github.com/primer/github-vscode-theme/blob/main/src/colors.js).
Their MIT notices are included in [theme licenses](../LICENSE-THEMES).

Lenz derives its semantic diff and syntax colors from these base palettes rather
than reproducing each editor's token mapping. The new palettes adjust muted colors
and a few light-theme syntax accents to keep at least 3:1 contrast on the editor
background; Solarized Light uses its darker `base01` for body text.
Selection, menus, diff highlights and Markdown use the same shared
palette in Reader, GUI Diff and TUI.

## Controls and Omarchy following

Choose **Change theme** from the command palette or the **☰** menu of either
frontend (in the TUI, F10 or `:`). It is available in comparison, repository,
merge, Conflict List and Version Picker views, including empty and loading views.
Use arrows, Home/End or Page keys and Enter to select; clicking a visible row
also selects. Esc dismisses the chooser.
It shows the current choice and effective source, wraps in narrow layouts and
keeps keyboard selection visible. Other document/toolbar input is suppressed
while it is open.

**Follow system** is the initial choice. On Linux it reads
`$XDG_STATE_HOME/omarchy/current/theme/colors.toml` first, falling back to
`$XDG_CONFIG_HOME/omarchy/current/theme/colors.toml` when the state file is absent.
Defaults are `~/.local/state` and `~/.config`. `current/theme.name` supplies the
source label. Lenz imports the actual palette, so additional/custom Omarchy
themes work without a matching bundled theme ID. Changes to files, symlinks,
directories and source names update an idle GUI/TUI without a keypress.

Missing, invalid or confirmed removed input falls back with a notice. The GUI
uses native light/dark appearance when Omarchy is unavailable; the TUI uses
Darcula and does not detect the terminal emulator's native theme. Filesystem
notification failure reports a notice and checks once per second. Temporary
incomplete replacements are retried before fallback.

A process shares one selection across its independent views. Separate GUI/TUI
processes read the same saved setting on startup; manual choices are not
broadcast between already-running processes. User themes are rediscovered when
the chooser opens and revalidated when selected.

## User theme format

Put a UTF-8 TOML file under `$XDG_CONFIG_HOME/lenz/themes/`, or
`~/.config/lenz/themes/`. Its ID is `user:<filename-stem>`. Files are limited to
64 KiB and the chooser considers at most 256 candidates in filename order.
Symlinks to regular files work; devices, FIFOs and directories do not.

```toml
name = "My theme"
mode = "dark" # or "light"
background = "#1e1e2e"
foreground = "#cdd6f4"
accent = "#89b4fa"
red = "#f38ba8"
green = "#a6e3a1"
blue = "#89b4fa"
yellow = "#f9e2af"
magenta = "#f5c2e7"
cyan = "#94e2d5"

# Optional Lenz semantic overrides:
[palette]
found_bg = "#45475a"
[syntax]
keyword = "#cba6f7"
```

Names are nonempty text up to 128 bytes, without control characters.
All colors are `#RRGGBB`. Optional base colors are `selection`, `muted`,
`dark_foreground`, `dark_background` and `lighter_background`.

`[palette]` accepts scalar keys `bg`, `fg`, `dim`, `gutter_bg`, `num_fg`,
`header_bg`, `sidebar_bg`, `status_bg`, `border`, `selected_bg`, `fold_bg`,
`sel_bg`, `caret`, `found_bg`, `banner_bg`, `accent`, `error`, and `blame_new`.
`block`, `inline` and `edge` accept four colors ordered inserted, deleted,
modified, conflict. `status` accepts five colors ordered added, deleted,
modified, renamed, conflicted.

`[syntax]` accepts `keyword`, `string`, `number`, `comment`, `doc_comment`,
`function`, `constant`, `macro_`, `type_`, `builtin_var`, `tag`, `attribute`,
`label` and `link`. Unknown override keys and invalid values are rejected.

## Selection and persistence

Follow system is the default. A manual theme stays selected across restarts and
ignores later Omarchy changes; choose Follow system again to resume. The choice is stored in
`$XDG_CONFIG_HOME/lenz/appearance.json` (home fallback as above). Selection takes
effect even when saving fails, with a notice that it was not saved. Missing
custom themes fall back while retaining their saved ID.

`LENZ_THEME=system|<id>` overrides saved settings for that launch.
`LENZ_APPEARANCE=light|dark` is read when LENZ_THEME is unset. Both products also
accept the old Reader variables with lower priority: `LENZ_THEME`, then
`LENZ_APPEARANCE`, then `LENZ_READER_THEME`, then `LENZ_READER_APPEARANCE`.
Unknown overrides report a notice and use the saved/default selection. Explicit
menu selection supersedes the environment override and saves normally.

The theme format, built-in themes, selection and Omarchy following live in the
shared `lenz-content` crate. Reader and Diff use the same `lenz/appearance.json`
and `lenz/themes`. If the canonical selection file is absent, both read
`lenz-reader/appearance.json`; a missing canonical user-theme file can fall back to
`lenz-reader/themes/<name>.toml`. Existing canonical files always win, including
invalid files, which produce a notice. Legacy files are never rewritten or
migrated; explicit choices save to `lenz/appearance.json` and preserve its unknown
JSON fields. Separate processes read the shared selection at their next launch;
they do not synchronize menu choices live. Follow system still watches Omarchy
changes live.

Both GUIs also read `[gui] font_family`, `font_size` (8–32, default 13) and
`reading_font` from `lenz/settings.toml` at startup. Font changes take effect on
the next launch. Reader's other settings, recent roots, providers and document
state remain independent from Diff.


## Verification (2026-10-18)

All ten additions passed production-window rendering checks in Reader, GUI Diff
and TUI, including selecting the final theme in a 400×300 chooser. Screenshots
were inspected on an owned Xvfb display using Mesa lavapipe; these are software
rendering checks. The full local CI gate passed, including shared contrast,
Reader chooser/persistence, GUI/TUI input and packaging tests.

Alternating release-binary comparisons against `3cca626` used the same generated
2,001-line Rust documents, isolated settings and two warmup pairs. Medians:

| Endpoint | Measured pairs | Before | After | RSS before / after at 1 s |
|---|---:|---:|---:|---:|
| Diff first-paint marker | 8 | 130.5 ms | 129.5 ms | 211.46 / 210.78 MiB |
| Reader mapped window observed | 12 | 103.91 ms | 101.45 ms | 173.24 / 173.56 MiB |
| TUI first content bytes | 16 | 5.42 ms | 5.29 ms | 14.59 / 14.71 MiB |

Reader window observation is not first paint or document readiness. TUI bytes
are not emulator display latency or completed refinement. Initial TUI timing
outliers did not reproduce in the 16-pair repeat. Reader's initial RSS difference
of 0.91 MiB narrowed to 0.32 MiB on repeat; executable mappings accounted for
0.46 MiB more residency, while anonymous mappings decreased by 0.09 MiB. This
is not evidence of a heap leak or zero memory overhead. The stripped Diff GUI
executable grew by 6,272 bytes.

Raw samples, executable/source identities and screenshots are retained locally
under `target/theme-evidence/`. Cold starts, hardware rendering of these palettes,
long-duration stress and native macOS were not rerun for this palette-only change.
