# Project Tabs and Format Selection

Status: ready-for-human

Two user-requested features for Lenz Code, implemented in the shared core with both adapters.

## Project Tabs

Several projects in one window (GUI) or terminal (TUI), each a **Project Tab** (see `CONTEXT.md`): its own
`Session`, folder, tabs, explorer, language servers and drafts. Shown one at a time, or Side by Side as 2…n columns.

- Core: `code_core::projects` owns the policy: order, active project, `Layout::{Tabs, SideBySide}`, how the active
  project moves when projects open and close, which projects a layout shows for the room there is
  (`MIN_COLUMN_CELLS` = 36), column division, titles and unsaved marks. Six catalog commands
  (`NewProjectTab`, `OpenFolderInProjectTab`, `CloseProjectTab`, `Next/PreviousProjectTab` on Ctrl+Alt+] / [,
  `ProjectsSideBySide`) resolve labels, availability and the toggle centrally from the `Info` each host hands its
  sessions; adapters handle `InterfaceAction::Project` exhaustively.
- GUI: `ProjectsWindow` is every production window's root. With one project it renders that `ReaderView` alone and
  unframed (unchanged). With more it draws the strip and gives each shown view a frame; `ReaderView` lays out in its
  frame's size and translates pointer and IME coordinates by its origin. Closing goes through the shared close
  protocol per session; a commit removes one project, the window closes with its last project.
- TUI: the existing independent views are the project tabs. With one view, drawing is unchanged. With more, the
  runtime draws the strip and renders each shown view into an off-screen `TestBackend` of its column size, copied
  into place with the active view's cursor; pointer input is routed by column and translated.

## Format Selection and offline formatting

- `FormatKind::Selection` / `CommandId::FormatSelection`: format the primary selection as JSON or XML (the file's
  type, else what the selection starts with), re-indented to its line; one undo step; version-fenced like every
  formatter. Ctrl+Alt+L (Format Document) runs it when the selection is well-formed JSON or XML (checked
  synchronously up to 256 KiB); otherwise Format Document formats the file as before, so HTML/JS/template users with
  an arbitrary selection keep their server or project formatter. Templates (ERB, Haml) never count as plain text.
- XML gets an offline whitespace-only formatter (element-only content re-indented; text, CDATA, mixed content,
  `xml:space="preserve"`, tags and attributes kept as bytes; malformed XML refused; entity-declaring documents left
  alone). Format on Save now formats XML files too.
- A plain-text document whose text starts like JSON or XML formats as that on request; Save never guesses.

## Welcome screen and opening projects

- `code_core::start` owns the shared presentation: names and `~` locations, badge initials and colour choice, the
  welcome copy, the filtered recent-project list and the Open Folder prompt's folder suggestions (one bounded
  directory read per typed input, cached by the terminal adapter). The start screen's keyboard selection is core
  state (`Command::StepStart`, `State::start_selected`).
- GUI: a centred welcome column (title, tagline, sections with rules, badged rows with right-aligned shortcut or
  location, hover and keyboard highlight, tip); Open Recent Project as a finder (heading, filter, location column,
  hint footer, Ctrl+Enter for a new project tab). Open Folder keeps the system folder chooser.
- TUI: the same welcome column in cells; the Recent prompt filtered with locations; Open Folder prompts list recent
  projects, then live folder suggestions with Tab completion and Ctrl+Enter.

## Verification (2026-11-27, Linux container, no display)

- Core: `projects` unit tests, catalog/availability and title tests, XML formatter tests, Format Selection /
  plain-text / XML session tests (`cargo test -p code-core`). One pre-existing failure unrelated to this work:
  `lenz_reader_folders_move_to_lenz_code_once_and_never_lose_settings` fails when run as root (chmod 555 does not stop
  root); it fails identically on the base commit.
- TUI: real input through `dispatch_input` / `project_pointer` / `open_new_view` (menu command, typed path, Ctrl+Alt+[,
  strip layout button, column click, close mark), unchanged single-project frame, tiny terminals; Ctrl+Alt+L on a
  JSON selection with Ctrl+Z.
- GUI (GPUI test platform): menu New Project Tab, Ctrl+Alt+[ / ], layout button, per-column layout width, click
  focus and coordinate translation, a click on the right column's menu button opening that project's menu only,
  tiny windows, unsaved-project close (Escape keeps it and the file; Discard closes only it), Ctrl+Alt+L on a JSON
  selection with Ctrl+Z.

Not verified: release performance measurements (issue 04), physical displays, Wayland input and native macOS.
