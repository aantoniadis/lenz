# Integrated editor: implementation handoff

Written at the pause of 2026-10-24 for resuming in a Claude Code cloud session, after
ticket 30 merged and the end-of-epic gate fixes landed. Rewrite or replace this file at
each pause rather than appending to it; the durable record of every merge and decision
belongs in the tracker (see [The tracker](#the-tracker)), not here.

## State

- The work is **paused**. Do not start tickets until the user says to resume.
- Branch `integration/integrated-editor` on origin is the integration branch; no PR.
  Leave `main` alone.
- **36 of 68 tickets resolved:** 01, 02, 04–23, 25–32, 34, 35, 38, 60, 65, 66.
- Ready frontier: 33, 36, 39, 40, 41, 42, 43, 55, 56, 67. Ticket 03 is `needs-info` (no
  macOS runner; do not ask again), which blocks 24 and therefore 68.
- Critical path for the remaining language work: 39–43, which together unblock the
  language and SQL tickets 44–54, 57–59 and 61–64.

## The tracker

- `.scratch/integrated-editor/` is committed on this branch: `spec.md`, `map.md` and one
  `issues/NN-*.md` per ticket. [Tracker conventions](issue-tracker.md) define claim,
  frontier, resolve and the map's Decisions-so-far.
- Commit tracker changes together with the work they record, so the pushed branch is
  always the whole state.
- The repository is public: tickets, docs and commits carry no machine paths, hostnames,
  hardware details or session identifiers.
- The local state file and checkpoint history of the first machine were not published;
  this file carries what still matters from them.

## Cloud session setup

What a cloud session gets (Claude Code docs, 2026-10-24): roughly 4 vCPU, 16 GB RAM and a
30 GB disk on Ubuntu 24.04; the repository's `CLAUDE.md` (which imports
[AGENTS.md](../../AGENTS.md)) and `.claude/settings.json`; no user-level settings, memory,
skills or plugins from the first machine.

- **Environment setup script** (the user sets it in the cloud environment's settings; it
  runs as root): the gpui system packages from the README's Build section, and a stable
  Rust of at least 1.92 (the dependencies' highest `rust-version`; this branch was built
  with 1.99.1):

  ```bash
  apt-get update -qq
  apt-get install -y -qq pkg-config libxkbcommon-x11-dev libwayland-dev libfontconfig-dev \
      libx11-xcb-dev libvulkan1 libglib2.0-dev libzstd-dev
  if command -v rustup > /dev/null; then rustup update stable && rustup default stable
  else curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; fi
  ```

  Check `rustc -V` and the package names in the first session; neither was verified on
  the cloud image.
- **Attribution:** the committed `.claude/settings.json` hides the session-link trailer
  and the pull request attribution line. Check the first cloud commit ends with only the
  `Co-Authored-By` trailer.
- **One checkout, one target:** a full `scripts/ci-local.sh` cycle writes about 16 GB
  uncompressed (dev about 12 GB, release about 4 GB), so a second worktree with its own
  target does not fit. Run `cargo clean` before the end-of-epic gate if space is short.
- **Long commands:** foreground commands stop after 10 minutes; run cold builds and gates
  in the background. Expect builds several times slower than on the 32-thread machine the
  earlier timings came from.
- **Pre-push gate:** a fresh clone has no `core.hooksPath`, so pushes skip
  `scripts/ci-local.sh`. Enabling it (`git config core.hooksPath .githooks`) makes every
  push run the full gate; ask the user before enabling it or before pushing with
  `--no-verify`.

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
- Build settings follow [AGENTS.md](../../AGENTS.md#build-output).
- Agents may install missing language tools from official sources for the language
  tickets. The cloud's default network allows GitHub, crates.io, the Rust and Ubuntu
  archives; a tool hosted elsewhere needs the user to widen the environment's network
  access.
- When the user asks to pause after the current ticket: finish and merge it, start
  nothing new, record the pause in `map.md`, and rewrite this file.

## Workflow

- Claim the ticket (`**Status:** claimed`), commit, then
  `git switch -c work/editor-NN-<slug> integration/integrated-editor` in the checkout.
- Implement test-first (the `tdd` skill where available), then run the filtered gate.
- A background implementer subagent can do the work, with a pointer-only prompt: the
  ticket path, the spec, [AGENTS.md](../../AGENTS.md), the policy block above, the
  filtered gate, and "merge `integration/integrated-editor` before reporting". It shares
  the one checkout and target, so run one at a time and leave the checkout to it while
  it runs.
- Merge with `git switch integration/integrated-editor && git merge --no-ff <branch>`,
  then run both panic-site scripts. On a conflict, `git merge --abort`, merge the tip into
  the work branch and resolve semantically; that is what cleared tickets 29 and 32.
- Watch these semantic-conflict points on every merge: the `CommandId::ALL` count and
  the `an_empty_session_offers_open_and_explains_the_rest` expected label list (165 at
  this pause), the keymap preset tables, and the menu and shortcut tests.
- Resolve with `Status: resolved`, ticked boxes, an `## Answer` carrying the merge SHA
  and the evidence, and a Decisions-so-far line in `map.md`; commit and push
  `integration/integrated-editor`.

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
- The `real_rust` opt-in provider test needs a provider-tested rust-analyzer; with
  another version it fails environmentally.
- User-visible key changes the user has not objected to: `Alt+Shift+Up/Down` duplicates
  and Add Cursor moved to `Ctrl+Alt+Up/Down` in both presets, Tab inserts the resolved
  indent width, and Toggle Comment is `Ctrl+/` and `Alt+/`.
- At the end of the epic: the `code-review` skill on the integration branch, one fix
  agent, the full gate, one performance campaign with a single cost decision, then
  ticket 68. Native macOS and Wayland GUI editing remain unverified and gated.

## Suggested skills

A cloud session loads the repository's skills and those enabled on the user's claude.ai
account, not the first machine's. The workflow above does not depend on any of these.

- `tdd` — per ticket.
- `superpowers:verification-before-completion` — before claiming a merge or resolution.
- `code-review` — only once every ticket is done.
- `handoff` — at the next pause, to rewrite this file.
