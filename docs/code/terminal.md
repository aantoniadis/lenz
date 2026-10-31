# Integrated terminal

Lenz Code on Linux and macOS includes a terminal panel with multiple sessions. Choose
**View → Terminal → New Terminal**, press **Ctrl+Alt+T**, or use **F10** to find a
terminal command. The panel uses Alacritty's terminal model and the same shared
session behavior in the desktop and terminal editors.

Each terminal tab has its own interactive shell, working directory and scrollback.
Choose **View → Terminal → Rename Terminal Tab…** (or find it with **F10**) to name
the active tab, for example “Codex” or “SQL”. Enter applies the name; Escape cancels.
Names can contain up to 128 characters. Renaming keeps the shell, history and
working directory, and the name survives Restart Terminal for that tab.
Use **+** to create another, click a tab to switch, or use the previous/next controls
when the window is narrow. New terminals start in the project folder, the open
file's folder when there is no project, or Lenz's starting directory. An existing
session keeps its directory when you switch files or projects; the panel identifies
its *initial* directory. The shell comes from `SHELL`, with `/bin/sh` as the fallback.

| Action | Default shortcut |
| --- | --- |
| Show / hide terminal | Ctrl+Alt+T |
| New terminal session | Ctrl+Alt+N |
| Focus editor | Ctrl+Alt+0 |
| Paste selected source into the active terminal | Ctrl+Alt+Enter |
| Previous / next terminal tab | Ctrl+Alt+PageUp / PageDown |
| Copy terminal selection | Ctrl+Shift+C |
| Paste clipboard into the terminal | Ctrl+Shift+V (terminal editor: the host emulator's paste) |
| Find terminal output | Ctrl+F (GUI also Command+F) |
| Scroll terminal history | Shift+PageUp / PageDown, Shift+Home / End, mouse wheel |
| Open Lenz commands while terminal has focus | F10 |

The commands remain in the menus if the host terminal cannot deliver a shortcut.
While the panel has focus, Enter, Tab, Escape, Ctrl+C, Ctrl+D, Ctrl+S, Ctrl+Q,
Ctrl+V and Ctrl+Z go to the terminal program. **Ctrl+Alt+0** returns the keyboard to the editor.
Lenz dialogs take input before the terminal and the documents beneath them.

## Find terminal output

With the terminal focused, **Ctrl+F** opens Find for that tab's retained scrollback
(the GUI also accepts **Command+F**). **Find in Terminal…** is available in the
terminal command menu. Type or paste a single-line literal query; matching is case
sensitive. Enter finds the next occurrence, Shift+Enter the previous one, with wrap
at either end. The current occurrence is highlighted and scrolled into view.
Backspace edits the query, Ctrl+A clears it, and Escape closes Find and returns
input to the terminal program. Search accepts up to 1024 UTF-8 bytes.

Find consumes typing, paste and Enter without sending them to the shell. Close it
before dropping or pasting file paths. Switching tabs, hiding the panel, stopping
or restarting the terminal closes Find. New output and size changes clear the
current highlight and show **Output changed · Enter searches**; Enter searches the
updated history. Output and ordinary repaints do not repeatedly scan scrollback.
Natural shell exit keeps searchable history; explicit Stop or a worker failure
keeps only the last screen, so Find is unavailable until Restart.

## Paste source text

Start the program you want in a terminal tab: for example `psql`, `duckdb`, `python`,
`codex` or `claude`. Return to the editor, select source text and choose **Send
Selection to Terminal** from the text menu or press **Ctrl+Alt+Enter**. Select All
first to send a whole file. The text comes from the current draft, including unsaved
edits. Sending it changes neither the draft nor the file on disk.

Sending **pastes** into the selected terminal tab and focuses it. Lenz adds no Enter;
press Enter yourself when you want the receiving program to act. Multiline text
and tabs require that program to enable bracketed paste. Without it, Lenz refuses
the send and explains why, because the embedded newlines could otherwise execute
commands. A program can still act on input however it chooses; bracketed paste is
a terminal protocol, not a sandbox.

Ordinary clipboard paste uses the same checks. In the desktop press Ctrl+Shift+V;
in the terminal editor use the host emulator's paste, usually Ctrl+Shift+V. A paste is limited to 1 MiB; control characters that could inject
terminal input are refused. Rejected sends write no partial prefix. The panel shows
the reason, and **Status details…** in F10 shows the full message.

## Paste a file path

In the desktop, drag a file from the explorer, an open file tab or your file manager
into the terminal's cells. Lenz pastes its absolute path with POSIX shell quoting;
spaces and apostrophes stay part of the filename. Several paths are separated by
spaces. Nothing is executed or saved, and a drop elsewhere does not send a path.
Untitled documents need to be saved before they have a path to send.

The explorer and file-tab context menus also offer **Paste File Path to Terminal…**.
From the command menu, that command asks for a path. Use this route in the terminal
editor: native file dragging is controlled by the host emulator, which may deliver
only pasted text rather than a file event. Paths containing control characters or
names that cannot be represented as terminal text are refused.

For an agent, paste a path as part of your request and submit it yourself. Lenz
does not implement an agent-specific attachment API; the running agent decides how
to use that path. File drops send the path, not file contents or unsaved edits.

## Hide, stop and close

**Hide** keeps every terminal session running and draining output. Natural shell
exit leaves the final output and exit status visible; **Restart Terminal** starts
a new shell in that tab's initial directory. **Stop Terminal…** ends the selected
session and keeps its last screen. **Close Terminal Tab…** removes that tab.

Stopping or closing a running terminal asks first. Closing a Lenz view also asks
about its running terminals and any unsaved drafts; Cancel keeps them. Confirmation
ends the owned shell and its attached foreground, background and stopped jobs.
Jobs you explicitly detach into another operating-system session are outside that
ownership. Terminal sessions and transcripts are not restored after Lenz exits.
The feature adds no transcript logs or recovery records; ordinary editor draft
recovery continues to protect source edits.

## Capability limits

- Terminal process ownership requires Linux with pidfd support, or macOS. Other
  platforms do not offer the terminal.
- **macOS support has never been compiled or run.** It was written on Linux and
  is first built by the next tagged release (`release.yml`). There, a thread waits
  for the shell's exit without reaping it (`waitid` with `WNOWAIT`), the session's
  processes are found with `proc_listallpids` and `getsid`, and the reactor waits
  with `select(2)` because macOS `poll(2)` does not support devices such as the
  PTY master. Without pidfds, a session member that exits between its check and
  its signal leaves a short window in which its PID could be reused. Check first
  on a Mac: output appears in a new terminal (else the PTY wait is broken), and
  closing a tab running `sleep 1000 &` leaves no `sleep` behind (else `getsid`
  is refused across sessions, and `sysctl` `KERN_PROC_SESSION` is the fallback).
- The adapters render terminal cells with Unicode, ANSI/true color, alternate
  screens, ordinary control/function/cursor keys and supported mouse reporting.
  Graphics protocols, clickable terminal hyperlinks, OSC clipboard access and
  extended Kitty keyboard negotiation are not enabled.
- The GUI's upstream windowing layer removes the distinction between numeric
  keypad and ordinary keys. Normal keypad typing works; application-keypad mode
  is available in the TUI only when the host reports keypad identity.
- History is bounded by both line count and cell storage. Terminal dimensions and
  per-cell/control-string payloads are bounded too; older output can be evicted.
  Memory grows with the number of deliberately opened terminal sessions.
- Layouts differ between adapters. Both retain command-menu access and terminal
  session switching in narrow windows; native file drops have the TUI limitation
  described above.
