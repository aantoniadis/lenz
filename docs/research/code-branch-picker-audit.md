# Code branch picker and workflow audit — 2026-11-10

CI and native validation pass on `fix/code-branch-picker-and-audit`; the measured
performance tradeoff awaits user acceptance. Baseline source:
`13af4f5` (0.0.14 plus release documentation/checksum metadata). Raw native input,
screens, identities, failed attempts, samples and process snapshots are retained
in `perf-results/app-audit/`, outside disposable Cargo output.

## Findings and changes

Code's branch picker discarded its inventory on every dismissal and waited for
changed/untracked path counts in every worktree before displaying branches.
It now reads navigation metadata without those per-worktree status scans,
retains one repository's inventory across dismissal, and refreshes on reopening
while cached choices stay available. Closing or changing repositories supersedes
queued work and rejects obsolete results. Comparison pickers do not overwrite
the full inventory cache; refresh errors retain usable choices and show a reason.
Cancellation skips queued work and later phases/results; it does not interrupt
a Git process already executing. The cache retains one repository's complete
metadata inventory, with a second copy while the popup is open, rather than a
fixed cap on reference count. It contains no document contents or per-file status.

Both adapters display separate, nonselectable Recent branches, Local branches,
Remote branches, Tags and Worktrees headings. The GUI's verbose footer could
consume all result space at 260×200; the compact footer reserves a choice row.
Page navigation, filtering, End then Up and tiny layouts have explicit checks.
The TUI now displays loading/failure/no-match messages instead of losing them
when replacing its title. Its Right key opens branch actions at the filter's end
and still edits a filter when its caret is inside the text.

Cached refresh introduces a selection hazard if new refs shift row numbers.
Both adapters preserve section/target identity and reject pointer hits from an
obsolete list until repaint. Tests cover insertion, deletion, filter edits and
native GUI text input before repaint. This is a safeguard for the new cache,
not a claim that 0.0.14 already used cached picker refresh.

The bounded workflow audit also found:

- Creating or reopening a Code TUI scratch file from project-search Results
  leaves keyboard focus there, swallowing immediate typing. The native baseline
  reproducer saves an empty file. Scratch selection now returns focus to source
  input in both adapters.
- Editable scratch/external/dependency sources say “Read-only source from …”.
  Native typing and Save prove that the notice is false. Origin text is now
  neutral; actual read-only reasons remain separate and edits remain refused
  when Read-only Mode is enabled.
- The GUI Scratch Files picker has no filter, although the terminal picker does.
  It now supports the same filename filtering, field editing, empty results and
  correct filtered-path activation.
- A manually named `scratch_18446744073709551615.*` makes scratch-number
  increment overflow. Creation now reports exhausted numbering without changing
  files, the current tab or the draft. Boundary and exact-byte regressions cover it.

Further deterministic checks confirm a separate shared-core Save race. A disk
check begun against the previous confirmed baseline can arrive after an own Save
and mark that write external; an old `Same` reply can also restore its old stamp.
Refresh replies now carry confirmed-baseline identity. A superseded observation
triggers a fresh check against the current baseline, including when a real
external write followed Save before the old reply arrived. Move replies are
unchanged and ordinary Save adds no unconditional disk I/O. Four regressions
pass after the failing pre-fix proof. This defect is independently confirmed;
the direct cause of the earlier intermittent Vim parity failure is unconfirmed.

### Code native Quit and window close

The user subsequently clarified that the macOS Quit/close report concerns Code:
Cmd+Q does nothing, and the red close button can leave the window open without a
question. Source review found no native Code Quit action/menu binding, and an
open transient modal vetoed native window close before the shared consent flow
could run. Code now installs a macOS Quit action/menu and Cmd+Q binding; the raw
platform-key route also works before a transient field consumes input. Native
close dismisses transient pickers through their adapter cleanup, then requests
the shared single-window or all-window close protocol. Existing unsaved-draft,
terminal, recovery, external-change, reconciliation, replacement and checkout
decisions remain visible and retain their consent instead of being dismissed.

Preparation can also create a terminal shutdown question when there are no
dirty buffers. The close coordinator's GUI pump previously synchronized only
the unsaved-draft question, so the terminal question could remain invisible
until another dispatch, including the five-second timer tick. The pump now
synchronizes all core prompts immediately.

The new terminal-close regression exposed a further feedback loop during this
fix. An unconditional notification in the pending close pump repainted an
unchanged question; rendering requested the still-unacknowledged terminal
resize again, dispatch scheduled close progress, and deferred pump/render work
kept running without yielding to the frame acknowledgement. The minimal fix in
`fa0768650e59938c4ae0783e7537a1acbbac3a10` notifies only when the synchronized
prompt's kind, title or text changes. Terminal resize state and ownership rules
are unchanged. The first run was interrupted after the third test stalled;
`native-close-focused-1.log` and its trace are retained. The corrected focused
run passes all **three GUI tests in 1.52 seconds** (`native-close-focused-2.log`):
native action and raw Cmd+Q use all-window consent, red close dismisses a picker
while preserving the dirty-file decision, and an idle terminal question appears
before polling or a timer tick, supports cancellation, and closes only after
confirmed terminal cleanup.

Shared teardown also omitted cancellation of an active Git-status request,
although its streaming reader already supports cancellation and reaps its child.
`Session::drop` now invalidates that request before joining workers. A bounded
lifecycle test starts an active worker, drops its session owner, then verifies
the real status reader returns `superseded` after killing and waiting for Git.
It passes in **0.01 seconds** in `native-close-focused-1.log`; a three-second
fallback prevents missing cancellation from hanging the test indefinitely. Cancellation
is checked between status entries; this does not promise to interrupt Git before
its first output or an uninterruptible filesystem call.

These are source-backed fixes and Linux GPUI/shared-core regression results.
Native AppKit menu delivery and actual macOS window/terminal shutdown remain
unverified. The complete combined-tree gate subsequently passes on `d7b5566`;
the native-helper compilation guards cover plain builds without test support.

## Audit coverage

Owned disposable projects, XDG state, Xvfb displays and PTYs keep the user's
desktop and repositories out of the workload. Baseline functional checks pass
Code GUI editing/Save/Undo/Redo on 100,000 Unicode/CRLF lines; Code TUI project
search, replacement preview/cancellation and tiny resize; Diff GUI comparison,
external-write refusal/recovery, merge and modal capture at 900×650, 640×480 and
400×240; and Diff TUI input, resize, merge/cancellation and terminal restoration
on normal, error and Ctrl-C exits. These ran during compilation and are functional
evidence, not accepted performance timings.

The initial GUI native-flow invocation passed an empty Xvfb argument; its corrected
owned-display run passes. The old native TUI comparison collector waits for
`Find:`/`Find: needle`, while the editable popup now paints its counter before the
colon. That collector stops early. Its merge case and the independent PTY checks
pass, but the complete native TUI comparison campaign is not marked passed.
Failures and exact coverage are indexed by `audit-checklist.md`.

Final production replay on `d7b5566` passes the scratch-focus Unicode Save
regression and all four grouped-picker cases (GUI/TUI × dark/light), including
filtering, actions/back, clearing, narrow/zero resizing and unchanged refs/files.
Four Linux X11 `WM_DELETE_WINDOW` scenarios pass: dirty file with a transient
picker; clean idle terminal; and dirty file plus terminal using Discard or Save.
Cancel preserves the window, live shell and file bytes; final approval collects
both required decisions, preserves the expected saved/discarded bytes and stops
the shell. These use X11 window-close transport, not AppKit. Diff GUI comparison
and merge, Diff TUI merge and basic PTY/restoration checks also pass. Evidence and
binary/source hashes are in `native-final-1/`; representative final screenshots
were inspected directly in both appearances. The first identity collector attempt
encountered a directory ending in `.rs` before launching apps; filtering regular
files corrects that collector failure, whose log remains retained.

### Retained validation attempts

All attempts remain under the same evidence directory. Earlier passing runs do
not substitute for a gate on the final combined source.

| Attempt | Result and disposition |
| --- | --- |
| CI1–3 | Corrected TUI API/ownership mistakes, obsolete inline-section expectations and pre-paint test setup. Also fixed the actual narrow GUI list/reveal defect and stale-hit handling. |
| CI4 | The existing tree-error fixture did not await its independent Git-status prerequisite. Its original failure did not identify which assertion conjunct failed. Preserve all assertions and wait for the required initial status; five focused repetitions pass. |
| CI5 | The terminal-selection fixture treated the initial blank frame as shell readiness. An independent shell probe reproduced prompt text moving immediate output away from column zero. Await size acknowledgement and print an explicit carriage return; exact selection/copy and disk assertions remain. Five focused repetitions and the full GUI suite pass. |
| CI6 | An intermittent Vim parity close mismatch remains unexplained. The preserved original executable passes three full-suite replays, and the current tree passes the full suite and twenty exact repetitions. Timeout diagnostics remain. The own-Save race above was established independently with a failing real-writer test. |
| CI7 | Complete gate passes before the native Quit/close follow-up: 2,979 workspace/serial and 938 adapter results across overlapping invocations; 86 Python tests, one skip. |
| CI8 | Plain workspace Clippy caught unused native test helpers without `test-support`. Narrow the compilation guards to match the actual test module, without suppressing the lint. |
| CI9 | An existing TUI terminal-search test times out after 252 passing TUI tests. Fifty exact diagnostic repetitions pass, so its original stage/cause remains unconfirmed. Source review establishes that the old readiness predicate can accept command echo before later output invalidates a search. The fixture now disables the prompt/echo and awaits escaped, actual output plus a final marker. All original safety assertions and the ten-second bound remain; twenty focused repetitions, the full standalone TUI suite and strict Clippy pass. Stage-specific diagnostics remain for recurrence. |
| CI10 | The existing selected-Node installer test receives an unexpected error during its wrong-version check, without reporting that error. Add diagnostic error/output text while retaining the exact expected rejection. One hundred focused repetitions pass; the original cause remains unconfirmed. |
| CI11 | Complete final combined-tree gate passes, including both formerly failing tests and all three new native-close regressions. |

## Validation and measurements

The complete local gate passes on
`d7b5566316d4dbe63971bdd59371a5f662ee978e`: **2,980 workspace/serial results and
941 adapter results**, across overlapping invocations, with no failures;
**86 Python tests, one platform skip**. Strict Clippy, panic-site budgets,
standalone terminal release builds and the Diff stripped-size check pass.
CI11 takes **267.986 seconds**, collecting **820 orphans without forced cleanup**.
The full log, source, exit status and summary are `ci-local-11.*`.

The final production Code GUI build and separate standalone Code TUI build also
pass using normal repository `release` profiles, without test or benchmark
features (`final-code-{gui,tui}-build.*`). Candidate native replays and the paired
correctness workloads pass as recorded below; performance acceptance is still
pending. Baseline source is
`13af4f5ea732fe2fbb5ef245fe66b04ad7446144`; its GUI and TUI were built separately
in an isolated checkout with its own target. Executable copies and SHA-256
identities remain in `baseline-bin/` and `baseline-executables.json` after cleaning
that target.

### Initial release measurements

Three alternating editor pairs and three further pairs starting in the opposite
order use the same two frozen binaries, 100,000 Unicode/CRLF lines, 30 ordinary
rounds and at least 20 seconds of sustained work in every process. All 24 GUI/TUI
processes pass correctness checks. Actual GUI stress lasts 20.068–20.514 seconds;
TUI stress lasts 20.010–20.066 seconds, 245 cycles per process. The first and
second campaigns take 408.005 and 405.797 seconds. Raw data, collector snapshots,
source/executable identities and exact commands are `editor-pairs-final-1/` and
`editor-pairs-reversed-1/`; scalar reports are `performance-*-scalars.*` and
`performance-six-pair-comparison.*`.

| Endpoint, six pairs | Baseline → `d7b5566` | Interpretation |
| --- | --- | --- |
| GUI ordinary edit, median of process medians | 8.070 → 9.685 ms | Five of six paired medians are higher; paired median +0.646 ms, range −0.146 to +3.177 ms. Pooled p95 18.413 → 17.650 ms. First XDamage can include preceding Save/refresh feedback; it does not identify the edit's own paint. |
| GUI sustained edit | Mixed paired signs | Three of six higher; paired median −0.102 ms, range −5.427 to +2.163 ms. No sustained editing speedup claim. |
| TUI ordinary key feedback | 0.7384 → 0.7689 ms | Five of six higher; paired median +0.0377 ms, range −0.0881 to +0.0981 ms; pooled p95 0.9434 → 0.9671 ms. A small upward tendency remains under investigation. |
| TUI sustained key feedback | 0.8544 → 0.8881 ms | Five of six higher; paired median +0.0499 ms, range −0.0254 to +0.1040 ms; pooled p95 1.3047 → 1.3388 ms. |

The ordinary editor comparison does not establish a consistent memory reduction.
GUI RSS ranges overlap, and both TUI executables reproduce two residency modes.
TUI idle output/CPU are zero in all runs. Schema 4 records the whole process RSS
timeline without an exact stress-start timestamp, so its first/last-quarter
figures include startup/navigation/Reload; they are not exact stress-quarter
measurements or leak proof.

A separate three-pair branch workload uses 1,000 generated local refs, two linked
worktrees (three total), and 2,000 untracked files. It passes all six runs,
20.002–20.073 seconds each, preserving every tracked/untracked byte, ref and
worktree HEAD and restoring the PTY. Baseline cycles are 277/249/259; candidate
cycles are 377/342/371. Usable local choices improve from **20.635 to 1.407 ms**
(pooled p95 27.019 → 2.025 ms). First PTY bytes are **0.727 → 1.354 ms**: the old
first frame contains a loading placeholder, while the new first frame contains
the cached choices. These endpoints represent different amounts of presented
content. Remote-filter readiness is 10.413 → 10.306 ms, with mixed paired signs.
Settled RSS medians are 16.375 → 16.641 MiB with overlapping ranges; stress-quarter
growth is 0.129–0.262 MiB before and 0.152–0.221 MiB after. CPU per completed cycle
falls, while CPU/second rises because more cycles finish. This is finite
allocation/worker stress, not a leak proof (`branch-stress-final-1/`).

### Rejected compact-cache experiment

The optional cache reserves 168 bytes inline in every Session. The three-site
Box experiment (`69ad5d5`) reduces measured Session size from 16,312 to 16,152
bytes without changing State/Input/BranchesView size. Six branch regressions,
strict core Clippy and a normal standalone TUI release build pass. Three balanced
baseline/inline/boxed triples (nine processes) complete in 234.407 seconds, each
with 20.016–20.053 seconds of checked stress and 245 cycles.

It does not rescue key latency: ordinary medians are 0.7772 ms inline versus
0.7776 ms boxed, with mixed paired signs. Sustained feedback is 0.8907 versus
0.9180 ms, with the boxed version higher in all three direct comparisons.
Its lower RSS mode cannot be attributed to saving 160 bytes; both residency
modes already occur in earlier builds. Metadata hashing/`--version` ending at
12:10:22 overlaps the first baseline process's early ordinary phase, so that
startup/ordinary comparison is contaminated. The later inline/boxed comparisons
and all stress intervals begin after that activity. Their negative finding
remains usable; no clean baseline ordinary preservation claim uses the affected
trial (`tui-box-triples-1/`, `tui-box-scalars.*`).

The experiment is reverted in `cc97b20`. All tracked runtime files match the
CI11 `d7b5566` tree exactly. A fresh normal standalone TUI build takes 59.50
seconds; both final Code binaries are byte-for-byte identical to the CI11/native
executables (`reverted-cache-final-identity.json`). Query fast paths and
viewport-rendering drafts remain unapplied. The final candidate retains the
measured small feedback-cost tendency; no unproven optimization is shipped.

### GUI diagnostic and performance decision

A separate versioned diagnostic removes interleaved Saves from edit/Undo/Redo/Undo
cycles while retaining the same first-XDamage helpers and 100,000-line fixture.
All six processes pass, including separate outside-timing exact edit-Save and
Undo-Save checks; stress lasts 20.210–20.353 seconds. Intermediate buffer contents
have a weaker oracle than the full-Save campaign, which remains retained and
required. These workload versions must not be pooled.

The upward GUI edit tendency is not reproduced: ordinary feedback is
22.666 → 22.213 ms, with one higher and two lower paired medians; sustained
feedback is 21.853 → 21.849 ms, again with one higher and two lower pairs.
Settled RSS is 165.320 → 165.363 MiB with overlapping ranges. This supports a
Save-associated repaint confound hypothesis, without proving the cause of the
original samples or correcting them (`gui-nosave-pairs-1/`, `gui-nosave-scalars.*`).

The later TUI control cohort also has mixed ordinary inline-versus-baseline
differences; its uncontaminated sustained comparisons still have a small upward
tendency. Exact latency preservation is not established. The final candidate
therefore keeps the faster usable picker while reporting its larger first-frame
cost and roughly 0.04 ms TUI key-feedback tendency. The
[repository performance rule](../../AGENTS.md#preserve-performance) requires
explicit agreement for a deliberate tradeoff. PR #53 remains draft pending that
decision; no merge or release is part of this audit.

The first audit snapshot has 788 processes, including 768 pre-existing PID1
zombies. At the end of the first performance campaigns there are 776 processes
and the exact same 768 zombie PIDs across 52 snapshots. Supervised builds and
test batches produce no zombie accumulation or fork/thread creation failures.
Later repeated measurements retain the same zombie count.
After all investigations and baseline-worktree cleanup, the stopped count is
775 processes and the exact same 768 pre-existing zombies. Preserved baseline
executables and evidence remain outside disposable Cargo output.

Native macOS/Wayland interactions, physical display/trackpad behavior, live
language-server installation/provider sessions and indefinite leak freedom are
unverified. This is a bounded audit, not a guarantee that the app has no bugs.
Zero and extremely small terminal frames establish safe recovery and restoration,
not full popup usability; narrow footer hints can clip while list navigation
remains available.
