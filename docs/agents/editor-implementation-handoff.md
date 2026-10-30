# Integrated editor: implementation handoff

Updated 2026-10-27 after completed Linux/shared measurements, before the cost decision. Replace this file at the next pause; durable history and decisions belong in the tracker.

## Current state

- The product is now **Lenz Code**, at the user's direction. Cargo executables are `lenz-code` and `lenz-code-tui`; `lenz` and `lenz --tui` remain the main entry points. Packages are `code-core`, `code-gui` and `code-tui`; settings, state and cache live in `lenz-code` folders, moved from `lenz-reader` on first launch. Historical measurement reports retain the names of their captured builds.

- Work belongs on `integration/integrated-editor`; leave `main` and release tags alone. Ticket 58's merged tip `dc37c30` is included, together with tickets 61–64 and 67.
- **65 of 68 tickets resolved:** 01, 02, 04–23 and 25–67. Every non-native feature ticket is resolved. Ticket 03 is `needs-info` because no native macOS runner is available; the user asked not to be asked for one again. It blocks 24 and full release ticket 68. Preserve the macOS/Wayland editing gates.
- Ticket 68's Linux/shared review and campaign use final Reader runtime `ff217fc8f7b1dafbffd1f4abe2ac8456393ea60b` with fixture-only measurement build `d6fd35b8d2d99c88ffc515eeb188d4f9fb4d4380`; canonical Diff/core/TUI source is `b072d0ea8ac141b260aa37eda5e5266018d663db`. The [Linux validation report](../research/code-integrated-editor-linux-validation.md) is the authority for final results, costs, evidence boundaries and unverified cases. Do not turn its Linux result into full native release acceptance.
- The fresh complete `scripts/ci-local.sh` invocation passed: strict workspace/default+adapter lint, three panic-site ratchets, workspace/integration/doc and six serial memory checks, 261 Diff GUI, 199 Reader GUI, 201 Reader adapter parity and 75 collector tests (74 passes, one platform skip), standalone terminal releases and stripped Diff size 68,422,440 bytes.
- Available live provider cohort: 62 checks, including 13 actual GUI/TUI provider replays and real ERB/Haml compilers; corrected Ruby completion and the fixture-only Ruff cancellation assertion each passed five additional real serial runs. Four fresh Kotlin opt-ins were unavailable because the official download endpoint was blocked; preserve ticket 45's older evidence separately.
- Native Linux X11 default-input close veto, modal precedence, two consecutive preedits, exact Save and normal exit passed. Default ordinary TUI Unicode/paste/Copy/Save, dirty quit and terminal restoration passed. Twelve SQL native/boundary/adapter cases passed with unchanged PostgreSQL rows and DuckDB file bytes; the disposable PostgreSQL server was stopped.
- Completed Linux measurements: canonical 133 cases/391 comparable metrics; final Reader startup/GUI/PTY/core/shared-history plus full 900-second reading and integrated editing stress; template/SQL costs, optional Tokio supplement, formatter traces, alternating follow-ups and final artifact checks. Details, measured costs and limits are in the report. Ordinary GUI editing repeated settled RSS medians are 162.69→207.71 MiB; no tradeoff is silently accepted.
- The single cost decision remains pending. The user directed integration merge `422cb9e5337f88750f78f6c22186988da4b0b645`, pushed with progress documentation at `d3dffc5`. The merged code tree matches the validated branch and preserves ticket 58. This direction does not invent unmeasured results or remove full native release dependencies/platform gates.

## Repairs and remaining diagnostics

The [Lenz Code rename verification](../research/lenz-code-rename.md) records the naming surfaces, stable data namespaces, new executable targets and Linux checks. The first parallel core refresh test and terminal draft-name navigation test timed out; unchanged focused repeats, a fresh full default core batch and a serial full terminal/parity batch passed. Original failures remain retained and unisolated; do not infer a fix or parallel reliability from those passes.

The absolute performance gate remains failed: view build was 4.61 ms against 2.4 ms (baseline also fails at 4.83 ms), and one regex-search attempt was 25.213 ms against 25 ms. Five alternating regex pairs measured 20.618→20.054 ms medians. Keep the original failures and unchanged budgets. One candidate GUI repetition timed out waiting for final restore-Undo paint feedback; its unchanged repeat and the remaining cases passed, but its cause was not isolated. Trace recurrence rather than claiming a fix or increasing the deadline.

The review fixed diagnostic-cache address reuse/stale publication, obsolete repository/root lookup replies, Git-status single-flight release after panic, recovery discard/Save ordering, inactive marker baseline retention, unchanged-source marker materialization, file-watch registration gaps and final-owner server cleanup. Home/End caret and modified document bounds now share core motions across both adapters. Exact tests and source commits are in the report; no timeout was raised.

Worker/request/render barriers repaired old fixture races without weakening pending-write, stale-request, recovery or Undo assertions. Followed rename and Save As each passed 128 runs at concurrency eight; recovery restart passed 128 and SQL settings 100 serial runs. Late completion passed 80 repeats. Some older GUI outline/semantic request and Diff Markdown timeouts were not reproduced in targeted repeats; their original causes remain unproven. See [editor follow-ups](../editor-follow-ups.md), and trace any recurrence before increasing a timeout.

The final real JDTLS runs do not need observer cleanup of their language-server process. Two Gradle scenarios still leave a separate Gradle daemon, captured and cleaned by the fixture. Test/child RSS sampling is coarse and can miss short children; do not treat unobserved peaks as true zero or test wall time as production action latency. Physical display/terminal latency, hardware GPU memory and native macOS/Wayland remain unverified.

Provider/template limits stay in [the capability guide](../code/providers.md). Fresh Ruby fixtures need installed compiler/formatter gems and a conventional RubyGems path when checking installed-gem origin. JDTLS can select a decompiled library declaration without a JDK source archive. Source/version aliases must be verified; rust-analyzer 2026-08-31 reports 0.3.3033 from the same tested source.

## Decisions still in force

- Work one feature ticket at a time. Claim its Markdown file and commit before implementation; preserve other agents' claims.
- The end-of-epic policy was one release campaign and one explicit cost decision. Its decision remains pending, recorded in the report and tracker; correctness or a loose performance ceiling alone does not accept a deliberate regression. Do not silently rerun per-ticket performance campaigns.
- Per-ticket checks cover touched behavior/crates, strict Clippy and Reader/Reader-TUI panic-site ratchets. The full regression gate runs at the end of an implementation batch. New failures or changed runtime behavior justify relevant verification again.
- Never run `cargo fmt` or rustfmt. Use truthful author/coauthor identity; do not manufacture another agent's attribution or rewrite old commits to add a trailer.
- Build with checked-in profiles and separate worktree targets. Keep immutable measured executables, hashes, raw samples, failed attempts and actual stress durations outside disposable `target/`. The retained local campaign is under ignored `perf-results/editor68/`; this path is not a public download.
- Use owned displays, fixtures, PTYs and servers. Do not stress the user's desktop/repositories. Public docs carry no machine paths, hostnames, hardware descriptions or session identifiers.
- Do not change hook configuration or bypass hooks without the user's authorization. A full gate was run manually in the current checkout; a missing `core.hooksPath` is not evidence that the gate ran.
- An undecided recovered draft blocks edits of its file (33); recoverable deletion uses in-house freedesktop trash (37); server commands require the per-provider allowlist (45).
- Python Organize Imports stays Ruff import sorting (`--select I`); broad Fix All/removing unused imports is a separate feature. Rust `checkOnSave: false` remains the current choice; enabling builds after every Save needs its own decision.

## Resuming work

Read [AGENTS.md](../../AGENTS.md), [tracker conventions](issue-tracker.md), [triage labels](triage-labels.md), the spec/map and the relevant ticket. The [tracker](../../.scratch/integrated-editor/map.md) records dependencies and outcomes. Merge semantically with integration, verify the runtime tree and relevant ratchets, commit evidence with its work, and push only the integration branch without force. Resolve full ticket 68 only after its native dependencies and cost requirements actually pass. No code-review skill was available in this session; the Linux review and deterministic regression work were performed directly.
