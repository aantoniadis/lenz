# 34: Reconcile an external file change against a dirty draft

**What to build:** Compare concurrent disk and draft changes and deliberately take disk, keep the draft or resolve differences.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Open the comparison from the existing conflict state with clear disk/current/confirmed-baseline identities.
- [x] Taking disk and applying resolved content are explicit undoable lifecycle decisions; keeping the draft does not accept a new disk baseline silently.
- [x] Recheck target identity/content at Save and retain all data after another external change or uncertain write; both adapters offer equivalent choices.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 34.

## Answer

Merged in 94bd935 (branch work/editor-34-external-change, 1603088, from daefa89; clean). The existing conflict state gains `Command::ResolveExternalChange`/`ChooseReconcile` (ALL = 165) and a comparison that names Disk (read-only), Current draft and Confirmed baseline (`Draft::baseline`) with each side's lines/bytes and the disk↔baseline relation; only the draft side accepts edits. Taking the disk version is one undoable edit that advances the baseline; saving the draft over disk is an explicit choice that advances the baseline only on a confirmed write; Keep Draft (Esc) drops only the pending dialog and adopts nothing, so markers and Compare-saved still show the old baseline. Every decision re-reads the file off the UI thread and applies only on identical content plus device/inode, so a second external change refreshes the sides and requires a new decision, and an uncertain write keeps draft, history and baseline with Save blocked pending Recheck. Filtered core 115 + semantics 103, GUI 38, TUI 47, parity replay passing; clippy clean; panic-site ratchets ok. Limits: an observed conflict is required, Keep Draft leaves Save offered until the writer rejects, and long side labels truncate in narrow panes — see docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.
