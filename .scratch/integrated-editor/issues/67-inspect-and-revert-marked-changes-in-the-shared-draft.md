# 67: Inspect and revert marked changes in the shared draft

**What to build:** Inspect prior content, open the editable comparison and revert a marked change with undo.

**Blocked by:** 66: Mark draft changes against HEAD or the last save.

**Status:** resolved

- [x] Provide marker and keyboard/menu access to prior content, Show Diff and Revert This Change in both adapters.
- [x] Label HEAD versus last-save baseline and share the existing draft/history in the comparison.
- [x] Revert changes the draft as one undoable action, writes only through Save, leaves the Git index unchanged and rejects obsolete baseline/hunk coordinates.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 107, 108, 109, 110.

## Answer

Merged at `6001628` (implementation `016ae18`). Gutter clicks, Alt+Shift+C and the shared Git command menu open prior content for a settled marker. Both adapters show the named HEAD commit/last-save/new-file baseline, Show Diff and Revert This Change. Show Diff opens the captured read-only baseline beside the existing editable draft and history; closing restores the original view. Revert is one normal undoable draft edit, with exact original text/endings, and writes only through Save. It never changes the Git index.

The existing background marker comparison now retains byte hunks and a shared indexed baseline without a second diff on click. Inspections reject changed document/source/baseline/epoch coordinates; repository reverts additionally check HEAD (or continued absence from HEAD) off-thread before publishing. A deletion at EOF is reachable on the last visible line, including restoration of its preceding separator. Preview is bounded to 64 KiB and prior revert text to 4 MiB, with a visible reason and Show Diff for larger blocks. Existing line-comparison limits for ending-only changes remain explicit.

Eleven marker/core tests pass, including insertion/deletion/replacement, CRLF/Unicode, EOF, new files, stale coordinates, an external HEAD change before polling, shared comparison history and unchanged file/index bytes until Save. Actual GUI/TUI input/render replays pass for HEAD and last-save baselines: gutter click, keyboard inspector, modal Save blocking, Show Diff, editing/Undo, Revert/Undo, narrow layouts and explicit Save. The replay caught and fixed a TUI inspector that opened only at draw time; synchronous command dispatch now installs it before the next key. Seven saved-comparison, nine reconciliation and eighteen keymap regressions, empty command-catalog expectations, existing marker/comparison replays, strict touched-crate clippy and both post-merge ratchets pass. Private evidence is under perf-results/editor67. End-of-epic latency/memory/stress and native macOS/Wayland/physical-terminal checks remain deferred/unverified; costs and the existing TUI Home/End limitation are recorded in editor follow-ups.
