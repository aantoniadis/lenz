# 66: Mark draft changes against HEAD or the last save

**What to build:** See modified, added and deleted lines in the editor against a clearly identified baseline.

**Blocked by:** 65: Show Git status separately from draft state.

**Status:** resolved

- [x] Use HEAD for tracked files and last save outside Git; include unsaved changes and retain repository markers after Save.
- [x] Render blue modified, green added and deletion-boundary markers with non-color equivalents, coexisting with folds/line numbers/bookmarks/blame.
- [x] Recompute safely after undo/HEAD/status changes without stale publication or viewport-wide recomputation, and identify new/untracked files honestly.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 102, 103, 104, 105, 110.

## Answer

Merged in 569e94c (branch work/editor-66-draft-markers, 0e2f729, from 9955ce0; clean). A draft is compared against HEAD for tracked files and against its last saved version outside Git; unsaved edits are included and repository markers survive Save. Marker letters `M`/`A`/`D` are drawn in their own colours with the letters as the non-colour equivalent and share the gutter with folds, bookmarks, line numbers and blame in both adapters (GUI keeps its gutter width and uses the left margin; TUI reserves two cells). Line answers come from a binary search over sorted ranges, so only drawn lines ask; a comparison is requested only for shown buffers when the (version, path, status, epoch, last-save) tuple changes and only after a 300 ms rest, through the shared admission slot fenced by project, request and the exact draft snapshot, with superseded runs cancelled. New/untracked files name their own baseline (`new file (untracked)`, `new file (added to Git)`) instead of a commit. No new commands (ALL stays 157). Filtered core/TUI/GUI sets passing, clippy clean over reader-core/lenz-content/lenz-gpui/reader-gui/reader-tui, panic-site ratchets ok. Limits: marker lag while catching up, pairing inside replaced blocks, GUI margin crowding at 5+ digits, other-repository files use last save — see "Draft change markers" in docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.
