# Integrated editor: implementation handoff

Written at the pause of 2026-10-24, after ticket 30 merged. Rewrite or replace this
file at each pause rather than appending to it; the durable record of every merge and
decision belongs in the local tracker (see [Read instead of reconstructing](#read-instead-of-reconstructing)),
not here.

## State

- The work is **paused**. Do not start tickets until the user says to resume.
- Integration checkout: the worktree on `integration/integrated-editor` (`git worktree
  list` shows where), clean and pushed to origin; no PR. Leave the primary checkout (the
  repository's main worktree) untouched.
- Since ticket 30 the branch also carries the dev build profile and the end-of-epic gate
  fixes listed under [Known issues to carry](#known-issues-to-carry).
- **36 of 68 tickets resolved.** No agents running; every worktree idle and clean.
- Ready frontier when resumed: 33, 36, 39, 40, 41, 42, 43, 55, 56, 67. Ticket 03 is
  `needs-info` (no macOS runner; do not ask again), which blocks 24 and therefore 68.
- Critical path for the remaining language work: 39 (unblocked by 30) and 40/41/42/43,
  which together unblock roughly twenty language and SQL tickets.

## Read instead of reconstructing

The tracker lives under `.scratch/integrated-editor/` in the integration checkout. It is
git-ignored on purpose: never force-add it, never copy it into a worktree, and give
sub-agents absolute paths into the integration checkout instead of pasting ticket text.

1. `.scratch/integrated-editor/implementation-state.json` — authoritative:
   `resolved`, `worktrees`, `verification_cadence`, `concurrency`, `build_cache`,
   `known_flaky`, `pause_after_current_wip`, past `performance_decisions`.
2. `.scratch/integrated-editor/current-checkpoint.md` — every merge, decision and pause,
   latest at the bottom.
3. `.scratch/integrated-editor/spec.md`, `map.md`, `issues/NN-*.md` — the spec and the
   tickets; conventions in [issue tracker](issue-tracker.md).
4. [AGENTS.md](../../AGENTS.md), [terminal guide](../tui.md),
   [domain rules](domain.md) — GUI/TUI parity, robustness and term rules.
5. [Editor follow-ups](../editor-follow-ups.md) — deferred items and suspected costs,
   one section per recent ticket (25–32, 34, 35, 60, 65, 66).
6. Per-topic research notes: [snippets](../research/reader-snippets.md),
   [completion](../research/reader-completion.md),
   [saved comparison](../research/reader-saved-comparison.md),
   [Unicode composition](../research/reader-unicode-composition.md).
7. `git log --first-parent integration/integrated-editor` for what each ticket merged.

## User decisions in force

- **One ticket at a time.** Batches caused repeated conflict rework: ticket 26 rewrote
  command dispatch and collided with 29, 32, 60 and 28 in turn.
- **Tightened per-ticket gate:** filtered tests for the behaviour touched in each crate,
  `cargo clippy` on the touched crates, and `python3 scripts/panic-sites.py --reader`
  plus `--reader-tui`. The full `reader-core`, `reader-gui`, `reader-tui` and
  `lenz --features gui-test gui_` sets run only at the end-of-epic gate. The accepted
  risk is that adapter regressions surface late, not per ticket.
- **No performance measurement or stress until the end of the epic**: one campaign, then
  one explicit cost decision covering every new cost, as [AGENTS.md](../../AGENTS.md)
  requires. Implementers ignore the tickets' "performance checks" clause and record
  suspected costs in [editor follow-ups](../editor-follow-ups.md).
- Never run `cargo fmt` or rustfmt. Commits end with only
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` — no session link, no
  "Generated with" line.
- `RUSTC_WRAPPER=sccache` per implementer command only, never in a global cargo config;
  final release and benchmark builds must unset it. The evidence so far is 22 hits out of
  40 executed requests, with most builds reusing warm per-worktree `target/` directories,
  so the benefit is unproven — do not claim it helps.
- Build settings follow [AGENTS.md](../../AGENTS.md#build-output). The
  `DEBUG0/INCREMENTAL0/JOBS4` environment in older checkpoints is retired.
- Sub-agents may install missing language tools from official sources for the language
  tickets.
- When the user asks to pause after the current ticket: finish and merge it, start
  nothing new, record the pause in the tracker, and rewrite this file.

## Workflow that worked

- Claim the ticket (`**Status:** claimed`), record it under `worktrees` in the state
  JSON, then `git switch -c work/editor-NN-<slug> integration/integrated-editor` in an
  idle worktree: those `git worktree list` shows on merged `work/editor-*` branches are
  the reusable ones (merged worktrees with no further use were removed).
- Launch one background implementer with a pointer-only prompt: the ticket path, the
  spec, [AGENTS.md](../../AGENTS.md), the policy block above, "call the Skill tool with
  `tdd`", the filtered gate, "the integration root is read-only for you" (two agents
  edited it and had to revert), and "merge `integration/integrated-editor` before
  reporting".
- Merge with `git merge --no-ff <sha>`, then run both panic-site scripts before
  resolving. On a conflict, `git merge --abort` and send the implementer back to merge
  the tip and resolve semantically; that is what cleared tickets 29 and 32.
- Watch these semantic-conflict points on every merge: the `CommandId::ALL` count and
  the `an_empty_session_offers_open_and_explains_the_rest` expected label list (165 at
  this pause), the keymap preset tables, and the menu and shortcut tests.
- Resolve with `Status: resolved`, ticked boxes, an `## Answer` carrying the merge SHA
  and the evidence pointer, the `resolved` list in the state JSON, and a checkpoint line.

## Known issues to carry

- Six tests time out at random, so roughly one full gate in three fails: the list, rates
  and repro are in [editor follow-ups](../editor-follow-ups.md#tests-that-time-out-under-load-2026-10-24).
  Two of the same class were product bugs, now fixed (`42e7fc7` rename identity,
  `9c4e9de` prompts settled during drawing): trace one before raising a timeout, but not
  during ticket work. Also seen before: reader-tui
  `double_and_triple_click_select_word_and_group`, and `semantics -- completion` while
  several suites ran at once.
- The filtered per-ticket gate let three TUI expectations go stale through tickets 66
  (gutter marker cells) and 27 (paired delimiters); the full gate caught them (`a6a63b8`).
- A panicked Git status read can leave `git_status_running` stuck (ticket 65).
- Some ticket 31 branch merge commits lack the trailer; left alone, no history rewrite.
- The installed rust-analyzer is not a provider-tested version, so the `real_rust`
  opt-in provider test fails environmentally.
- User-visible key changes the user has not objected to: `Alt+Shift+Up/Down` duplicates
  and Add Cursor moved to `Ctrl+Alt+Up/Down` in both presets, Tab inserts the resolved
  indent width, and Toggle Comment is `Ctrl+/` and `Alt+/`.
- At the end of the epic: the `code-review` skill on the integration branch, one fix
  agent, the full gate, one performance campaign with a single cost decision, then
  ticket 68. Native macOS and Wayland GUI editing remain unverified and gated.

## Suggested skills

- `implement-spec` — when the user resumes, continuing the task graph under the
  constraints above.
- `tdd` — implementers call it per ticket.
- `ponytail:ponytail` (full) — the minimal-change style the user has active.
- `superpowers:verification-before-completion` — before claiming a merge or resolution.
- `code-review` — only once every ticket is done.
- `resolving-merge-conflicts` — only when a real conflict must be resolved on
  integration.
- `handoff` — at the next pause, to rewrite this file.
