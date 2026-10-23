# Lenz Reader (working name)

A fast code reader (it never edits files; only the Git checkouts and branches you choose change your repository) that shares decoding, syntax highlighting, themes and single-pane text
presentation with Lenz through local workspace crates `lenz-content` and `lenz-gpui`.

This is the imported Reader documentation. The proposed product name is **Lenz Code**;
[the command-family trial](../launcher-trial.md) explains how to launch it as `lenz [file-or-folder]`.

- `crates/reader-core` (`reader-core`): documents, commands and the session. No GPUI.
- `crates/reader-gui` (`reader-gui`, binary `lenz-reader`): the window.
- `crates/reader-tui` (`reader-tui`, binary `lenz-reader-tui`): the [terminal reader](tui.md).

```sh
cargo run --release -p reader-gui -- path/to/file.rs   # or a folder; with no argument, the folder it starts in (not ~ or /)
cargo test -p reader-core
cargo test -p reader-gui --features test-support     # real input on GPUI's test platform, no display
cargo test --release -p reader-core -- --ignored --nocapture open_perf
cargo test --release -p reader-core --test semantics -- --ignored --nocapture   # real rust-analyzer, if installed
```

Keys: Ctrl+O open, F5/Ctrl+R reload, Ctrl+F4 close, Ctrl+A select all, Ctrl+C copy, Shift+arrows/Home/End
select, Ctrl+W / Ctrl+Shift+W extend / shrink the selection, Ctrl+- / Ctrl+= collapse / expand, Ctrl+Shift+- /
Ctrl+Shift+= collapse / expand all, arrows move the cursor, PageUp/PageDown/Home/End scroll, Ctrl+P find file (`name:line`), Ctrl+G go to
line, Ctrl+F find, F3 / Shift+F3 next / previous match, Ctrl+Shift+F search, Alt+Z soft wrap, Ctrl+Shift+. breadcrumbs,
Ctrl+Shift+O outline, Ctrl+T declarations, F12 / Ctrl-click definition, Alt+F12 peek, Ctrl+Shift+B type definition, Ctrl+Alt+B
implementations, Shift+F12 usages, Ctrl+Alt+H call hierarchy, Ctrl+H type hierarchy, Ctrl+Q documentation, Alt+Left/Right (or the mouse's side buttons) back/forward, F11 toggle bookmark, Ctrl+F11 bookmark key, Shift+F11 bookmarks,
Ctrl+Shift+0–9 / Alt+0–9 set / go to a numbered bookmark, Ctrl+E recent files, Ctrl+Alt+E recent locations, Ctrl+Shift+V Markdown source/preview, F10 command menu,
Ctrl+Shift+P find action; Escape closes a menu or cancels a pending definition.

The **☰** button (GUI and terminal) opens File, View, Navigate, Find, Selection and Language Servers menus. They expose the
existing commands with setting values and disabled reasons; scroll long menus with the wheel or arrow keys. Click **‹ All menus**, or press Left/Escape, to go
back to the groups. **All commands…** and the bottom-right **Commands** button open searchable Actions.
F10 retains the plain full command menu. Narrow windows shorten the button label; below 240 px it gives way to the LSP status, with ☰ and F10
still available. Menus consume input before the document beneath them.

**Search Everywhere** follows [IntelliJ's name-navigation pattern](https://www.jetbrains.com/help/idea/searching-everywhere.html):
one popup with **All / Files / Symbols / Actions** tabs, a query, fuzzy matches and secondary paths/details.
Double-tap bare **Shift** in the GUI, or press **Ctrl+K**, to search all three categories. Direct shortcuts are:

| Search | Shortcut |
|---|---|
| Files by name or path | Ctrl+P or Ctrl+Shift+N |
| Symbols by name | Ctrl+Alt+Shift+N |
| Actions by name | Ctrl+Shift+A or Ctrl+Shift+P |

Tab / Shift+Tab cycles categories while keeping the query. Arrows, Page keys and Home/End select a
result; the wheel scrolls the popup without changing selection. Enter opens/runs the selected result and Esc returns to reading. Tabs and rows are clickable. File results distinguish
identical basenames by path and accept `name:line[:column]`. Empty queries prefer open/recent files; search
keeps at most the best 100 results. Action rows show shortcuts, setting values and current disabled reasons.
Long Unicode queries keep the typed tail visible. Search opens candidates only when selected.

Symbols come from the existing bounded project declaration index, including types, methods, functions and
constants supported by its syntax grammars. They work without a language server; indexing, syntax errors and
coverage limits remain visible. Open a folder to index project symbols. Language-server workspace symbols
remain separately available as **Find Symbol (language server)…**, including that provider's own coverage.

F2 and the Language Servers command group offer **Reuse Saved Declaration Index** (On by default),
**Exclude Ruby Tooling Gems for This Project** (On unless overridden), and **Rebuild Project Index**.
Both optimizations can be turned off. Rebuild clears saved local declarations, refreshes open documents and
restarts the project's language servers without changing source files or settings. See
[index settings and rebuilding](providers.md#saved-declarations-and-rebuilding).

Ctrl+Shift+N now finds files; **New Window** remains in the app/command menus. These workflows are also in the
[terminal reader](tui.md), with a portable shortcut when bare Shift cannot be reported.

Following a definition into a dependency or standard library reveals it under **Libraries** in the explorer,
including dependencies excluded by the project's ignore rules. Expand its folders and open sibling files as
usual. This also works when reading a single file without an open project. Libraries are discovered as you
visit them; only expanded folders are listed, in background jobs. Adding a library does not add files to the
local project declaration index. The browsing root stays unchanged.

Collapse and expand a library folder to refresh its listing after a dependency update or branch checkout.
Listings are limited to 2,000 entries per directory and 20,000 overall, with a status message when truncated;
the definition you followed remains visible. At the overall limit, following another definition releases old
listings. Up to 128 library roots and 128 retrieved sources are kept for the current project session. Changing
projects clears them. Unrecognized dependency layouts use the nearest package manifest or containing folder.
Filenames that cannot be represented as Unicode are omitted with a partial-listing message.
Sources retrieved from a server, such as Java class sources, appear under that server's name; only visited
sources are listed, since the protocol does not provide a directory listing. Reopening one asks its original
server session for the text; after that session ends, follow the definition again.

Holding Ctrl over a name underlines it and shows a hand pointer: a click there follows it. A right-click in the text
opens a menu with every navigation action, Copy Path / Copy Path:Line, Open in Editor, Open with Default App,
Compare With… and Split Right / Split Down (Close Split once split). The call hierarchy lists callers (Tab:
callees) in the results panel, a level loading as it unfolds (Right/Left). Type Hierarchy (Ctrl+H) lists the subtypes of the type at
the caret the same way (Tab: supertypes). It comes from the server's own type hierarchy where it has one (JDT LS,
ruby-lsp), else from the server's usages read against the syntax of the files they are in: Rust `impl … for` and
supertraits, Python bases, TypeScript/JavaScript/Java `extends` and `implements`, Kotlin `:`, Ruby `<` and `include`.
So a Rust struct's supertypes are the traits it has an impl for; names are matched by their last segment, and types
made by macros or reached through renamed imports are not followed; a Rust trait is labelled "trait". A level built
from usages reads at most 200 of them and says so when it stops there, or when the server was still indexing. On a
plain Java source tree (no Maven, Gradle or Eclipse project) JDT LS is told the file that started it and imports the
folder as a project of its own, kept outside the folder; nothing is written into it. A level takes about as
long as the server's usages plus reading its files (a warm rust-analyzer: 0.1 s for `Default` across this
repository; its first query of a session takes seconds); launch and memory are unchanged. Resting the pointer on a name shows its
documentation in a popup (Documentation on Hover in the command menu turns that off), and the other occurrences of the
name at the caret are tinted. Definitions come from installed language servers (see [provider compatibility](providers.md)); explicit
provider settings live in `$XDG_CONFIG_HOME/lenz-reader/providers.json`. The toolbar keeps navigation and frequent file/search actions above the tabs, with a visible app menu.
The footer separates language-server status, the current message, cursor position and Commands.
Long messages end with an ellipsis; click the message or use **Status details…** (Shift+F2)
for a wrapped, scrollable snapshot. Ctrl+C copies the complete snapshot. Routine server-memory
samples stay in details so they do not continually change the footer.

Click the LSP footer status or press F2
for Restart/Enable actions and **Server Details…**, a scrollable diagnostic popup with Copy and Close buttons. Java and Pyright also offer in-app runtime selection. Ruby projects offer an opt-in **Exclude Ruby Tooling Gems for This Project** action; **Reset LSP Settings for This Project** restores inherited settings. See [project settings](providers.md#project-language-server-settings) for scope and navigation limits, and [status and recovery](providers.md#status-and-recovery).

The language servers also add inlay hints and semantic colours to the text panes (sticky lines included). Type and
parameter-name hints are drawn inline and dim (cut at 30 characters); semantic tokens recolour names over the syntax
colours, underline mutable ones and strike deprecated ones through. Hints are display only: the caret steps over
them, a click on one lands at its place, and selection, copy, find and links use the source text. Search snippets,
the peek list, large files, images, Markdown previews and sources a server returned get none. The command menu's
"Inlay Hints: Types", "Inlay Hints: Parameter Names" and "Semantic Highlighting" (on by default, remembered in
`settings.json`) turn them off; turning one on says when the file's server offers none. A server is asked per file
version, again when it asks for a refresh, and once more when it finishes loading (rust-analyzer answers empty while
it loads). Measured on Linux (Xvfb, 2026-10-15, against `main` 1cf0893): launch unchanged (145 ms median to
readable); with all three off, scrolling CPU and memory as before; with them on, 10 s of scrolling costs about 5% more
reader CPU (+0.1 ms per frame) and the reader holds +5 MiB with `session.rs` and +19 MiB with a 10,000-line file
(rust-analyzer sends about 6.5 MB of hints for it, decoded straight from the bytes); 100 rounds of toggling and tab
switching settle at the same memory.

Code folds like IntelliJ's: a gutter mark (▾ open, ▸ collapsed) on every block, comment run and import run spanning
lines; a collapsed one shows a boxed `…` and its closing brace, and a click on either opens it. Each pane folds on its
own; a jump into a collapsed block opens it, and a reload keeps the folds that still match. Folds come from the same
background parse as the syntax colours, so large files and files without a grammar have none. Ctrl+W grows the
selection through the syntax around it (word, string or block inside, expression, statement, block, … whole file;
without a grammar word, line, paragraph, file) and Ctrl+Shift+W walks back. A double click selects the word (or
the run of spaces or punctuation) under the pointer, a triple click Ctrl+W's next step from that word, and a fourth
starts over; Ctrl+W then goes on from there. Copy with no selection copies the caret's
line; a selection over collapsed code copies the hidden source too; on Linux every selection is also the primary
selection (middle-click paste). In a Markdown preview, drag, double-click, Shift-click and Ctrl+A select the rendered
text and Ctrl+C copies it.

Choose **Change theme** directly from the ☰ app menu (or Find Action). The chooser works like Diff: arrows and Page/Home/End preview a theme, Enter or a click selects it, and Escape or an outside click restores the current theme. Scrolling moves the list without changing the highlighted theme.

**About Lenz Code**, below Change theme in the ☰ app menu (or Find Action), shows the version, platform, licence and project page shared with Lenz Diff's About, with the Lenz logo when the dialog has room (a half-block logo in the terminal). Copy copies the text; Escape, Enter or an outside click closes it. The logo is decoded only while About is open.

**File Icons** in the View menu shows or hides file-type icons beside file names in the explorer, tabs and results; the choice is remembered and shared with the terminal reader, where the icons are Nerd Font glyphs.

The reader never writes the files it shows (a Git checkout you choose changes them as `git switch` would). It shares Diff's theme choice in
`$XDG_CONFIG_HOME/lenz/appearance.json`, user themes in `lenz/themes`, and font preferences
(`[gui] font_family`, `font_size`, `reading_font`) in `lenz/settings.toml`.
`LENZ_THEME` overrides `LENZ_APPEARANCE`, followed by the compatibility variables
`LENZ_READER_THEME` and `LENZ_READER_APPEARANCE`. Old Reader appearance files and user themes
remain read-only fallbacks when the canonical files are absent; invalid canonical files produce a notice.
Saved appearance and font changes reach other processes on their next launch; Omarchy following remains live.
See [Themes](../themes.md) for the full precedence and format. Reader's other settings remain independent.
Files over 5 MiB or 100,000 lines open in large-file mode: read in full, without syntax highlighting.

The explorer is its own panel left of the files panel (the tabs sit over the text). The toolbar's first button, or
Ctrl+B, shows and hides it; Ctrl+Shift+E focuses it. Its target button selects the opened file, opening the folders
above it. Opening a folder whose only entry is a folder opens that one too, down to a folder with more. Folders list
before files; the command menu's "Explorer: All by Name" switches to plain name order ("Explorer: Folders First" back) (bytewise:
capitals first), saved in `$XDG_CONFIG_HOME/lenz-reader/settings.json`.

Ctrl+\ splits the file panel side by side, Ctrl+Shift+\ one above the other (also "Split Right", "Split Down"
and "Close Split" in the command menu's View group and the text's right-click menu). Each pane has its own tabs and shows its own place, also in the same file;
Ctrl+1 / Ctrl+2, a click or the wheel focus a pane, and its shown tab is marked. Closing a pane's last tab, or dragging
it into the other pane, closes the split. Tabs drag within a bar to reorder or
into the other pane. The explorer's edge, the results panel's top and the split's divider drag; the sizes and the
split are remembered in `settings.json` (a remembered split starts with an empty second pane).

**Open in Editor** (File menu, right-click) opens the active file in your editor at the caret, as Lenz Diff's `o`
does: `LENZ_EDITOR`, else `$VISUAL`, else `$EDITOR`, else `[tools] editor` in `$XDG_CONFIG_HOME/lenz/settings.toml`
(for a Lenz Code started from the Dock or a desktop menu, which rarely has the shell's variables). A terminal editor
opens in a new terminal window. With no editor, and for images, the file goes to the system's default app with a
message; **Open with Default App** always does. **Compare With…** compares the active file, or the explorer's
highlighted file or folder when the explorer has focus, with a file or folder you pick (the system dialog in the GUI,
a typed path in the terminal), in Lenz Diff: its window from the GUI; from the terminal, Lenz Diff's terminal app
takes over the terminal until you quit it. Lenz Diff must sit beside Lenz Code, as release packages install it.
**Compare Selected** (Ctrl+D, Cmd+D on macOS) compares two explorer entries, as in IntelliJ: Ctrl+click (Cmd+click)
the second one, or press Space on each; marked entries show a ✓. Escape in the explorer, or a plain click, forgets
them.

**Git Branches…** (Git menu, Ctrl+Shift+\`, or a click on the branch in the status bar) lists **New Branch…**,
**Checkout Tag or Revision…**, then the **Recent**, **Local**, **Remote** and **Tags** branches and, with more than one,
the **Worktrees**; typing filters. Enter checks out the highlighted branch (a remote one as a local branch tracking it,
a tag or revision on a detached HEAD) or opens the highlighted worktree as the folder; → or a right-click lists a row's
other actions (New Branch from Here…, Open in New Window). Your uncommitted changes go along when git allows. When they
would be overwritten, a dialog lists the files: **Smart Checkout** stashes them, checks out and puts them back (a
conflict keeps the stash, and **Resolve Conflicts in Lenz Diff** opens Lenz Diff on it); **Force Checkout** first lists
every uncommitted change it discards, then discards them on a second confirmation; untracked files in the way must be
moved or deleted first. After a checkout the open files reload (removed ones show as unavailable), the explorer and
index refresh and annotations blame again. The status bar shows the branch (or the detached commit) and follows a
switch made elsewhere within a few seconds. Checkouts run git as your terminal would, with the repository's hooks and
filters; everything else Lenz Code asks git keeps them off.

The Git menu also compares the working tree as it is (uncommitted and untracked files included) in Lenz Diff:
**Show Local Changes** against the last commit (what the next commit would hold); **Compare with Branch, Tag or
Revision…** against one picked from a list of branches and tags, or any revision typed; **Compare with Base Branch**
against the base branch as it is now (the remote's default branch, else `main`, `master`, `origin/main` or
`origin/master`); **Show Changes Since Branch Start** against the point the current branch split from that base, so
only what the branch changes, not the base's later updates; **Compare with Worktree…** lists the other worktrees and compares the two folders. A row's actions in Git
Branches… add **Compare with Working Tree** (branches, tags and worktrees) and **Show Changes Since Branching from
Here** (other branches). Folder comparisons inside a repository skip what git ignores (`target/`, `node_modules/`); submodules and nested
repositories are compared by their own ignore rules.

**Annotate with Git Blame** (View menu, right-click, Alt+B) shows, left of the line numbers, the date and author of
the commit that last changed each line, tinted by age (newest strongest); lines not committed yet say so. It is per
tab, streams in as git reports it, and runs again when the file changes on disk; after a commit, amend or checkout
that leaves the file's bytes as they are, press F5. Hovering over an annotation (GUI) shows its commit and highlights
that commit's other lines; in the terminal the status line shows the caret line's commit. A click on an annotation, or
**Show Commit…**, opens the commit (message, author, date, changed files) with **Show Diff**, which opens the whole
commit in Lenz Diff against its first parent, and **Copy Revision**; both are also in the right-click menu. Annotating
an earlier revision, choosing what the column shows and a commit log are not available.

Find (Ctrl+F) opens a box over the pane's text: matches are tinted as you type and the counter says which one is
selected (at most 10,000 are listed); Enter / Shift+Enter or F3 / Shift+F3 step, Alt+Enter lists every match in the
results panel, Escape closes. Alt+C, Alt+W and Alt+R (or the box's Aa, W and .* toggles) match case, whole words or a
regular expression; the options are shared with project search while the reader runs. A match never spans lines. Go to
Line (Ctrl+G) takes `line`, `line:column` or `line,column`, and the file finder opens `name:line[:column]` there.
Soft Wrap (Alt+Z, remembered) wraps long lines at the pane's width; continuation rows are indented like their line and
carry no number. Large files, images and previews do not wrap.

Sticky Scroll pins the first lines of the declarations around the top line over each pane's text (at most five, the
innermost kept; Rust `impl` blocks and Markdown sections count); a click goes there. A breadcrumb bar under each pane's
tabs shows the file's path in the project, then the declarations holding the caret: a folder or file shows in the
explorer, a declaration gets the caret, and the `›` after a crumb lists a folder's entries, the files beside a file,
or the declarations beside a declaration (type to filter, Enter goes there). Ctrl+Shift+. gives the bar the keys (Left /
Right, Enter, Escape). Both come from the syntax tree, for the languages the outline covers; both are on by default
and turn off in the command menu (Sticky Scroll, Breadcrumbs; remembered in `settings.json`).

Bookmarks work as in IntelliJ: F11 adds one on the caret's line (or removes the one there), Ctrl+F11 asks for a key
(0–9, A–Z; the line's own key removes it, a key used elsewhere moves here), Ctrl+Shift+digit sets a numbered one and
Alt+digit goes to it. On an AZERTY keyboard Alt takes the digit row's key without Shift (Alt+&, Alt+é, …); there
Ctrl+Shift+digit cannot be told from Ctrl+digit (the toolkit reports no physical key), and Ctrl with a symbol of the
digit row sets the bookmark that symbol numbers on a US keyboard (Ctrl+& sets 7), so set a numbered bookmark with
Ctrl+F11, then the digit. A bookmarked line shows its key (or a dot) at the gutter's left edge, sticky lines included. Each
bookmark keeps its line's text and finds it again when the file changes (the nearest copy wins; when none is left it
keeps its line number and lists as "moved?"). Shift+F11 lists them; Next / Previous Bookmark are in the command menu.
Recent Files (Ctrl+E) lists files by when they were left, the file before this one first, so Ctrl+E, Enter switches
back. It shares Find Files' icons, secondary paths and fuzzy name/path matching, with a dedicated Recent Files
heading. Recent Locations (Ctrl+Alt+E) lists where navigation went from and to, and where a file was left (a place
within five lines of a listed one replaces it). In each list, typing filters, Enter or a click goes there (a history step), Delete
removes the entry, and files that no longer exist show "missing". The reader keeps 50 recent files and 30 recent
locations per folder and every bookmark, in `$XDG_STATE_HOME/lenz-reader/projects/` (one file per folder, at most 4 MiB,
saved a second after the last change and at exit); without a folder they last until the reader closes. Two windows
on one folder merge their places when either saves; a window shows the other's changes after its own next save. A
places file from another reader version is left as it is, and places are then not saved. An unreadable file is kept
beside it as `.json.unreadable`.


Reopening a folder restores its last reading workspace in both GUI and TUI: up to 200 file tabs in order,
the active file and focused split pane, caret/selection and scroll positions, source/preview choices, soft wrap,
panel sizes, expanded folders, and explorer visibility/selection/scroll. Switching folders saves the old workspace
and replaces its tabs with the destination's workspace. An explicit file opened before the first folder takes
precedence over saved tabs. Missing files are skipped, positions are clamped to current UTF-8 content, and only
the shown files load immediately; other tabs load when activated. Search queries/results and language-server
virtual sources are not restored.

Workspace metadata shares the folder's places file above. It is checkpointed when switching folders, closing a
window/view or quitting; forced termination can lose changes made since the last checkpoint. The last window
to save determines the workspace for the next opening; existing windows keep their own live state. Bookmarks
continue to merge independently. Workspace metadata exceeding the places-file size limit is omitted so it
cannot make bookmarks unreadable.

Project search (Ctrl+Shift+F) opens a centered dialog that takes the same options (literal and case-insensitive by
default); a click on a field focuses it and a click on an option toggles it, and an invalid regular expression is
shown there and not searched. Tab moves to its files field:
comma-separated
globs, any case, `!` to exclude (`*.rs, src/, !*.test.ts`); a name without `/` matches at any depth, a path with `/`
from the root, and a folder means everything in it. Matches list as a tree of folders, files and lines, coloured by
the file's language with the match emphasized; a click or Left/Right folds a folder or file. A line's colours come
from that line alone, so one inside a multi-line comment or string may read differently than in the file.

Markdown files open as their source. **Preview / Source** sits at the right edge of each file pane's header
and controls that pane's document, including in split views. Ctrl+Shift+V and the command menu remain available
when the pane is too narrow for the button. Switching preserves the reading line, and a file switched to its
preview opens that way again. The preview shows local raster images only: inside the
browsing root, or beside the Markdown file when it is outside the root, resolved through symlinks, at most 64 per
document and 64 MiB each. Remote, missing and outside images say why in place. Links are not followed and folded
callouts are shown open.

The Reader uses the local `lenz-content` and `lenz-gpui` crates from this workspace.

The [terminal reader](tui.md) presents the same core session and commands, with terminal-specific controls and display limits.
