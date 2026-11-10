# Lenz 0.0.14 beta validation — 2026-11-10

The release source is `cc4411322c58756d5486e127ba9ef45af1772c16`.
Local validation passed. Native packages and publication are in progress in
[release run 42784617241](https://github.com/aantoniadis/lenz/actions/runs/42784617241).
Raw logs, samples, identities, failed attempts and process snapshots are retained
locally in `perf-results/release-0.0.14/`, outside disposable Cargo output.

## Configuration and identities

Linux x86_64, kernel 6.18.57, AMD EPYC 9V74, five visible logical CPUs with a
four-CPU quota, 18.9 GB reported RAM, Rust 1.99.1/LLVM 23.1.1. Automated GUI
checks use an owned private Xvfb display and Mesa lavapipe software Vulkan.
Builds use the repository's normal `dev`, `release` and shipping `dist` profiles,
without ad hoc Cargo profile or Rust compiler overrides. Standalone TUIs are
built separately without GUI features; production executables exclude probes
and `gui-test`.

The performance baseline is the retained combined candidate
`efe382d3662897bf260b9f795f985411d9fb9555`, already containing PRs 49–52 and the
0.0.14 version bump. It is **not a pristine 0.0.13 release package**. The candidate
adds the allocation fix `86c6106c1ce786fa2724fc7db06f75dd15d9d170`, collector
repairs `fd381e046da3ec1785b181a18763ab59aeee41e1`, and the test-only Linux
liveness fix in the release source. Source and executable hashes accompany
the retained normal-release binaries in `view-baseline/` and `candidate-bin/`.

## Automated gates

The final `scripts/ci-local.sh` passes on the release source:

| Check | Result |
| --- | --- |
| Workspace and serial memory invocations | 2,959 passed, 0 failed, 145 ignored |
| Dedicated frontend adapter invocations | 925 passed, 0 failed, 30 ignored |
| Python collector checks | 86 tests, one macOS-only skip |
| Strict Clippy, formatting and panic-site budgets | Passed |
| Stripped production Diff GUI | 68,916,816 bytes; below the 80 MB limit |
| Supervisor | Exit 0; 373.029 seconds; 818 adopted children reaped; no forced cleanup |

The Rust counts are **3,884 passing results across invocations**, including
dedicated adapter reruns, not a count of unique test names. Evidence:
`ci-local-4.log`, `ci-local-4.exit` and `ci-final-summary.json`.

The complete normal-release component gate passes (`perf-check-6.log`, exit 0).
The full suite uses workload version **7**, `--profile full --runs 3
--stress-seconds 60 --check --require-ui`: **133 cases, 394 metrics, no skipped
workloads**, source identity matches the clean release commit. Evidence:
`full-2/results.json`, `full-2/summary.md`, `full-2.exit`, `full-summary.json`.
Normal-release gate and full-suite results are separate from shipping-profile
package verification.

## Repeated performance and sustained work

Three alternating process pairs measured view construction at **3.962 → 2.157
ms**. Scroll/layout medians were 14.01 → 13.32 µs, with overlapping ranges.
The optimization avoids empty per-line inline caches, appends row runs directly
and reserves known uncollapsed row counts. Collapsed layouts keep their smaller
capacity. Refinement tests cover inline spans arriving and being removed.

Both executables were warmed before five alternating startup pairs. Six GUI
and native PTY interaction pairs include three additional reversed-order pairs
to investigate the initial Find-scroll variation. The following are medians of
per-process summaries, with identical fixtures and normal-release feature graphs:

| Endpoint | Baseline | Candidate |
| --- | ---: | ---: |
| Diff GUI first paint | 215 ms | 202 ms |
| Startup settled RSS | 210.1 MiB | 208.5 MiB |
| Startup peak RSS | 423.0 MiB | 420.2 MiB |
| Startup process CPU | 2.82 s | 2.75 s |
| GUI page-scroll first XDamage | 31.47 ms | 31.65 ms |
| GUI Find typing first XDamage | 49.70 ms | 49.80 ms |
| GUI Find-scroll first XDamage | 49.71 ms | 52.42 ms |
| GUI next-match first XDamage | 49.74 ms | 48.31 ms |
| Post-interaction settled RSS | 253.60 MiB | 242.15 MiB |
| Native TUI first paint | 4.272 ms | 3.786 ms |
| Native TUI first comparison content | 34.75 ms | 32.64 ms |
| Native TUI input first byte | 0.669 ms | 0.643 ms |
| Native TUI resize first byte | 0.462 ms | 0.614 ms |
| Native TUI settled RSS | 49.79 MiB | 49.13 MiB |
| Native TUI peak RSS | 57.16 MiB | 52.98 MiB |

Find-scroll timing is **inconclusive**, with paired differences from −2.755 to
+6.315 ms and overlapping baseline/candidate ranges of 48.35–54.42 and
48.72–54.67 ms. No Find-scroll improvement or universal frontend speedup is
claimed. Sub-millisecond TUI resize differences also vary in sign and overlap
(baseline 0.28–0.67 ms, candidate 0.27–0.72 ms). Idle GUI CPU in the final
startup second and native TUI idle output/CPU are zero in all paired samples.
RSS includes software-renderer residency; it is not a measurement of GPU
allocation or solely application heap. XDamage and PTY bytes measure first
feedback, not completed display presentation.

The candidate mixed model stress actually runs **60.027 seconds**, 284 cycles;
the baseline runs **60.005 seconds**, 282 cycles. Candidate sampled RSS settles
at 312.70 MiB with **0.0 MiB last-quarter growth**; baseline median/final RSS is
315.18/315.31 MiB. Bounded stability does not prove indefinite leak freedom.
The final production Code TUI replay also passes on 100,000 lines, 30 rounds
and **10.042 seconds / 123 stress cycles** (10 seconds requested; schema 4).
Actual process samples and checks are in `code-tui-final/samples.json`.

Evidence: `view-alternating-3.log`, `view-comparison-summary.json`,
`frontend-comparison.log`, `frontend-confirmation.log`, `frontend-summary.json`,
`candidate-executables.json`, `baseline-mixed-stress.log`,
`baseline-stress-summary.json`, `performance-review.json` and `full-2/`.

## Failures investigated and repaired

The first combined candidate passed local CI but failed the existing 100k-line
view-build ceiling: 3.93 ms against 2.4 ms, reproduced at 3.67–5.25 ms. Lazy
inline allocation alone still measured about 2.83–3.10 ms. Direct row appending
improved isolated runs, but the complete second gate remained at 2.51 ms.
Reserving known row counts completed the fix; a subsequent complete gate
measured 1.98 ms, and the final gate passes. Budgets were not raised.

The first full suite stopped after 26 cases at the obsolete `Log filter:` PTY
marker. The app separates the heading and editable field. The repaired collector
requires both the heading and visible editing controls, and cancellation waits
for both prompt and result panel dismissal. A corrected 2,000-line/11-round
native repository/conflict replay passes history, filtering, refresh, search,
blame, modals, tabs, About, cancellation and conflict return. Workload version 7
keeps this separate from version 6. Standalone Python entry points now collect
classes previously defined after `unittest.main()`.

A later CI run exposed a test-only Linux liveness race: successful `kill -0`
followed by ENOENT after reaping was interpreted as alive. Diagnostic logs
capture that exact sequence. One `/proc` observation now handles live, zombie
and reaped states, including command names containing spaces. A real-process
regression fails against the original helper and passes after repair. All 160
available semantics tests and two 100-run cleanup batches pass. Production LSP
cleanup was unchanged. Failed runs and diagnostic builds remain in the evidence.

## Process ownership

Initial settled counts were 775 processes and 767 pre-existing PID1 zombies.
Small supervised batches, the exact fixture-maintenance test and all eight
supervisor lifecycle tests passed without accumulation. The first CI reaped
818 orphans without forced cleanup or additional PID1 zombies.

One runtime starting/ready transition discarded an outer execution session
after a performance gate completed. Its completed Python supervisor (PID 987810,
exit 0) was adopted by PID1, bringing the backlog to 768. No live validation
descendants remained. Supervisor and fixture tests passed again afterward.
All subsequent completed batches, including final CI, full suite and frontend
pairs, stayed at **768 zombies**; no fork/thread-creation failures occurred.
`health.jsonl` retains counts and PID identities before and after batches;
`environment-transition.json` records the exception.

Long execution-session collection twice reported status 1 although CI's own
supervisor and saved explicit exit status were 0. Both observations are retained;
the final CI's saved exit status is 0. No failed assertion or forced cleanup was
omitted from the evidence.

## Native packages and publication

The final native build is
[run 42784617241](https://github.com/aantoniadis/lenz/actions/runs/42784617241),
for the exact release source, using the normal shipping `dist` profile and
independent frontend builds. Publication remains pending native package checks
and verification of the six archive/checksum assets against that run.

Superseded runs 42769225197, 42773313394 and 42775328614 built earlier unpublished
candidates. They are retained as history and must not be used as evidence for
the final package source.

## Coverage limits

The feature-specific [replacement/rename validation](project-replace-rename-validation.md)
and [feedback/setup/Find validation](editor-feedback-setup-search-validation.md)
document the merged workflows and their own baselines. Their earlier passes do
not substitute for the fresh combined-tree gates above.

Native macOS/Wayland interaction, physical displays, trackpads and Dock behavior
remain unverified. Native macOS CLI/signature checks do not establish interactive
acceptance. Real npm and a fresh offline Ruby installation passed in the feature
campaign; live Ruby registry discovery and a complete Go dependency download
were blocked. This release does not add a new prolonged live-provider campaign.
AUR registration/submission remains unavailable; only its repository recipe can
be updated. macOS packages are ad-hoc signed, not notarized.
