# Terminal editor

The [integrated terminal panel](terminal.md) supports multiple shell sessions inside
the terminal editor too: Ctrl+Alt+T shows it, Ctrl+Alt+N creates another tab and
Ctrl+Alt+0 returns to the source editor. Ctrl+F searches the focused terminal’s
scrollback; Enter / Shift+Enter step matches and Escape returns to the shell.
Native file dragging depends on the host
emulator; use **Paste File Path to Terminal…** for the equivalent Lenz action.

![Terminal editor with Nerd Font icons, explorer and focused reading tab](../images/code-tui.png)

Lenz Code's terminal editor presents the same `code-core` session as the GUI. Installed, run
`lenz --tui path/to/file.rs` or `lenz-code-tui` (see [the command family](../launcher-trial.md)).
Build the adapter separately so its executable
does not include GPUI, GPU or windowing dependencies:

```sh
cargo build --locked --release -p code-tui
./target/release/lenz-code-tui path/to/file.rs
./target/release/lenz-code-tui path/to/folder
```

With no argument it opens the starting folder, except home and `/`, as the GUI does. `--help` and `--version`
work without a terminal; ordinary reading needs an interactive terminal on stdin and stdout. `--` permits a
path beginning with a dash. Opening a file alone does not enumerate its parent; Open Folder starts project
browsing and project search.

A window with nothing open starts on **Open File…**, **Open Folder…**, the recent folders and the files recently
opened without a folder (each newest first, at most ten, kept in `recent-roots.json` and `recent-files.json` in the
state folder), in the terminal as in the GUI. A click runs the command or opens the folder or file; closing the last
tab returns to it. From the keyboard, Ctrl+O opens a file and the command menu (F10) has Open Folder… and Open Recent
Folder…. Rows a small terminal cannot show are clipped and take no clicks.

Reopening a directory restores its saved tabs, active file, caret and scroll positions, split panes and explorer
state through the same core as the GUI. Switching directories saves and replaces the workspace; closing a view
or quitting saves it too. Files load lazily and missing files are skipped. See [workspace persistence](README.md)
for limits and concurrent-window behavior.

Dependency definitions reveal the same lazy **Libraries** section as the GUI, including when no project folder
is open. A single click selects a folder; click its arrow or double-click its name to expand or collapse it.
Ctrl+Shift+E focuses the explorer; arrows expand or collapse folders and Enter opens a file. A click on a file
opens it and leaves the keys in the explorer, as in the desktop. Drag the explorer's last column to resize it; the
width is remembered and the text keeps at least 24 columns. With two panes, drag the divider (`│`, the first pane's
last column), or the lower pane's tab bar past its tabs, to share the space differently; the share
is the desktop's, remembered, and each pane keeps 4 rows or 12 columns (the first pane's last one is the divider). Drag the results panel's heading, past its
buttons, to change its height; that share is the desktop's too, and the text keeps 3 rows. Collapse
and expand to refresh a listing. Type to search visible file/folder names: Up/Down cycle highlighted matches,
Backspace edits, Enter activates and clears the query, Escape restores the earlier selection. Space still marks
a file when no query is active. Server-retrieved sources list visited files only. See the
[Lenz Code guide](README.md) for listing limits and server-session behavior.

Explorer rows and tabs show the file's Git status against HEAD as a letter: **M** modified (staged or not),
**A** added, **D** deleted from disk, **R** renamed, **U** untracked (a file inside an untracked folder too) and
**C** conflicted; the color is extra. `●` marks unsaved changes separately, so a draft over a Git-clean file shows
`●` and no letter, and Save leaves the letter to Git. Both adapters read the status in the background, once per
change: after the repository is found, on file changes, and when the index or HEAD changes (checked on the
five-second tick). Refreshing never touches drafts or history. At most 20,000 changed paths are read; folders show
only their own untracked status.

The gutter marks the draft's changes too, left of the line numbers: **M** in the modified-file blue for a line the
draft modifies and **A** in the added-file green for a line it adds, each identified by its letter rather than its
colour (`NO_COLOR` loses only the colours); a second cell shows **D** where the draft deleted lines just above the
line. The baseline is named in the status bar (`vs HEAD`, `vs new`, `vs saved`) and in **Status details…** (Shift+F2,
`vs HEAD (main · 1a2b3c3): 2 added, 1 modified`), and the caret's line names its change there
(`Modified from HEAD (main · 1a2b3c3)`) unless blame annotates that line. Tracked files are compared with HEAD's version — unsaved edits included, so
Save leaves them — new and untracked files are compared with nothing and say so (`new file (untracked)`, or
`new file (added to Git)`), and files outside Git with the last confirmed save. Blame keeps its column further left,
bookmarks their cell, and folds and line numbers keep theirs; pinned (sticky) lines carry their letters too. The comparison is made
once the draft rests (about a third of a second after the last keystroke) for the files on screen only, so typing
starts no work of its own: the last result stays drawn until the newer one is ready.

**Compare saved** (Ctrl+Shift+D, also in the palette) opens an in-window comparison against the last
confirmed saved version. **Last saved (read-only)** and **Current draft** show changed rows with a blue
background and `▌` gutter marks. The current side shares the ordinary tab’s buffer and undo history;
the saved side cannot be edited. **Close comparison**, Ctrl+F4 or Close Split restores the original views
without discarding the draft. Rejected or uncertain Save keeps the prior baseline. The panes scroll
independently. Platform input availability is described below; see the [implementation and verification limits](../research/code-saved-comparison.md).

**Resolve External Change…** (File menu; enabled while a draft is dirty and the file is known to have changed
on disk — by the watcher, a rejected Save or a recheck that found another version; Ctrl+S opens it once the watcher
has reported the change, and the unsaved-changes prompt then offers no Save) reads the file again and shows
its three sides: **Disk** (the file as read now, with its lines and bytes and how it relates to the baseline),
**Current draft**, and **Confirmed baseline (last confirmed save)**. It decides nothing. It offers **Compare Disk,
Draft and Baseline…** (`c`), **Take Disk Version** (`d`), **Save Draft over Disk** (`s`) and **Keep Draft** (Esc);
keys `c`, `d`, `s` and Esc answer this dialog, so they do not follow the keymap preset. Every choice except Keep
Draft reads the file once more, off the UI thread, and decides only if it still holds the same identity and content
the dialog showed; another external change or an unreadable file retains the draft, the history and the bytes and
refreshes the sides. **Take Disk Version** replaces the draft with one undoable edit and confirms that version, so
the tab becomes clean and `⟲` is released; the file is not written. **Save Draft over Disk** adopts nothing until
the write is confirmed: it writes the draft after that one check, and a rejected or uncertain outcome keeps the
draft, its history and the confirmed baseline. **Keep Draft** keeps the text and the baseline it was edited from —
the new disk version never becomes a baseline implicitly — and leaves the conflict recorded. **Compare Disk, Draft
and Baseline…** opens the comparison with the disk version in place of the last saved one and the confirmed
baseline as its own read-only side beside it; the draft side shares the ordinary buffer and history. Closing that
comparison returns the dialog for the draft as it stands, so a resolved draft can then be saved over the disk.
The GUI shows the same dialog with the same labels and greys a choice with the core's reason; the comparison's
three sides carry the same titles there. A very narrow pane can show only some comparison tab titles; the tab bar
scrolls rather than changing the labels.

Opening a dirty file in an editor or default app offers **Save and Open** (`s`),
**Open Saved Version** (`o`) or **Cancel** (Esc). The saved-version choice keeps
the draft/history and omits dirty caret coordinates. Save and Open launches once
after confirmed Save; Cancel, stale decisions and failed/uncertain Save launch
nothing. An already requested Save can still finish after Cancel. External
comparison actions explicitly use saved disk inputs. See the
[handoff behavior and verification](../research/code-external-open-decisions.md).

**New Scratch File…** (F10 → File) creates a named personal file; include an extension such as `.rs` or `.md`
to choose its language. **Scratch Files…** lists these files to reopen, with typing to filter and Enter to open.
They live in `$XDG_STATE_HOME/lenz-code/scratch-files` (by default `~/.local/state/lenz-code/scratch-files`),
independently of the project, and remain available after switching projects or restarting. Save writes edits and
ordinary draft recovery protects unsaved changes; closing a tab does not delete the file. Scratches have no run
or debug controls. Both adapters ask for the name inside Lenz and offer a scratch-file list.

**New File** (Ctrl+N) asks for a name inside Lenz and creates an empty file in the highlighted explorer folder
when a project is open, using the same rules as **New File in Explorer…**. Without a project, it opens an
untitled document with its own history; the first Save or **Save As…** (Ctrl+Shift+S) asks for a path inside Lenz.
Save As
refuses missing folders, folders, broken links and files open in another tab, and an existing file needs
**Replace** (`r`) or **Cancel** (Esc) in a Lenz dialog; Replace writes only if that same file is still there.
A new file is created without replacing anything that appears meanwhile. The draft keeps its encoding, BOM
and exact line endings (UTF-8 without BOM when untitled). Only a confirmed write moves every view to the new
path; a rejected or uncertain write leaves the document, its original file and its Save state unchanged.
The original file and its bookmarks stay where they were. The GUI supports Ctrl+A to replace the suggested
path or scratch name and Ctrl+U to clear it; Escape cancels without writing.

**New File in Explorer…** and **New Folder in Explorer…** (right-click an explorer row, or File → New) create inside
the highlighted folder (a highlighted file creates beside it; nothing highlighted means the project root). The
terminal asks for the name; the GUI uses the same in-window prompt with the same words. The shared rules refuse an
empty name, `.`/`..`, a path separator, a control character, a name over 255 bytes, a name another entry takes, a
folder that has gone, a path outside the project, and a folder whose own mode allows no new entries — each with the
reason in the prompt, and nothing is created. Creating writes exactly one entry. A new file is an empty UTF-8 file
that opens as that file (its own buffer, draft and history, so Save/Undo/⟲ behave as for any file) and is selected
in the explorer when the watcher's refresh arrives; it is not an untitled document and leaves no recovered draft. A
new folder appears open in the explorer at once, and the next create inside it needs no rescan. Escape closes either
prompt and changes nothing.

The explorer’s right-click menu also offers **Copy File Name**, **Copy Full Path** and
**Copy Project-Relative Path**, all for the highlighted entry rather than the active editor. **Copy File / Folder**
and **Paste File / Folder…** (Ctrl+C / Ctrl+V while the explorer has focus) copy saved disk contents. Paste asks
for a name in the highlighted folder, or beside the highlighted file; Ctrl+A replaces the suggested name in the
GUI and clears it in the terminal. Existing entries are never overwritten. Unsaved drafts stay unchanged;
save first to include them in a copy. Copies run in the background, with **Cancel File Copy** available, and
publish only after the complete copy succeeds. Symbolic links, special files and more than 256 nested folders
are refused; ordinary file and folder permissions are preserved.

On Linux, Copy also puts a file URI on the desktop clipboard, for pasting into a file manager. It needs
`wl-copy` on Wayland (`wl-clipboard`) or `xclip` on X11; a missing helper is reported and tree Paste still works.
A terminal must share that local desktop environment for desktop copying; OSC 52 only carries text, so it cannot
publish files. Desktop file clipboard support on other platforms is not implemented. Tree Paste uses this window’s
last explorer Copy, rather than importing arbitrary files from another app’s clipboard.

**Show File / Folder Git History** opens Lenz Diff’s history for the highlighted entry: files follow renames,
folders constrain the history and comparisons to that folder. Clearing a typed history filter retains this scope.
Rename, Move, trash/delete and Restore remain in the same menu, with their existing confirmations and draft protection.

**Recovered unsaved work** is offered when a session starts. While a draft differs from its last confirmed saved
version, Lenz keeps a private copy of it and of that confirmed baseline under `$XDG_CACHE_HOME/lenz-code/recovery`
(directory mode 0700, files 0600), never in the project and never in logs. The record goes when Save confirms, or
when the draft is explicitly Discarded, Reloaded or replaced from disk; Discard also leaves a tombstone, so a copy
left behind by an interrupted write cannot be offered again. Nothing here writes the file it shows.

At start the dialog names the recovered file (or the New File document), when it was recorded, and whether the file
on disk still matches the draft's confirmed baseline. **Restore Draft** (`r`) reopens it as the ordinary draft, with
its own undo history, and marks the tab `⟲`; the file on disk is not changed. If the file still matches, Save works
as usual; if it changed, Save stays closed until **Recheck Saved File** compares the current file, and the retained
draft can still be written elsewhere with Save As, and **Compare with Last Saved Version** shows it against the
recovered confirmed baseline. A missing file restores the draft only. **Discard Draft** (`d`)
deletes the private copy. **Later** (Esc) keeps it for this session; **Recover Unsaved Draft…** in the File menu
offers it again. The GUI uses the same dialog and labels. `r`, `d` and Esc answer this dialog, so they do not follow
the keymap preset or an override; the File command is the route when the dialog is not open, and the presets give it
no chord (like Close Window, it stays in the menus and F10).

Records are one per document path, so two windows editing the same file keep only the last record each writes. A
draft and baseline together over 8 MiB (about a 4 MiB document) are not kept, and an older record is removed rather
than offered as older work. At most 20 records and 32 MiB of text are read at start, and each is compared with its
file there. Recovery copies live in the cache: the operating system may clear them, and they are not a substitute
for Save. Native macOS editing remains gated.

The checkpoint23 candidate enables basic editor input by default on Linux TUI and Linux X11/Wayland GUI.
The GUI uses its actual backend identity; native macOS stays gated. q/j/k/slash/Space type literally, bracketed paste is
one transaction, and Cut/Copy/Paste/Delete plus Undo/Redo use shared commands. **Read-only Mode** applies to
all views of the same buffer and cannot override intrinsic restrictions or enable writing. Terminal Paste
normally comes from the emulator (often Ctrl+Shift+V); the Paste command explains this route. See the
[basic-input report and limits](../research/code-text-input.md).

**Vim Editing** (Edit → Editing Preferences, or F10) enables the same optional modal editor as the GUI. Escape enters Normal,
`i` enters Insert, `v/V` select, `h/j/k/l` and word/line motions navigate, counts and `d/c/y` operate,
`p/P` paste the internal register, `u` / Ctrl+R undo/redo. `:w`, `:q`, `:wq`, `:x` and `:q!` use shared
save/close safeguards. **Relative Line Numbers** (View → Editor Appearance, or F10) is independent; the current line stays absolute
and continuation rows stay blank. Both choices persist. See the [supported Vim subset](README.md#vim-editing-and-relative-line-numbers).
JSON **Format Document** (Ctrl+Alt+L) works offline and preserves tokens; it participates in project format-on-save.

**Keymap…** (F10 → Application, in both adapters) lists every command with a chord, marks overrides as *your choice*
and names a binding an override displaced. Type to filter, Enter gives the highlighted command the next key
(Esc cancel, Delete restores the preset's chord), Esc closes. A chord another command already runs is refused with
that command named, so no action silently loses its key. **Keymap: IntelliJ (Default)** and **Keymap: VS Code**
persist with any overrides in Lenz Code's settings file, never in the project. The terminal keeps Ctrl-Q for closing
the view: a command rebound to it is warned about here and stays in F10. Flow control (XOFF) can also swallow Ctrl-S
in some terminals, so Save is in F10 too. The [Lenz Code guide](README.md#keymaps) lists both presets.

**Several selections** (Ctrl+Alt+Down/Up in both presets—IntelliJ has no chord of its own for this, so it takes
VS Code's; Alt+Shift+Down/Up duplicate lines since the structural edits below; also **Add Cursor Below** /
**Add Cursor Above** in the menus and the palette) put a caret on the next or previous shown line at the caret's
column and keep the earlier ones. These are ordinary keymap chords, so **Keymap…** can move them; terminals that
do not report modified arrow keys deliver them as other keys, which is why both commands also sit in the palette
(F10) and the Edit menu. **Select Next Occurrence** (Alt+J) first selects the word under the caret, then adds the next
literal occurrence outside the selections already made; overlapping and repeated matches are skipped, and a press
with none left says so and changes nothing. Typing, Enter, Tab, bracketed paste, Delete, Cut and Paste then change
every selection as one transaction against the displayed version, and each caret follows its own edit. One Undo
takes the whole typing group, exactly as with one caret; Undo of a paste or deletion restores the text and puts each
caret back at its own selection. Delete removes one character per caret (a CRLF pair as one); Cut and Copy take each
selection's text, one per line, so bare carets copy nothing while several cursors exist. **Collapse to Single Cursor**, Escape (a fixed editing key, like Delete, so not rebindable), a click or any caret
movement (arrows, Home, End, clicking, Select All, a jump) returns to one selection, so navigation never acts on a stale set. The selections
belong to the view: a split pane of the same buffer keeps its own, and closing or restoring a workspace keeps only
the primary one. Read-only mode, an image, a preview, a retrieved or binary source disable all four commands with
the same reasons as typing, and an open dialog takes the keys first. Several selections that an edit makes identical
or overlapping are merged into their union; a caret inside a selection is dropped. Only the primary selection is
composed through an input method, and folding moves a hidden caret to its region's first line.

**Indentation, comments, moving and duplicating** work on every selection of the focused view and are one Undo step
each (**Edit → Lines and Indentation** menu, F10 and the palette in both adapters; the GUI has the same keys):

- **Tab** indents every line a selection touches by one step, keeps the selection on its text, and inserts that step
  at a caret; **Shift+Tab** removes one step from the touched lines (a tab, or up to the indent size in spaces) and
  never takes text back. The step comes from the file's EditorConfig, then the project's choice, then the language
  default, resolved when the key is pressed, so a changed `.editorconfig` applies at once. While a snippet is live
  (below), Tab visits its placeholders instead; once it has ended, Tab indents again. Outside editable source,
  Tab keeps cycling the controls as before.
- **Ctrl+/** (or **Alt+/**, because terminals often send Ctrl+/ as Ctrl+_) comments or uncomments the touched lines
  after their own indentation with the language's prefix (`//`, `#`, `--`), leaving blank lines alone and completing
  a partly commented run instead of removing it; markup without a line comment (`HTML`, `Markdown`, `CSS`) gets one
  block pair around the selection instead. A file type with no comment syntax (JSON, plain text) says so.
- **Ctrl+Shift+Down/Up** (VS Code keymap: **Alt+Down/Up**) moves the lines a selection touches one line down or up;
  **Alt+Shift+Down/Up** duplicates them directly below or above. A multi-line selection moves or repeats as a block,
  several selections move one block each in the same transaction, and the caret or selection stays on its own lines
  (a duplicate leaves the original selected). Each line keeps the newline written at its position—CRLF, LF, mixed
  endings and a missing final newline are preserved byte for byte—so the last line of a file has no line below it
  and the commands say "Already on the last line" / "Already on the first line" there.
- Typing an opening delimiter wraps a selection (`(` around `x` gives `(x)` with `x` still selected) or opens a pair
  with the caret between (`{}`, `[]`, `""`, `''` or a backtick pair); the closing delimiter already in front of the caret is
  stepped over rather than doubled, a quote after a word stays an apostrophe, and no pair is opened in front of a
  word or in Markdown, YAML or TOML. Pasting is always literal. Comment toggling and pairs read the language and the
  characters around the caret only; they do not parse strings, so a selection inside a multi-line string is
  commented like any other line.
- In an environment file (`.env`, `.env.local`, `app.env`; see [environment files](README.md#environment-files))
  Ctrl+/ comments with `#`; only `"` and `'` pair, and only right after an entry's `=` with nothing after the caret
  on its line (the typed closer then steps over the one at the line's end unless a backslash escapes it); a selection
  is wrapped in the quote typed; brackets, braces and backticks are always typed as they are, so `${VAR}` and `$(…)`
  stay exactly what was typed. The GUI does the same.
- In a Markdown draft Enter after a list item's text continues the list (bullet, next number with the following
  items renumbered, open task box) and Enter on an empty item ends it, one Undo step each; `](` and `#` in a link
  destination suggest project files and headings (see [Markdown](README.md#markdown)). The GUI does the same.
- In a YAML draft Enter starts the new line at its line's indentation, under a sequence item's content after `- `,
  and one indentation step deeper after a key that opens a block (`key:`, a trailing comment aside) or a block
  scalar (`key: |`); YAML indents with spaces only, whatever EditorConfig or the project says, two by default. A
  pasted line break is literal. The GUI does the same.

Read-only mode, an image, a preview, a retrieved or binary source, or a gated input backend disable all of these with
the same reasons as typing, and an open dialog takes the keys first; the empty document has nothing to comment,
move or duplicate and says so, while Tab still writes the indentation step.

**Editing Preferences…** (Find Action or the Edit menu, in both adapters) shows the active file's
indent style, indent size and tab width, each with its source: EditorConfig first, then the open project's
choice (**Project Indentation: Tabs / 2 Spaces / 4 Spaces / EditorConfig and Language Defaults**), then the
language default. It also names the formatter a project configures and where its executable was found;
resolution only reads files and never installs, upgrades or runs tools or environment files. **Format on
Save for This Project** is off by default; when on, Save asks the file's language server to format first (see
Format Document below). Project
choices are saved in Lenz Code settings file, never in the project. EditorConfig `{n..m}` ranges are not
matched. The indentation commands above use these preferences; indentation is not detected from the file's text.

**Format Document** (Ctrl+Alt+L; Alt+Shift+F in the VS Code keymap) asks the file's language server to format the
whole file with these indentation preferences, and **Organize Imports** (Ctrl+Alt+O; Alt+Shift+O) applies the
server's organize-imports action; each result is one Undo step of the draft and never writes the file. Only edits to
this file at the version shown are applied: an answer for text that has changed since, or an action that would also
change another file, is refused whole and says why. With **Format on Save for This Project** on, Save (Ctrl+S)
formats first and writes the formatted draft; the status bar says "Formatting before saving…" and Escape cancels. If
the server cannot be asked (none for the file, or it does not format), Save writes the draft unformatted and says
so; if formatting fails or the text changes meanwhile, nothing is written and the draft stays, and **Save Without
Formatting** (F10 or Find Action) writes it as it is. Lenz formats through the language server only; a formatter
the project configures is used when that server runs it.

**SQL Dialect** (F10 → SQL, in both adapters) declares which SQL dialect an `.sql` file is written for.
**Project SQL Dialect: PostgreSQL / DuckDB / Not Set** is the project default; **SQL Dialect for This File:
PostgreSQL / DuckDB / Follow Project Default** overrides it for one file, including a file outside the open
folder. The footer keeps the active choice visible next to the LSP status — `SQL: PostgreSQL (this file)`,
`SQL: DuckDB (project)` or `SQL: not set` — and clicking it, or **SQL Dialect…**, shows the effective
choice, the project default and what offline assistance this build has. An unset dialect stays unset: Lenz
never assumes PostgreSQL. Choosing a dialect only writes Lenz Code settings file (project choices under
`projects`, file choices under `sql_dialects`), never the document or the project, and opens no database,
DuckDB file or process. Offline assistance is not wired into the editor yet, and the details say so; the
next dialect or edit invalidates an earlier request identity, so a stale reply cannot publish.

The terminal emulator owns input-method preedit; Lenz receives committed Unicode text and bracketed paste.
GUI preedit and both adapters' long-line edits use the bounded source-coordinate path; see the
[composition verification and limits](../research/code-unicode-composition.md). The activation candidate
still awaits the combined correctness gate and performance acceptance.

**Code Completion** shows the language server's suggestions for the name at the caret. Typing a name (or a
server trigger character such as `.`) opens it unless **Completion While Typing** is off; Ctrl+Space asks
manually. Many terminals cannot deliver Ctrl+Space (some send NUL, which also works); **Code Completion** in the
F10 palette is the equivalent route. While the popup shows, Up/Down/Page Up/Page Down choose, Tab or Enter accept,
Esc dismisses and a click accepts a row; any other key, punctuation included, types normally and never accepts.
Empty or indentation-only lines and Enter never open automatic suggestions. Ctrl+Space still requests them there.
Rows show their kinds, separate adjacent kind groups and emphasize matching text without changing server ranking.
The selected documentation renders Markdown and syntax-highlighted fenced code. Wheel over it, or use
Alt+PageUp/PageDown/Home/End, to read the full document within the Markdown size limits; selecting another
suggestion resets its scroll. These documentation controls leave the draft and selected suggestion unchanged.
Acceptance is one undoable draft edit; suggestions carrying imports or server commands are listed but refused
whole. A snippet suggestion (`insertTextFormat: 2`) is expanded instead: its placeholders are visited by Tab,
`$0` last, and the status bar says which one is selected (`Snippet: placeholder 2 of 3 · Tab next · Esc done`).
Inserted lines take the file's indentation and line ending, a placeholder written more than once is one stop that
selects every occurrence, and Escape, a caret move, another tab or Undo ends it without editing text. A body this
editor will not expand (a placeholder inside a placeholder, a snippet variable or transform) is refused with its
reason. Inserting with several cursors is refused, since one Tab cannot visit two snippets. See the
[completion behavior and limits](../research/code-completion.md) and
[snippets and placeholder navigation](../research/code-snippets.md).

## Problems and XML

**Problems…** lists known parser and language-server reports across files. Tabs show `!N` errors (`?` while
rechecking), and explorer names turn red with a separate `•` for known errors, including containing directories.
The most serious message appears after the final wrapped segment of its starting line, with `(+N)` for others.
It clips to the available cells; Problems and the caret status show full text. These labels never enter selection,
Copy or Save. Empty/EOF findings retain a visible underline anchor. F8 / Shift+F8 navigate the current file's exact-version
underlines. The list names its coverage: unopened files are not scanned automatically, and old locations are
marked “last reported”. **Rerun Search** refreshes the list. Built-in syntax recovery works without an LSP;
it does not establish that a project compiles.

**Check Project Syntax** in F10 explicitly scans the captured project inventory, including unopened files,
without starting servers or writing files. Its results show progress, errors and coverage. Use **Cancel Project
Syntax Check** or close results to stop, and **Rerun Search** to check again. **Status details…** in F10 shows the
full coverage summary when the footer is too narrow. Ignored paths, unsupported languages, large or unreadable
files and validation limits remain explicit; see the [syntax-checking limits](README.md).

XML uses built-in highlighting, well-formedness errors and multiline folds, with tag-aware Enter indentation,
attribute quote pairing and XML comments. It needs no server and never resolves external entities. XML names
are case-sensitive. Internal entity declarations show a validation-limit notice while retaining highlighting
and folding; they are not marked malformed. Schema completion, schema validation and formatting are outside this pass. The GUI and
terminal use the same parser and editing commands.

## CSV tables

CSV's header **Table / Source** button and F10's **CSV: Table / Source** switch modes. Arrows move cells;
Enter edits, Ctrl+A clears the pending value, Enter commits and Escape cancels. Pending fields must be finished
before Save or closing. `/` filters the selected column by a literal substring; `s` cycles ascending text,
descending text and source order. Filtering and sorting affect the view only. Column move/drop and header-row
commands are in F10 and the context menu. Cell edits and column changes become undoable draft edits, written by
normal Save. The [CSV guide](README.md#csv-tables) describes quoted/multiline fields and size limits.

## Controls

F10 opens the command palette; `:` is an alias outside source input mode. Type to filter, use the arrows and Enter to run. It includes all shared
commands, their setting states and disabled reasons, including provider controls and features without a terminal
shortcut. F1 opens wrapped, scrollable help; F2 opens language-server status and recovery.
A dedicated toolbar above the tabs exposes Menu, navigation and the branch selector.
The footer reserves separate language-server, message, cursor-position and Commands areas.
Long messages end with an ellipsis: click the message or press Shift+F2 for a wrapped, scrollable
status snapshot; Ctrl+C copies the full text through the terminal clipboard. Routine server-memory
samples stay in details. Narrow layouts keep actions available through F10. Esc dismisses a modal before underlying
controls can receive input. Lenz Code modals also consume view-switching and quit keys. Ctrl-Q requests closure of the current independent view; F8 and Shift-F8 switch views. New Window
in the palette opens an independent terminal view. Ctrl-Shift-Q requests closure of all views in terminals that report it.
Before removing the last view of an unsaved draft, choose **Save** (`s`), **Discard** (`d`) or **Cancel** (Esc).
All-view closure prepares every session before removing any; Cancel keeps every view and draft. An earlier
explicit Save remains on disk even if another view cancels. Failed or uncertain Save cancels the pending close.
After an uncertain result, a fresh actual tab, split, window or all-view close offers **Discard Draft and Close**
without requiring a disk recheck. This discards the in-memory draft/history and leaves disk as it stands;
it never resumes the failed close or retries Save. Recheck remains available before deciding. Another Save
still requires **Recheck Saved File** and **Confirm Rechecked Save**; a changed disk result cannot be confirmed for Save.
Tab closure and Close Split keep a draft whenever another accessible tab still owns it.

A tab's right-click menu and F10 → File offer **Close All Tabs**, **Close Other Tabs**, **Close Tabs to Left**
and **Close Tabs to Right**. These affect the clicked tab's pane (the active pane in the command menu), in its
visible tab order; comparison sides close together. Every affected draft approves before any tab closes, so
Cancel or a failed Save retains the whole group. A buffer still open in the other pane keeps its draft.
Other/Left/Right on a comparison side keep both sides and at least one ordinary owner of its draft.

The wheel over a tab bar scrolls that pane's strip without switching files or focusing the pane. The GUI also
retains small trackpad movements. Activating a tab reveals it again, and resizing clamps the strip to its tabs.


Reload, changing folders/worktrees and checking out or creating-and-checking-out a branch use the same
Save/Discard/Cancel decisions. Their Discard decision after an uncertain Save still requires a successful
current-file recheck. The actual-close exception does not apply to Reload, project/Git transitions or external opens.
Cancel retains drafts and positions; earlier confirmed Saves remain written.
Failed or blocked Reload/checkout retains the draft and requires a fresh decision on retry. Watcher,
focus and index refresh retain drafts and undo history, with a visible notice when disk content changes
or becomes unavailable. Reload from a saved comparison's Current pane returns to the ordinary draft;
the Last saved pane remains read-only.

| Operation | Control |
| --- | --- |
| Search Everywhere / files / symbols / actions | Ctrl-K / Ctrl-P or Ctrl-Shift-N / Ctrl-Alt-Shift-N / Ctrl-Shift-A or Ctrl-Shift-P |
| Open file / go to line | Ctrl-O / Ctrl-G |
| New file / Save As | Ctrl-N / Ctrl-Shift-S (where the terminal reports it; also F10 → File) |
| New file or folder in the explorer | Right-click an explorer row, or File → New: New File in Explorer… / New Folder in Explorer…; type the name, Enter creates, Esc cancels |
| Copy / paste files in the explorer | Ctrl+C / Ctrl+V with explorer focus; right-click also copies the name, full/relative paths and opens file/folder Git history |
| Compare two explorer entries | Space on each (or Ctrl+click where the terminal reports it), then Ctrl+D; Esc forgets them |
| Open in editor / default app, compare with | File menu or right-click: Open in Editor, Open with Default App, Compare Saved Disk Inputs… (a typed path; Lenz Diff's terminal app runs in this terminal until it quits) |
| Find in file / next / previous | Ctrl-F or `/` / F3 / Shift-F3 |
| Replace current / every match | In find: Tab to the Replace field, Enter replaces the match; Alt-A replaces all (also F10 → Find) |
| Find options | Alt-C / Alt-W / Alt-R in find or search |
| Search project / file pattern | Ctrl-Shift-F / Tab in search |
| Reload / close tab | Ctrl-R or F5 / Ctrl-F4 (VS Code keymap: Ctrl-W closes) |
| Explorer visibility / focus | Ctrl-B / Ctrl-Shift-E |
| Focus tree, reading area and results | Shift-Tab; click a pane (outside source input mode Tab / Shift-Tab cycle; in editable source they indent and outdent) |
| Navigate tree or results | Arrows, PageUp/PageDown, Home/End; Enter opens |
| Move text cursor | Up/Down/Left/Right; Home/End for line bounds; Ctrl-Home/Ctrl-End for document bounds (Shift extends selection) |
| Scroll text | PageUp/PageDown; wheel (j/k and Home/End scroll while text input is gated) |
| Select / copy | Click or drag, double click word, triple click its syntax group, Shift-arrows, Ctrl-A / Ctrl-C |
| Git branches | Ctrl-\` (if your terminal sends it), F10 → Git, or click the branch in the top bar; type to filter, Enter checks out / opens a worktree, → or right-click for a row's actions; checkout problems: s Smart, f Force (f again to confirm), Esc cancels; new branch: Tab toggles Check out |
| Git comparisons | F10 → Git: Show Local Changes, Compare with Branch, Tag or Revision… (a list of branches and tags; type to filter or for any revision), Compare with Base Branch, Show Changes Since Branch Start, Compare with Worktree… (the other worktrees; Enter compares); a Git Branches row's → menu: Compare with Working Tree, Show Changes Since Branching from Here. Lenz Diff's terminal app runs in this terminal until it quits |
| Annotate with Git blame | Alt-B (View menu, right-click); click an annotation for its commit (d Show Diff, c Copy Revision); the status line shows the caret line's commit |
| Extend / shrink syntax selection | Ctrl-W / Ctrl-Shift-W |
| Indent / outdent the selection | Tab / Shift-Tab |
| Comment / uncomment | Ctrl-/ or Alt-/ (F10 → Edit when the terminal swallows it) |
| Move lines down / up | Ctrl-Shift-Down / Ctrl-Shift-Up (VS Code keymap: Alt-Down / Alt-Up) |
| Duplicate lines below / above | Alt-Shift-Down / Alt-Shift-Up |
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
| Keymap preset, rebinding, every chord | F10 → Application: Keymap…, Keymap: IntelliJ (Default), Keymap: VS Code, Reset All Keybindings |
| Recovered unsaved work | Offered at start; r Restore Draft, d Discard Draft, Esc Later; F10 → File: Recover Unsaved Draft… |
| SQL dialect default / this file / details | F10 → SQL; click the `SQL:` status label |
| Definition / peek / usages | F12 / Alt-F12 / Shift-F12 |
| Next / previous result; earlier / later result list | F4 / Shift-F4; Alt-Left / Alt-Right in the panel |
| Show / hide the results panel (keeps the results) | Ctrl-J; Esc in the panel stops a running search, then hides it; click `Results (N)` in the status bar |
| Type definition / implementations | Ctrl-Shift-B / Ctrl-Alt-B |
| Call / type hierarchy | Ctrl-Alt-H / Ctrl-H or Alt-H (many terminals send Ctrl-H as Backspace) |

**Search Everywhere** has the same All / Files / Symbols / Actions scopes as the GUI. Type to fuzzy-match,
Tab / Shift+Tab changes scope while keeping the query, arrows/Page/Home/End select, Enter opens/runs and Esc
closes. Scope tabs and rows are clickable when they fit; narrow windows show the current scope and keep Tab
cycling. Results show their kind, path or action group, setting value, shortcut and disabled reason; only visible
rows are formatted. Long Unicode queries keep their typed tail and caret in view. Empty queries favor recent
files; file searches retain `name:line[:column]`. The shared core caps results at 100 and rejects stale replies
and clicks. Symbol search uses the project declaration index without requiring an LSP and reports its limits.

**Recent Files (Ctrl-E)** uses the same searchable rows, file icons and secondary paths, limited to files
already visited. Empty queries keep the previous file first for quick Ctrl-E, Enter switching; typing fuzzy-matches
names and paths. Delete forgets an entry without deleting its file. In popups, the wheel scrolls the viewport
without changing the selected option; keyboard navigation selects and reveals an option.

**Double Shift** also opens All when the terminal sends bare modifier press/release events through the enhanced
keyboard protocol. Lenz Code requests these reports and restores the previous mode on exit. Terminals that report
only a bare Shift's release (foot) count that release as a tap. Legacy terminals,
multiplexers and SSH paths may omit them; **Ctrl-K** and the catalog's Search Everywhere always work. Shifted
typing, selection, held-key repeats and focus loss do not count as double Shift. Ordinary held arrows and query
characters continue to repeat. Ctrl-Shift-N now finds files; New Window remains in the command palette.

The toolbar's **☰ Menu** opens the same dropdown as the GUI's ☰ button: the File, Edit, View, Navigate, Find, Git, Language Servers
and SQL groups with task submenus, All commands…, then below a line Change theme, About, Keyboard Shortcuts, Close
Window and Quit All Windows; Enter or → opens a group, ← or Esc goes back, and a
disabled command's reason shows in the footer. The F10 palette labels commands with the same groups; type a group
name followed by `:` to filter that group. The GUI additionally has a clickable
Commands button in its status bar; the terminal keeps F10 / Ctrl-Shift-P access to the same catalog.

The toolbar follows the GUI: explorer toggle, Back/Forward, SVG Source/Image and the Git branch selector. Find File, Search and comparisons remain in their menus and shortcuts.
Markdown's **Preview / Source** button sits at the right edge of its file header; each split pane controls its
own document, focusing that pane before switching. **Open Markdown Preview to the Side** (F10, the source's
right-click menu) shows a preview beside the source that follows the typing; a left click in a preview sets the
position for **Show in Source** and **Follow Link** (also in its right-click menu), and Ctrl+click follows a link
there or shows the clicked text in the source, where the terminal delivers Ctrl with mouse clicks (the menus work
everywhere). The explorer title has Select Opened File. Narrow panes omit controls that do not
fit; every action remains available through F10. Left-click selects source text; right-click in either reading
pane opens the same contextual command list as the GUI, with shared availability and separators. Clicking
outside a menu dismisses it without acting on the document underneath.

The footer keeps a clickable LSP status before paths and notices. Click it, press F2, or choose **Language Server
Status…** in F10 for the compact actions menu. Choose **Server Details…** for the full error, setup guidance,
project settings and indexing context, the executable and its Java or Node.js runtime, the names of environment
variables the provider settings add (never their values), and which assistance the running server offers or does
not offer (each missing command stays disabled with that reason). Typing, Undo, Save and text search never wait
for a server that is missing, failing or indexing. Its clickable **Copy** and **Close** controls stay visible while scrolling;
Ctrl-C copies the full, unwrapped snapshot through OSC 52, and Esc closes it. Reopen the popup to refresh its
contents. **Copy Language Server Details** remains in F10. The menu offers Restart, Enable/Disable for This
Folder and Set Workspace Root with shared disabled reasons. Recovery actions stay in fixed positions while
background status changes. **Install Language Server…** previews curated npm installs and requires explicit Start (`S`/Enter); progress/output, Cancel (`C`), Server Details (`D`) and Close/Escape are in its focused dialog. Missing managers and unsupported servers show a manual setup action; see [managed installation](providers.md#explicit-managed-installation). Other missing servers need user installation or an executable/PATH correction; restarting
rechecks setup. Disabled servers stay off until explicitly enabled. Unsupported documents explain that no server
is available; text search and syntax highlighting remain usable. See [provider setup](providers.md#status-and-recovery).

F2 and the Language Servers command group also offer **Exclude Ruby Tooling Gems for This Project** and
**Reset LSP Settings for This Project**. These use the resolved server workspace, save outside the checkout,
and restart only the affected provider/workspace. Exclusions default to On and reduce navigation into tooling
gems; toggle them Off to restore that coverage without clearing unrelated settings. Reset restores inherited settings for that provider/project. See [project settings](providers.md#project-language-server-settings)
for JSON overrides, precedence and legacy Ruby `.index.yml` handling.

The same menu offers **Reuse Saved Declaration Index: On/Off** (On by default) and **Rebuild Project Index**.
Rebuild discards the current project's saved declarations, refreshes open documents and restarts its language
servers while rebuilding local symbols. Both controls also appear in F10 and Find Action. See
[cache and rebuild controls](providers.md#saved-declarations-and-rebuilding) for settings and limits.

The finder accepts `name:line[:column]`. Go to line accepts `line`, `line:column` or `line,column` and reports
invalid input. Find updates while typing; Enter leaves its matches available for F3, while Esc closes find.

The find prompt's second field is the replacement text: Tab gives it the keys and Shift-Tab returns to the query,
Enter in the Replace row replaces the current match and keeps the prompt open for the next one, and Alt-A replaces
every listed match. The
prompt's footer shows the counter, "No matches" or the expression's error. The replacement is literal (no `$1`
groups), an empty one deletes the match, each replace is one undo step and only the current draft changes; Replace
All refuses a search capped at 10,000 matches. Read-only files and previews refuse replace and say why.

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

Nerd Font glyphs decorate the command bar, explorer, tabs, search results heading and status bar by default,
as in the comparison TUI. **File Icons** in the View menu turns them off (and on again); the choice is
remembered and shared with the GUI reader, which then hides its file icons too. `LENZ_ICONS=plain|nerd`
overrides it for one run, until File Icons is toggled. Configure a [Nerd Font Mono](https://www.nerdfonts.com/)
in your terminal, for example JetBrainsMono Nerd Font Mono, so icons occupy one cell; fonts are never installed
or changed by the app. Active reading tabs use
bold accent text and an underline when focused; pane headings and rounded dialogs follow the selected theme.
Files use the GUI reader's file-type glyphs and colors, including special names such as `Cargo.toml`,
`Dockerfile` and `.gitignore`. Folder icons distinguish open and closed directories.

`NO_COLOR` disables **all** terminal colors, including syntax and diff highlighting. If the entire app is
monochrome, use `env -u NO_COLOR lenz --tui` with the staged launcher (or run
`lenz-code-tui` directly). The app leaves your terminal and shell configuration unchanged.

**Change theme** is an application command in the F10 menu. Its chooser highlights the current selection and previews themes with arrows, Home/End and Page Up/Down. Enter or a click saves the choice; Escape or an outside click cancels the preview. The wheel scrolls the list without changing the highlighted theme.

Themes use the shared Lenz configuration: `lenz/appearance.json` and `lenz/themes` under
`$XDG_CONFIG_HOME`, with `LENZ_THEME` taking precedence over `LENZ_APPEARANCE`, `LENZ_CODE_THEME`,
`LENZ_CODE_APPEARANCE` and the legacy `LENZ_READER_*` variables. Explorer order, wrap, sticky scroll, semantic settings, split orientation and saved
places use the GUI's `lenz-code` configuration and state directories. See [Themes](../themes.md)
for precedence and compatibility fallbacks. Follow system falls back to a dark palette
when the shared theme service cannot detect appearance. There is no native file picker: Open File, Open Folder
and Workspace Root accept paths. Opening another view creates another session with its own documents and places;
it does not mirror a live document from a different view.

Clipboard transport is OSC 52: the terminal or multiplexer must permit it. Source text is base64 encoded before
entering the escape sequence. Legacy terminals cannot distinguish every Ctrl/Shift combination; use the palette
for any shortcut your terminal does not deliver. The explorer's edge, the split panes' divider and the results
heading drag; mouse tab dragging and the divider between the results list and its preview are not available; tabs
can be reordered with the keys or moved between panes through the palette.

Results panel and terminal differences from the desktop, by design of a character grid:

- Below 64 columns the results panel shows its list without a preview; the desktop puts the preview below the list.
- Up and Down in the results list also stop on folder and file rows (Enter or Space folds them); the desktop's Up and
  Down move between matches and its folder and file rows fold by click or Left/Right. Left/Right, Tab and clicks
  behave the same in both.
- The terminal panel's header buttons read `Stop`, `Close`, `Hide`, `+ New`, `<` and `>` (single characters when narrow);
  the desktop's read `Stop`, `×`, `Hide`, `+`, `‹` and `›`. Both run the same commands. Native GUI text shaping and operating-system primary selection
are not available in the terminal. The terminal restores raw mode, mouse/focus/paste reporting and the alternate
screen when it exits normally or returns an I/O error; an uncatchable process kill cannot run cleanup.

## Verification

```sh
scripts/ci-local.sh
cargo test --locked -p code-tui --features gui-test parity_tests
python3 scripts/code-tui-bench.py --binary target/release/lenz-code-tui \
  --out perf-results/code-tui-check --lines 100000 --rounds 30 --stress-seconds 60
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
describes historical Lenz Code measurements with different workloads.

For a Java server runtime failure, F2 → **Choose Language Server Runtime…** offers installed executables and a path/JDK
home field. Save validates the executable, persists the override and restarts enabled Java servers; Escape
before saving cancels. The GUI exposes the same commands. See [provider runtime repair](providers.md#repairing-a-java-server-runtime-in-the-reader)
for validation and installation limits.


Python failures use the same runtime dialog to choose an existing Node.js installation containing Pyright.
F2 shows the detected/tested server version, bounded launch log and selected runtime version when available;
**Copy Language Server Details** copies the same diagnosis. See [version context and runtime repair](providers.md#version-context-and-pyright-runtime-repair).

Haml templates (`.haml`, including `.html.haml`) suggest tag names after `%` and attributes inside `(…)`
with Ctrl+Space. Enter follows Haml indentation. Format Document uses installed haml-lint when the project
has `.haml-lint.yml`; each result is one Undo step, and only Save writes it. Ruby completion, Parameter Info
and problems use exact template locations. See [Haml capabilities and limits](providers.md#haml-editing).

For PostgreSQL SQL drafts, choose the dialect in the command menu. Ctrl+Space completes query-local
names, Parameter Info shows offline builtin help, Problems/F8 show native syntax errors, and Format
Document uses the guarded offline formatter. The GUI exposes the same session behavior. See
[PostgreSQL offline editing](providers.md#postgresql-offline-editing).

File-operation prompts are compact labelled forms with clickable Cancel and action buttons; Enter and Escape
retain their existing behavior. Validation stays in the prompt. The branch filter stays above its results, and
details keep their actions below the scrolling text. When a details footer is too wide, its buttons stack so
choices remain visible. Server actions separate setup and indexing, and menus show shortcuts beside their labels.

### Activity Log

**Logs (N)** in the bottom bar opens a pane for session notices, warnings and SQL metadata connection
progress/failures. It starts collapsed; **View → Layout → Show / Hide Activity Log** also toggles it.
Use the wheel or Up/Down, Page Up/Down, Home/End while the pane has focus. **Copy** (Ctrl+C) copies
its retained text, **Clear** removes it, and **Hide** or Escape collapses the pane. Tab returns keyboard
input to the editor. The latest 256 entries are kept in memory, up to 8 KiB each, with UTC timestamps;
they survive file switches but are not saved between sessions. New entries follow the bottom when
scrolled there. This is application activity, not an unrestricted capture of every child process's output.
