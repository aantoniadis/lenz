# Reader basic text input foundation

Ticket21 completes the existing opt-in input route for printable typing, selection replacement,
Cut/Copy/Paste, Delete/Backspace, Enter, Tab and shared Undo/Redo in both Reader adapters.
**Production writable input remains gated until safety checkpoint23.** There is no CLI, setting
or user-facing enable option. Existing test opt-ins enable the shared Session capability; normal
constructors keep it false. Approved direct `Command::Edit` tests retain their existing seam.

Cut, Paste and Delete use shared commands, permissions, labels and disabled reasons. Cut copies only
when its exact selection deletion succeeds. Clipboard and bracketed paste each enter one transaction;
terminal q/j/k/slash/Space insert literally in editor input, including inside pasted text. Native GUI
clipboard delivery uses the platform clipboard. The TUI emits copied text through its existing terminal
clipboard path; its Paste action explains terminal paste delivery (usually Ctrl+Shift+V), rather than
claiming it can synchronously read the emulator's clipboard.

**Read-only Mode** is a shared per-buffer permission, reflected in both menus and status details.
Split and comparison current views share it. It blocks editing, Undo/Redo and new Save requests without
clearing history; Copy remains available. Turning it off cannot override intrinsic image/binary/lossy,
retrieved-source or saved-side restrictions, and cannot enable the production input gate. This flag has
no new persistence setting. Repeated blocked text input is consumed without turning into reading shortcuts
or accumulating one diagnostic message per character. Existing modal input keeps precedence over typing,
clipboard mutation, deletion and Save.

Deletion follows the existing logical UTF-8 scalar boundaries, with CRLF treated as one newline. The
focused fixtures explicitly remove/undo a combining scalar and the final scalar of a family-emoji sequence;
this is not a claim of whole-grapheme deletion. Enter uses the confirmed file's CRLF preference (otherwise
LF); Tab inserts a literal tab. Neither operation reformats the remaining source. Project indentation,
keymap policy, native composition and long-line composition are separate tickets. No input/render path
flattens the current source for these operations; an explicit clipboard copy allocates its requested text.

Focused evidence is retained under `target/perf/editor-implementation/21-text-input/`:

- Core checks exercise default gating, per-buffer read-only permissions across split views, clipboard
  effects, the intrinsic saved-side restriction, exact scalar/CRLF deletion, grouped Undo and unchanged disk bytes.
- Real GUI and standalone TUI constructor/event traces cover gating, Delete/Backspace, Cut/Paste and Undo/Redo.
  The readonly terminal `q` regression failed before the shortcut-fallthrough fix.
- A combined real-input replay compares semantic state and exact Unicode text after literal keys, paste,
  CRLF Enter/Tab, Undo, both actual F10 read-only controls, blocked operations and modal input. Files stay unchanged.
- The first GUI deletion tracer failed before implementation. An initial combined-menu oracle omitted the
  existing `(off)` label suffix; the corrected lookup retains the same actual menu interaction and assertions.

Final focused results: 3 core input checks, the command catalogue, one GUI and two standalone TUI
input traces, the combined input replay and the existing split-history parity regression pass. Ordinary
GUI/TUI compilation and all three affected strict Clippy checks pass. Local links and `git diff --check` pass.
The next combined batch owns the full gate, release comparisons, memory and sustained stress; no per-ticket
suite or timing/stress campaign ran here. Native macOS/Windows, physical clipboard/input delivery, IME/preedit,
long-line composition and physical display latency remain unverified by ticket21.
