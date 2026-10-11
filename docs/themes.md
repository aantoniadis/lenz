# Themes

Lenz shares semantic chrome, syntax and diff colors between its GUI and TUI.
Bundled themes are Darcula (`darcula`), Light (`light`), Catppuccin Mocha
(`catppuccin`), Catppuccin Latte (`catppuccin-latte`), Tokyo Night (`tokyo-night`)
and Gruvbox (`gruvbox`).

## Attribution

Darcula and Light follow IntelliJ's color schemes (JetBrains, Apache-2.0). The
other base colors are adapted from [Omarchy's stock palettes](https://github.com/omacom/omarchy/tree/master/themes),
under [Omarchy's MIT license](https://github.com/omacom/omarchy/blob/master/LICENSE).
Semantic diff and syntax colors are derived for Lenz.

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
`LENZ_APPEARANCE=light|dark` is a legacy override when LENZ_THEME is unset.
Unknown overrides report a notice and use the saved/default selection. Explicit
menu selection supersedes the environment override and saves normally.
