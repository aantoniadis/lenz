# Explicit external opens for dirty files

Ticket 19 adds shared **Save and Open**, **Open Saved Version** and **Cancel**
decisions before either adapter hands a dirty file to an editor or default app.
Clean files keep the existing platform launch policy. Production text input
remains gated until the foundation safety checkpoint.

## Behavior and ownership

`Session::external_command` resolves the existing platform/editor configuration
into an opaque command. Dispatch revalidates the active tab, source key and
metadata, path, project generation and edit permission. The pending decision
retains plans and weak identity, not another text snapshot. Choice tickets are
single-use; replaced, cancelled or obsolete choices produce no launch.

Open Saved Version retains the draft and history and omits dirty caret
coordinates. Save and Open uses the existing bounded Save lane and launches only
after the actual accounted Written receipt matches that request and captured
source. It does not infer success from a busy status becoming Ready: post-save
compaction has a separate purpose. New edits, permission changes, file/context
switches and close preparation retire the continuation. An already requested
Save can finish after Cancel; it cannot resume the external launch.

Rejected or uncertain Save drops the old continuation and retains draft/history
and the confirmed baseline. An uncertain disk outcome requires a current Recheck
before a fresh saved-version decision; opening those bytes does not confirm a
new baseline. Another Save retains the existing explicit confirmation rules.
Close preparation cancels competing external consent even when the all-session
coordinator invokes it directly, before another dispatch.

Write accounting marks the existing pending request ready without consuming its
plan. The normal GUI command/completion handler and TUI command/completion handlers
explicitly issue `TakeExternal` with that ticket. The core revalidates the current
context and saved-version permission or exact Written receipt, removes the request,
then returns one optional `Effect::external`. Render housekeeping can therefore
account a completed Save without losing delivery. Cancel remains available until
the take, and repeated, premature or wrong-ticket takes cannot launch a second
time. The take is internal delivery acknowledgement and preserves edit grouping
and pending layout intentions. Their existing wrapped Details presentation displays
the same choices and disabled reasons; S/O/Escape and pointer controls precede
underlying typing, paste, Undo and Save. The raw `editor_plan` API refuses dirty
files so existing callers cannot bypass the decision.

External comparison menu actions, chooser captions and shared notices identify
**saved disk inputs**, distinguishing them from the in-window saved comparison.
Launch arguments and real file paths stay unchanged. In particular, a synthetic
Diff `--name` is not used to label the inputs: that option also affects displayed
paths and copy/navigation semantics. Drafts remain owned by Reader.

## Focused verification

The implementation follows the approved Session-command boundary and real
adapter input/render replay:

- Seven core external-open checks cover all choices, exactly-once confirmed Save,
  real rejected/uncertain writes and recheck, delayed Save with Cancel/edit/file
  switch, permission/close invalidation, history and exact Unicode/CRLF bytes.
  The delivery protocol also checks readiness across housekeeping, premature and
  wrong-ticket takes, and Cancel/edit/permission/file changes before delivery.
- Three actual GUI and three standalone TUI traces drive the command palette,
  clipboard/bracketed paste and decision controls. They cover both appearances,
  narrow/tiny resizing and scrolling, modal isolation, saved-version coordinates,
  confirmed Save, Undo, failure presentation and stale choices. Test seams record
  external launches instead of starting applications on the user's desktop.
  A controlled completed Save is accounted by direct GUI render housekeeping or
  actual TUI drawing before normal wake delivery; both then launch exactly once,
  or zero times if Escape cancels the ready request first.
- The existing clean editor-plan, catalogue, shared permission and comparison
  authority checks pass, as do 26 affected close checks. Strict core/GUI/TUI lint
  and ordinary production binary checks pass on the merged foundation.

Observed red/green cases include the missing adapter decision, missing saved-disk
labels, permission changes retaining old consent, and coordinator preparation
retaining a competing external decision. A later caller audit found that direct
render dispatches ignored effects, consuming an external launch before its normal
handler. The actual GUI and TUI regressions failed with zero launches or missing
repaint, then passed with explicit ready/take delivery. Auxiliary failures were fixture-only:
the system opener may add arguments before the file path, and the TUI's existing
`#[path]` test module needs an explicit child path. Failed logs are retained, including two incorrectly filtered zero-test runs;
only executed test cases count as verification.

Initial runtime implementation checkpoint: `259869c`; the follow-up delivery fix
is identified separately by source/executable hashes in the evidence directory. Raw commands, logs, source file and
test-executable identities are under
`target/perf/editor-implementation/19-external-open/`. The merge preserves tickets
14/15/17/20/21; focused checks replace per-ticket full-suite and stress runs.

Final batch correctness/performance acceptance, real external-application startup,
native macOS, production input/IME and long-session behavior remain unverified.
No new performance tradeoff is accepted by these checks. The combined foundation
campaign runs after the remaining cohort implementations are ready.
