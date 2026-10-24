# Reader comparison with the last saved version

Ticket20 adds **Compare with Last Saved Version** (Ctrl+Shift+D) to both Reader adapters.
**Compare saved** in the toolbar opens it; **Close comparison**, Ctrl+F4 on a comparison tab,
or Close Split returns to the original views. Narrow windows retain the palette commands.

The left side is immutable confirmed text; the current side is another view of the ordinary buffer,
with its shared edit/Undo/Redo history. Original pane identities and reading positions survive the
comparison. Changed source rows have a blue background and a gutter mark. The two source panes
scroll independently; line-ending-only differences are reported in status. This minimal comparison does not align unchanged rows or draw connectors.
Production typing remains gated by editor tickets21/22; edit input replays use existing opt-in test support.

The saved side has a distinct temporary BufferId backed by shallow-shared snapshot chunks. It has no
alias, writer provenance or draft, and cannot become project-search, declaration-index, navigation,
language-server, watcher or workspace authority. Normal bounded Reader projection/layout/Find/syntax
work can therefore run for both versions without competing for one current-source analysis group.
Both temporary tabs and the synthetic buffer/analysis ownership are explicitly removed on closure.
Closing an ordinary owner first ends the temporary comparison, then evaluates its dirty-state guard.

Both Documents are admitted through the existing bounded conversion service before a typed comparison
result is queued on the existing Jobs executor. The workspace diff engine computes line changes off-thread.
No input or render path flattens text. Project, comparison request and both text/metadata identities fence
publication; changes disappear immediately when either input changes. A confirmed captured Save advances
only that baseline, leaving a newer current draft dirty. Rejected/uncertain Save retains the prior side.
Same-key backing compaction rebinds the saved side without inventing a semantic version or retaining its
old backing indefinitely. Temporary pane layout is not written into settings/workspace state.

Focused evidence is retained under `target/perf/editor-implementation/20-saved-comparison/`:

- Session tracers cover shared ordinary/split/comparison edits and history, immutable old-side commands,
  simultaneous different-version Find, source authority, captured Save/newer edits, stale replies,
  rejected/uncertain outcomes, split restoration, compaction, a pending Reload, line-ending-only changes
  and ordinary-owner dirty closure.
- Real GUI/TUI key and render replays cover modal priority, both side labels, changed-row presentation,
  shared Undo/Redo and Save, attempts to modify the old side, tiny sizes and return to the ordinary tab.
  Both supported appearances are checked. Disk bytes and buffer identities are asserted.
- Initial tracer compilation failed before implementation. Early adapter oracles incorrectly expected an
  offscreen changed line and used Ctrl+W (Extend Selection) for Close; they now use real Go To and Ctrl+F4.
  The ordinary-owner integration regression failed before its shared close-target fix.

Final focused results: 10 comparison-filter checks, 25 close-filter regressions, the command catalogue,
one real comparison replay and one toolbar check per adapter pass. Strict core/GUI/TUI Clippy passes.
Local documentation links and `git diff --check` pass.

Full batch gate, release timing, peak/settled memory and sustained stress remain pending the next combined
batch. No per-ticket benchmark or stress run was performed. Native macOS/Windows, native OS input delivery,
physical display latency and long sessions remain unverified. The existing diff engine has no internal
cancellation checkpoint: cancellation is checked around its computation, and running work retains its bounded
executor credit until it returns. This report makes no responsiveness claim for a pathological full-file diff.
