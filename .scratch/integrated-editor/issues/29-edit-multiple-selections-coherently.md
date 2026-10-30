# 29: Edit multiple selections coherently

**What to build:** Create and edit several selections with predictable shared undo in either frontend.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Insertion/deletion/paste applies one validated nonoverlapping transaction with stable per-cursor positions.
- [x] Define and test overlapping/duplicate selections, multiline Unicode edits, differing contexts and undo/redo.
- [x] Provide usable keyboard/menu routes in both adapters and preserve modal/read-only behavior.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 11.

## Answer

Merged in 7f31676 (branch work/editor-29-multi-selection, implementation 78acb5d, keymap-merge 4487991; conflicted with ticket 26 and was resolved by the implementer against the integration tip). `selection::normalize` keeps ranges ascending/disjoint; insert/cut/paste/delete build one `Vec<TextEdit>` through the single `Session::edit` path with stable per-caret positions, one undo grouping per typing burst. Chords come from the keymap presets (IntelliJ `Alt+Shift+Down/Up`, `Alt+J`; VS Code `Ctrl+Alt+Down/Up` plus the Alt+Shift aliases); Escape collapses to one cursor as a fixed editing key. Core 597, GUI 185, TUI 131/158, lenz GUI 261 passing; clippy clean; panic-site ratchets at baseline. Limits: motions/clicks collapse to the primary caret, IME and completion acceptance stay single-cursor, extras are not persisted; see docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign per user.
