# Existing-file editor foundation: batch 14–23

Candidate `fce466ff805d490d4d0e411ed00e0c0f3c9c0c12` passes the phased local correctness gate,
native default-input acceptance and the complete release measurement campaign. Its runtime is
`2ca23dd`; the final commit changes only a parity-test capability fixture. **Measured costs await the
user’s decision before activation is merged.** The comparison baseline `bb2f55f4` and its earlier
11–18 costs were explicitly accepted on 2026-10-24. That approval does not cover this batch.

## Delivered scope

This cohort covers tickets 14, 15, 17, 19, 20, 21, 22 and 23: guarded dirty closure and transitions,
current-draft semantic navigation and anchors, explicit dirty external handoffs, a last-saved comparison,
ordinary text input, Unicode composition and bounded long-line editing, and validated-target enablement.
It is an existing-file editor milestone, not completion of the 68-ticket editor specification.

Production input is enabled only in the candidate’s Linux X11 GUI and Linux TUI constructors. Headless
Session defaults remain conservative; Wayland GUI and native macOS remain gated. Both adapters use one
shared Session input capability and the same editing/history/write/transition commands.

The user’s 2026-10-24 policy refinement permits a **fresh Discard Draft and Close** after uncertain Save
without disk recheck, for actual tab, pane, window or all-view closure. It discards memory/history and
leaves disk untouched; the failed old close remains cancelled. Busy operations and source/owner fences
remain protected. Save, external saved-version opening, Reload, workspace and Git transitions retain
their existing recheck/confirmation requirements.

See the scoped reports for [Save](reader-explicit-save.md),
[transitions](reader-refresh-transition-guards.md), [navigation](reader-semantic-navigation.md),
[external opens](reader-external-open-decisions.md), [saved comparison](reader-saved-comparison.md),
[input](reader-text-input.md), [composition](reader-unicode-composition.md) and
[acceptance tools](reader-foundation-acceptance-tools.md).

## Correctness evidence

The full set of local gate checks passed **in phases with focused repairs**, rather than one fresh
`ci-local.sh` invocation returning zero. Raw `gate-summary.json` identifies sources and commands:

- Workspace strict lint in default and adapter-feature configurations; affected final Reader lint;
  all three panic-site ratchets. Reader now counts only its owned core/GUI code, with a lower budget;
  unchanged shared Engine/Content/GPUI code remains enforced by the Diff budget.
- Core 555 tests and the remaining writer/provider/projection/storage integrations; Diff core 631,
  terminal 294 and actual GUI 261; Reader GUI 176, default TUI 119 and feature parity 139 plus the
  focused repaired case. The semantic suite had 97 passes plus its focused repaired menu case.
- Six serial memory checks, workspace documentation checks, the collector suite (72 passes, one skip)
  and three focused updated ownership checks; standalone terminal releases and the unchanged Diff
  stripped size of 68,510,392 bytes, below the 80,000,000-byte gate.
- Real GUI/TUI open → Unicode/CRLF replace/paste → split → edit either pane → Undo/Redo → Save →
  actual 17 MiB + 17 MiB cache eviction → return → Undo, with exact bytes, history, action availability,
  shared buffer identity, independent positions and independent dirty sessions.

All initial failures remain archived. Repairs corrected transitive dependency double-counting in the
ratchet, an obsolete expected context menu, and a parity fixture comparing different platform input
capabilities. A real GUI regression was also fixed: the new close shortcut had shadowed Ctrl-Q Quick
Documentation. Ctrl-Q documentation is restored, Ctrl-Shift-Q retains Quit All, and Close Window remains
available through its menu and native control. GUI hints and help text reflect those bindings.

Owned Xvfb/XTest acceptance with actual defaults, without a writing opt-in, passed native WM_DELETE
veto/repeat/Cancel, two consecutive positive preedits → `éé` → independent Undo, exact Save, new dirty
input, selective Discard and normal process exit. Ordinary standalone TUI PTY acceptance passed literal
q/Unicode, bracketed CRLF paste, exact Copy/Save, dirty quit/Cancel/Discard and terminal restoration.
Native proof executables and their source identities are retained separately; later Ctrl-Q routing is
covered by final writable TestWindow input checks. No native macOS execution is claimed.

## Paired release measurements

Same host and release configuration; immutable ordinary/probe GUI and separately built standalone TUI
executables; private fixtures/displays/XDG state with HOME preserved. No competing owned builds or
collectors ran. Core opening/navigation uses ABBA process order; startup uses eight alternating pairs.
GUI first-content markers and PTY byte receipt are not physical display latency.

| Endpoint | Baseline | Candidate |
| --- | ---: | ---: |
| Core first text, 50,400 bytes | 0.142 ms | 0.147 ms |
| Core first text, 945,000 bytes | 1.386 ms | 1.373 ms |
| Syntax ready, 50,400 bytes | 10.431 ms | 10.615 ms |
| Syntax ready, 945,000 bytes | 230.623 ms | 228.210 ms |
| Cached tab / back-forward | 6.692 / 7.584 µs | 6.753 / 7.635 µs |
| Fake-provider definition completion | 2.164 ms | 2.163 ms |
| GUI first-content marker | 140.50 ms | 139.35 ms |
| GUI ten-tab settled / peak RSS | 176.309 MiB | 176.625 MiB |
| TUI first content | 12.073 ms | 11.986 ms |
| TUI completed Go To | 1.559 ms | 1.597 ms |
| TUI startup settled RSS, median of eight | 16.121 MiB | 16.797 MiB |
| Core stress iterations, three minutes | 994,340 | 994,320 |
| Core stress last-three-window RSS / peak | 23.707 / 37.707 MiB | 23.102 / 36.719 MiB |

Paced wrapped/unwrapped repaint and actual-scroll intervals remain about 48 ms in both builds. Actual
scrolling produced 30 distinct source crops per mode/build; the separate long-logical-line workload is
labelled repaint. Scroll peak RSS was 193.957→193.863 MiB unwrapped and 178.141→179.031 MiB wrapped.
These GUI RSS values are observations from one process per case, not independently reproduced bounds.

The 60-second terminal workflow passed 740 cycles per build, with zero idle output and zero sampled idle
CPU. Its settled RSS was 45.645→37.449 MiB and peak 48.172→40.496 MiB. Prior work already observed
multiple allocator-dependent RSS bands; this result is not claimed as an optimization or leak proof.
The new TUI startup RSS increase is clearer: the eight-run bands are disjoint (16.086–16.215 versus
16.504–16.941 MiB), giving a median cost of **0.676 MiB**.

## Sustained editing and retained history

One actual 180-second shared-command editing workload ran per build, using identical harness source
and the existing fixtures: 19,780-byte/2,000-line Rust, 3,800,000-byte/100,000-line text and a
900,001-byte Unicode logical line. Only owned copies were edited. External providers were disabled;
background syntax work remained active. Every edit, captured Save with newer edits, split and independent
session was checked. All retained history was then completely undone and redone, without a history cap.

| Editing endpoint | Baseline | Candidate |
| --- | ---: | ---: |
| Actual timed editing | 180.009 s | 180.012 s |
| Complete workload cycles | 21,996 | 20,722 |
| Retained transactions per file | 22,082 | 20,803 |
| Rust edit dispatch median / p95 | 0.874 / 1.831 ms | 1.031 / 2.082 ms |
| Text edit dispatch median / p95 | 1.194 / 2.506 ms | 1.283 / 2.558 ms |
| Unicode edit dispatch median / p95 | 1.078 / 2.198 ms | 1.276 / 2.440 ms |
| Full Undo/Redo validation, outside timing | 88.510 s | 85.894 s |
| RSS before full-history validation | 78.387 MiB | 84.051 MiB |
| RSS after full Redo / whole-process peak | 111.391 / 117.285 MiB | 101.203 / 116.223 MiB |

The candidate completed **5.8% fewer cycles**. Existing raw samples were also compared in five matched
cycle windows up to 20,000 cycles, with identical cycle indices and current text lengths. Every window
showed a higher candidate edit median: **+0.048 to +0.319 ms**. This is treated as a cost within this
campaign, rather than explained away by different final history sizes. There was one sustained run per
build; no independent repeat or per-ticket stress rerun was performed. Individual feature attribution
has not been isolated.

Editing RSS is mixed: before history validation it is +5.664 MiB, while whole-process peak and final
Redo RSS are lower. The builds retain different transaction counts, and RSS includes the independent
oracle and legitimate history. Those values do not establish improved memory efficiency or a leak.

Candidate-only synchronous TestWindow callbacks at 50 KiB / 1 MiB measured median initial preedit
0.128 / 0.140 ms, partial update 0.130 / 0.142 ms and cancel 0.088 / 0.088 ms. Four explicitly tagged
warmups are excluded from each set of 32 measured samples. Assertions preserve source/history/disk and
zero UI-thread full Document conversions. These are callback wall times, excluding worker completion,
paint and native display; the baseline has no equivalent composition feature.

## Decision and evidence

**Pending explicit acceptance:** the repeated TUI startup RSS cost and consistent matched-window edit
costs above. Small GUI RSS differences and editing RSS phase differences remain visible with their
sampling limits. Passing a loose ceiling is not treated as baseline preservation. A decision accepting
these measurements applies only to this cohort.

Raw samples, exact commands, actual durations, machine/configuration, fixture/harness/source/executable/
driver hashes, both failed attempts and the phased gate are under
`target/perf/editor-implementation/merge-14-23/performance/`. Start with `summary.json`,
`matched-editing-windows.json`, `gate-summary.json`, `configuration.json` and the baseline/candidate
identity files. All campaign cases passed; none was rerun.

Native macOS/Dock termination, Wayland GUI input, physical display latency, GPU memory, real language
provider performance, native composition cancellation and longer soaks remain unverified. Native
macOS editing stays disabled. Completion, the remaining editing/language features and the rest of the
68-ticket specification remain separate work. After this WIP merges, work pauses at the user’s request.
