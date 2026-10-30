# Optimization notebook

> Dated measurement reports and their raw evidence were retired from the tree before release; they remain in the private development history.

Evidence and decisions from the 2026-10-03 Linux performance work. Keep unsuccessful and superseded approaches here so a later platform, renderer or workload can justify another trial. Results are specific to the machine and measurement method; they are not universal performance claims.

## SQL metadata completion — 2026-10-29

DuckDB/SQLite metadata stores schema-qualified identifiers. Unqualified table resolution now falls back
to `main` only at table lookup sites, and FROM completion offers the corresponding short names. Visible
aliases remain exact, CTEs shadow the default table, and table functions do not pick up catalog-table
columns. PostgreSQL lookup is unchanged. Empty completion results no longer claim that metadata is
disconnected; the GUI chooser returns to the metadata controls, with Connect selected.

A first implementation duplicated the default-schema entries in the completion map. A release probe
with 512 tables × 16 columns, 300 qualified-column completions and exact 16-column assertions took
542→1,141 ms; that implementation was rejected. Table-only fallback retains the original map. Four
initial alternating pairs were noisy; ten additional alternating pairs measured medians of
551.712→558.405 ms per 300 operations (1.84→1.86 ms each), with paired differences in both directions
and overlapping ranges (baseline 545.616–630.980 ms, candidate 539.719–626.452 ms). The original doubling
did not recur. Per-process peak RSS ranges overlapped at approximately 15–17 MiB; this is neither a
settled-memory measurement nor leak proof. Raw samples, probe source, source descriptions, executable
hashes and the rejected candidate are in ignored `perf-results/sql-connect/`.

Core regressions cover disconnected/connected empty results, quoted names, schema isolation, CTEs,
visible aliases and table functions. The GPUI chooser-to-Connect click test uses a disposable tool fixture;
a real DuckDB session test on a disposable copy of the reported SQLite backup loads 31 relations and
completes an unqualified table while preserving database and SQL bytes. No benchmark instrumentation
remains in product source. This targeted workload does not remeasure startup, physical GUI/terminal
presentation or sustained stress; native macOS remains unverified.

## Explorer commands and file clipboard — 2026-10-29

Explorer names, absolute/project-relative paths, saved-byte Copy/Paste and Git history use shared commands in both
adapters. Copies stream through a 64 KiB buffer into a private staging folder and publish without overwriting;
cancellation, source changes, collisions, symlinks, special files and excessive depth have focused checks.
Desktop file copying uses Linux's `wl-copy` or `xclip`; ordinary text keeps GPUI/OSC 52. A process-wide request
identity and cancellable worker gate order clipboard work across windows. Adapters validate at the actual output
sink, including inactive terminal views, and acknowledge deferred text before graceful close. Missing workers
refuse blocking operations. No extra worker pool or Cargo dependency was added.

Private Wayland light/dark checks passed for actual filename/path clipboard data, file MIME with escaped names,
exact CRLF/UTF-8 tree Paste and Nautilus file-manager Paste. A native 32-file, 32 MiB folder copy passed every SHA-256
comparison; sampled process RSS changed from 193,800 to 194,072 KiB during the request. Its 0.347 s observed completion
includes keyboard-injection delays and polling, not isolated copy time. Native context menus passed normal/narrow
End/Up/dismissal checks. File and directory history launched the actual Diff executable and left the fixture
repository clean. Scoped history tests also retain staged/unstaged changes that cancel against HEAD after Refresh.

An initial history probe exposed pinned GPUI's missing-keymap panic while its temporary virtual keyboard exited.
Keeping the keyboard that delivered Return alive through the child window corrected the fixture: the trace shows
keymap, enter, then modifiers, and both native history launches pass. No dependency workaround was added; this does
not establish correctness during physical keyboard removal. Initial gate attempts also caught stale catalog/menu
expectations and a 200-scroll test budget that needed 203 steps after adding commands; it now derives the budget
from the current collection. The complete local gate passes with unchanged panic-site budgets.

Three quiet alternating release GUI pairs ran 30 Enter/Undo cycles on 10,000-line Rust fixtures. Median process CPU
was 5.21→5.21 s over about 3.46 s of input; observed mapping 320.03→317.22 ms and typing-to-saved-file feedback
841.30→840.87 ms. Settled/peak RSS medians were 195,772/199,048→195,316/199,132 KiB. Mapping is not content readiness;
feedback includes injected key delays. Three full terminal pairs each ran 30 inputs and roughly ten seconds of stress;
median stress key-to-first-PTY-byte was 0.722→0.693 ms, idle output/CPU remained zero, and settled/peak RSS medians
were 19.219/19.484→19.398/19.398 MiB. A small initial startup/arrow-key timing difference was inconclusive. Eight
additional balanced pairs (after one warmup pair) measured first filename/content at 4.578/5.665→4.167/5.573 ms and
median arrow-key first-byte feedback at 0.659→0.651 ms; the initial difference did not reproduce. A one-off probe
attempt accidentally hashed the binary inside its timing interval and was discarded, with its output retained.

The terminal baseline was rebuilt from `5157367` in a disposable worktree with its own on-disk target, reflinked
unchanged dependencies and an explicit clean/rebuild of all workspace dependencies; its artifacts and build logs
were retained before cleaning and removing that worktree. Raw samples, failed attempts, fixture/helper identities
and source/executable hashes are under `perf-results/explorer-actions/` and `perf-results/wayland-editing/`.
No material repeatable regression appeared in these matched short workloads. Physical display/terminal latency,
long-session stress, real X11 file-manager paste, native macOS and GPU-memory/leak bounds remain unverified.

## Editor polish — 2026-10-28

Typing previously dropped all lexical colours until the next parse. The editor now retains one prior syntax arena
and snapshot, with surviving unchanged byte intervals; inserted and replaced bytes wait for fresh colours, and
semantic navigation and modifiers remain current-only. A release component probe after 512 disjoint edits measured 0.166 ms
for 60 rows of mapped lexical lookup, versus 0.00655 ms for direct current syntax. The interval map occupied
12 KiB, alongside 1,040,016 bytes of retained syntax and 420,679 bytes of snapshot storage in that fixture.
Accepting a current parse releases the fallback; restored adjacent intervals coalesce. The byte-identity oracle
also covers crossing deletions and inserted gaps. These figures exclude adapters and display latency.

The first full-document completion popup prototype shaped on the UI thread. A development test exposed a
22.29-second render for a 100,000-character line in a 180 px window; that approach was rejected before delivery.
Popup layout now uses background work with preview/source/width/theme identity, cancellation between bounded
shaping chunks, cached glyph-position lookup, and grapheme-safe wrapping. One oversized grapheme remains atomic
and can exceed the usual 4 KiB chunk; cancellation is checked around it. Table cells wrap in documentation
popups; ordinary Markdown preview retains its existing layout. The GPUI test platform uses a no-op text system,
so its feedback/completion timings establish orchestration only, not native font cost or display latency.

An owned X11 native-font probe then checked the same 22,718-byte Unicode corpus three times at 64/180/320 px:
completed layout medians were 26.29/15.72/15.45 ms. Native row widths and full grapheme/content preservation
passed, including the one-oversized-grapheme exception. Ten concurrent foreground-shaping trials measured
0.014–0.043 ms; cancellation-to-join was 0.49–0.92 ms. These are text-system calls, not display timings.
The production Wayland executable also passed light/dark file/tree/typing workflows and full documentation
scrolling, including a 100,000-character entry and narrow popups, on a private software-rendered display.

Three alternating release pairs ran 30 Enter/Undo cycles on an owned 10,000-line source fixture, about 3.46 s
of active input per sample, with saved-byte assertions. Median observed mapping was 320.1→319.6 ms; settled
RSS 197,200→197,596 KiB and peak RSS 203,720→203,800 KiB. Burst process CPU was 5.10→5.09 s. The roughly
843→841 ms typing-to-saved-file observation includes injected keyboard delays and is not physical input latency.
No reproducible material regression appeared in these short matched workloads; long-duration stress is unverified.

Raw regression, component and release/native evidence is retained under `perf-results/editor-polish/`, including
source/executable identities and unsuccessful attempts. Native macOS and physical-terminal display behaviour
remain unverified; short native workloads do not establish a long-session leak bound or GPU-memory use.

## Inline diagnostics and explorer error styling — 2026-10-28

Visible source lines now use one exact-version, severity-ranked diagnostic summary. Messages are bounded at
160 Unicode scalars and painted separately from source text, selection, native input and scroll extents. Empty
or absent diagnostics return before extra GUI layout traversal. File/folder names remain literal; errors use red
names and a separate standard bullet, independently of dirty/Git indicators and icon fonts.

The reported Java `[1,2,3];` was detected and underlined by the previous executable on an owned full-file copy,
and after native typing before F8. The user also confirmed F8 revealed its underline. No parser or scheduler fix
was warranted; inline text improves visibility. Tests verify exact source/presentation identities and actual
underline rectangles, severity grouping, UTF-8/EOF anchors, wrapping/folding/sticky rows, narrow layouts and unchanged
Copy/Save bytes. The full local gate and private Wayland light/dark checks pass. A new tree test initially raced
Git status; it now waits for the actual Modified row and passes ten repeated checks without changing timeouts.

Three alternating release pairs per corpus ran 30 native Enter/Undo cycles on 10,000-line Rust fixtures, once
without findings and once with a visible syntax error. Clean median process CPU was 5.00→5.01 s over 3.46 s of
input; error-present CPU was 5.04→5.05 s. Window mapping was 296.9→298.1 ms and 297.4→303.6 ms, respectively;
the collector's window observation is not content/display readiness. Observed typing-to-saved-file feedback was
839.33→839.95 ms and 839.57→839.43 ms, including injected key delays and polling. Clean settled/peak RSS medians
were 196,540/200,888→198,092/202,740 KiB; with errors they were 197,516/201,480→198,076/202,496 KiB.
No material CPU or feedback regression appeared in these short samples; RSS is not GPU allocation or leak proof.
Raw samples, executable/source identities and failed attempts remain in `perf-results/inline-diagnostics/` and
`perf-results/editor-problems/`. Native macOS, physical terminals and long-duration stress remain unverified.

## Data editor and syntax checks — 2026-10-28

The remaining Java flicker came from dropping semantic colours while keeping lexical colours. One prior semantic
payload now follows surviving byte intervals until current tokens arrive; modified token fragments borrow no
colour. Modifiers, navigation and underlines still require current results. A release Java component probe after
512 disjoint edits measured a median 0.471 ms for combined mapped colour lookup over 60 rows, versus 0.061 ms
with current lexical/semantic data. Both maps occupied 24 KiB; this fixture retained 160,000 token bytes,
560,016 syntax bytes and 482,598 snapshot-storage bytes. This is a different corpus from the earlier lexical-only
probe, and excludes frontend shaping, painting and display latency.

Explicit project syntax checking uses the captured inventory and one background file continuation at a time.
Five 1,024-file XML scans completed in 35.0–41.1 ms, with exact finding, no-server and unchanged-file assertions.
RSS settled at 19.64–19.77 MiB after scans and 19.86 MiB after 5.03 seconds of scan/cancel stress. A 48-row
file/folder-count helper batch measured 1.21 ms with 1,024 known error paths and 0.0021 ms without reports;
this changes data volume, not code, and does not measure adapter rendering. Only an explicit action scans files.

The XML validator/highlighter/folder processed 800,014 bytes in a median 20.99 ms across six samples. A
five-second loop completed 386 parses with correctness assertions. Two parser-limit attempts were insufficient:
the dependency bypasses attribute limits for general entities and expansion limits for parameter entities.
Internal declarations now produce an informational validation limit before reaching that expander, while
highlighting/folding remain available. External entities are never resolved. Small internal entities are therefore
also outside validation; an expanded-data limit is not evidence of malformed XML.

The first sorted CSV projection accidentally retained the larger sort-key allocation as its row-index vector
(6,399,968 bytes for 199,999 indices). Keeping compact indices reduced it to 1,600,000 bytes. The final 200,000-record,
6,177,761-byte component fixture measured median parse/filter/sort times of 3.55/1.08/16.98 ms; column move/drop
assembly was 2.44/1.86 ms, and decoding 60 × 3 visible cells was 0.0048 ms. A 10.000-second loop completed 219,587
500-row edit/Undo cycles with byte assertions; RSS samples settled at 69.26 MiB while the large projection remained
owned. These are candidate-only component measurements, without rendering or storage throughput. Cell edits use narrow
source ranges; whole-column changes are assembled off-thread into one undoable source edit, preserving raw fields
and line endings. Filters and sorting are view state. Parser, scan, native and failed-attempt evidence lives under
`perf-results/data-editor/`; benchmarks use disposable fixtures and private Linux displays. Short workloads do not
establish long-session leak freedom, physical display pacing or hardware GPU allocation.

The final full local gate passed, including GUI/TUI adapters and unchanged panic-site budgets. Private native
Wayland light/dark replays passed XML errors, explicit unopened-file scanning, identical unchanged Java pixels while
semantic tokens waited, CSV inline editing, column moves, Save/Undo bytes and narrow layouts. Native testing caught
and repaired a table-mode Undo key that the initial GUI test had not asserted; its regression now checks exact
content after Save for Undo and both Redo bindings. Clean-file checking no longer changes tab widths; only known
errors receive a pending badge. Manual tab-strip scrolling remains independent of selected files, with activation
and ordinary resizing revealing the selected tab.

Three alternating production pairs ran 30 Enter/Undo cycles on the same owned 10,000-line Rust fixture. Median
window mapping was 317.6→298.3 ms, burst process CPU 4.91→4.82 s over about 3.46 s of input, and observed
typing-to-saved-file feedback 839.36→839.84 ms. Settled RSS was 197,456→197,916 KiB and peak RSS
202,720→204,104 KiB; both ranges overlap across repetitions. No material CPU or feedback regression appeared in
these short samples. Mapping is not content readiness; injected keyboard delays and file polling are included in
the save observation. Native macOS, physical terminals and long-duration stress remain unverified.

## 2026-10-24 dev profile and build output

Long sessions filled the disk: one worktree's `target/` reached 213G, 148G of it
`debug/`. Each worktree's target only grows (Cargo collects no build artifacts),
and evidence kept under `target/perf/` made `cargo clean` unsafe.

Method: `scripts/ci-local.sh`'s cargo steps with tests compile-only, each variant in a
fresh on-disk target, then edit loops (`touch crates/content/src/lib.rs` or
`crates/code-gui/src/lib.rs`, rebuild). Source `d17b3b4`, rustc 1.99.1 on Linux (LLD
already the default linker). One cold sample per variant; the core edit loop ran twice.
"Disk" is the filesystem's used-bytes delta over all dev steps, release steps excluded;
`du` overstates it on a compressing filesystem.

| Dev profile | Cold workspace tests | Core edit | Leaf edit | Disk (MiB) |
|---|---|---|---|---|
| none (full debug info) | 145 s | 179–204 s | 9.2 s | 15,156 |
| **kept:** line tables, dependencies none | 90 s | 5.0–5.5 s | 3.9 s | 6,500 |
| full for workspace crates, dependencies none | 96 s | 8.9–16.1 s | 4.8 s | 8,143 |

The core edit relinks every test binary at once. With full debug info the 52 test
executables totalled 14.3 GB (up to 813 MB each); a repeat sampled 26 concurrent
`ld.lld` holding 37.6 GiB (242 s), against 12 holding 4.1 GiB with the kept profile.

- **Rejected: a target shared between worktrees** (`CARGO_TARGET_DIR` or `build.build-dir`).
  Workspace crates are keyed by relative path and judged fresh by mtime: after a
  second worktree built into the first's target, the first compiled nothing and its
  `code-tui` test binary lacked a test only the first has.
- **Rejected: `CARGO_BUILD_JOBS=4`.** With the kept profile it cost 111 s cold (90 s) and
  6.5–7.9 s per core edit (5.0–5.5 s); its reason, link memory, is gone.
- **Inconclusive, now cheap:** `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0` in a warm
  target rebuilt only the 9 workspace crates (15.8 s, +521 MiB) and did not thrash
  when alternated; with full dependency debug info it duplicated every dependency.
- Feature variants (`gui-test`, `test-support`) recompile ~100 units and alternate
  without rebuilding; their cost was the relinked debug info.
- Not tried: sccache (handoff evidence unproven), mold/wild, `split-debuginfo`, fewer
  integration-test binaries. Release, dist and benchmark builds are unchanged; native
  macOS is unverified. Raw logs, timings and drivers: `perf-results/build-profile-20261024/`.

## 2026-10-21 final release validation

The full Linux suite passed 133 cases and 394 metrics, alongside local CI,
absolute performance gates, ten real language-server lifecycle checks and live
Ruby dependency navigation in both adapters. Fresh paired runs measured Reader
wheel feedback at 26.00→26.01 ms and overflow recovery on Discourse at
6.79→0.95 seconds. Three-minute core stress completed 32,380 iterations and 647
external rewrites; sampled RSS windows stayed between 56.20 and 57.04 MiB.

The earlier Reader GUI startup difference did not recur in six fresh pairs
(158.13→159.41 ms), and paired Diff launches did not reproduce the larger
differences against historical measurements. Small timing/RSS differences on
this shared host remain inconclusive; saved local declarations still do not
remove Ruby's initial 12–13-second server indexing.

Two failed collectors needed corrections. Xvfb's fragmented display-number
write caused 8 failures in 150 starts when readers closed before the newline;
the shared bounded readiness reader passed all 150 repeats without retries.
Reader TUI's obsolete title oracle was replaced by filename feedback and its
schema advanced to 2; the real source-content endpoint is unchanged. These are
collector fixes, with no application Rust/Cargo changes in this pass.

See [release validation](release-validation.md) for complete paired measurements,
actual stress durations, failed attempts, limits and reproduction. Raw samples,
identities and drivers are under `target/perf/release-check-20261021/`. Native
macOS, hardware rendering and distribution packaging remain unverified here.

## 2026-10-18 Reader toolbar and status

Reader now follows Diff's separate toolbar/tab layout in both adapters. Shared
core helpers provide commands, brief status, position and complete details;
adapters retain their pixel/cell layout. Brief status excludes routine provider
memory samples, the GUI paints each footer segment once, and terminal independent
views no longer cover status with view-switching text. Full details are captured
on request, so background changes cannot move the text while it is being read.

The complete local gate and production dark/light, narrow/tiny, clipboard and
modal-isolation replays passed. Sixty-second status cycles showed bounded sampled
RSS and unchanged source/configuration bytes. Two alternating release comparisons
found timing differences within pair variation. GUI initial sampled RSS rose
0.6–1.3 MiB, while both 15-second sustained comparisons settled below the saved
baseline; this distinction remains explicit in the report. The first native GUI
OCR oracle missed a visible Actions tab and was narrowed to the relevant tab;
the failed run was retained. No source changes were needed after visual review.
See [Reader chrome measurements](code/performance.md#reader-chrome-parity-2026-10-18)
for raw samples, identities, limits and the unverified native OS chooser/macOS
and hardware-rendering cases.

## 2026-10-18 Reader theme chooser parity

Reader now uses a shared ephemeral preview command for the same chooser behavior
as Diff. Navigation previews, Enter/click commits, and Escape/outside click
restores the latest committed theme. The palette changes without copying a
source document or saving a preview. Theme registry refreshes retain the
highlighted ID. Change theme is a direct GUI app-menu item and an application
command in the TUI, outside View.

Actual GUI and TUI input tests first reproduced the absent preview; the complete
local gate then passed with the fix. Real production input checks covered current
selection, header/wheel isolation, commit/cancel and tiny layouts, including
60-second preview/cancel exercises with unchanged source/config bytes. The
initial GUI collector's log-close error was corrected and its unsuccessful run
retained. See [Reader theme measurements](code/performance.md#theme-chooser-parity-2026-10-18)
for alternating timing, memory, raw evidence and limits. Native macOS and hardware
rendering remain unverified.

## 2026-10-18 dependency refresh

The audit found 13 outdated registry dependencies and a GPUI tag update. Both
GPUI references now use Zed v1.24.2 with the same pinned upstream patches.
Compatible transitive refreshes reduce the lockfile from 821 to 783 entries.

| Experiment | Evidence and outcome |
|---|---|
| Keep the old theme parser after upgrading TOML | Rejected: the existing custom-theme test failed because `Value::from_str` now parses a value. `toml::from_str` reads the whole document; all 19 selected theme tests passed. |
| Rely on GPUI to enable resvg's raster support | Rejected: the embedded-PNG SVG test rendered transparent pixels after the renderer versions separated. Both GUI adapters now enable `raster-images` explicitly; the same test passed. Standalone TUIs retain their existing feature scope. |
| Reuse XZ fixture generation without feature changes | Rejected at compilation: lzma-rust2 0.23 gates its encoder separately. Enable it only for tests; production decoding and archive validation remain unchanged. |
| Production Reader input/resource checks | Passed in light/dark appearances, including Recent wheel/Enter, exact copied Unicode, caret arrows, tiny resizes, unchanged fixture hashes and terminal restoration. Short stress samples remained bounded; they do not prove long-session leak freedom. |
| Full-suite screening outliers | Both suites passed 75 cases/394 metrics. Adjacent repeats did not reproduce the merge-startup or small-GUI/Reader memory increases. Raw observations remain available rather than replacing them with only the favorable repeats. |
| 4K blend microbenchmark | Reproduced 34.12→41.20 ms in test executables. Disassembly shows changed compiler output, with the opposite layout change in production; the precise timing mechanism remains unresolved. Three production image pairs instead improved onion feedback 93.32→79.21 ms with comparable memory. Retain the blend code and document the slower test result; no product regression was reproduced in this path. |

See the [dependency refresh measurements](performance.md#dependency-refresh-2026-10-18)
for versions, verification, workload boundaries and raw evidence locations.

## 2026-10-17 bottom commit panel

The Focus commit panel now spans the bottom in both adapters and can be hidden
completely. Alternating release measurements found comparable startup, scrolling,
and Find feedback when controlling the displayed content. Sequential page inputs
reached the end earlier in the old layout; search caret movement also selected
different folded/expanded regions because the viewport height changed. Both
confounded observations remain in the evidence. A trial caching panel height once
per frame did not help and was reverted. See the [panel verification report](commit-panel-performance.md)
for measurements, short stress runs, correctness checks, and unverified platforms.

The hardware follow-up used an owned headless compositor with RADV Vulkan and
verified application graphics-engine activity. Fifteen-minute GUI runs completed
688 rounds per build; TUI runs completed 2,584/2,592. GPU allocations plateaued at
the same levels; GUI final-quarter RSS grew by 0.25 MiB in both builds and TUI
final-quarter RSS was flat. Hardware log first paint measured 137→134 ms in eight
alternating pairs. No product optimization was needed. Xvfb's hardware-ICD trial
was excluded because it fell back to CPU rendering. Detailed results and the
different framebuffer-polling feedback endpoint are in the
[follow-up report](commit-panel-performance.md#hardware-gpu-and-15-minute-follow-up).

## 2026-10-13 archives as folders and Folder Summary

Quick suite (`perf-suite.py --profile quick`, one run, Linux, rustc 1.99.0) on `main` (`12b682b`) against the branch, then repeated alternating pairs for the outliers. Raw results were kept only in the session scratchpad.

| Change | Evidence | Status |
|---|---|---|
| Archive readers (zip with deflate, deflate64, bzip2, lzma and xz; tar with gzip, bzip2, xz and zstd), all pure Rust | Stripped `lenz-tui` 40.70 → 41.41 MB (`.text` +0.5 MB). Every terminal process +0.7–0.9 MiB RSS (`tui/pty/settled-rss` 10.9 → 11.7 MiB; large workloads under 1%). `/proc` split: anonymous +36 kB, file-backed code +0.93 MiB. Page attribution: about 130 kB of resident archive code (lzma 68, zip 40, zstd 16 kB) faulted in next to hot code; the rest is existing core/std/lenz code spread over more pages by the new layout | **Kept; the maintainer accepted the cost.** Clean, shared, reclaimable code pages, not heap. Revisit with symbol ordering, or by dropping rare zip codecs (deflate64, bzip2, lzma/xz inside zip), if baseline RSS matters more. |
| Speed of existing workloads | Quick-run outliers did not reproduce in alternating pairs: `tui/scroll-render-100k` base 0.305–0.307 ms vs branch 0.269–0.307 ms (3 pairs); `tui/merge/render-100k` 0.876–0.884 vs 0.864–0.906 ms; PTY `history-filter` 0.22–0.28 vs 0.18–0.28 ms and `view-switch` 2.01–2.30 vs 2.04–2.09 ms (2 pairs). `resize-first-byte` already ranged 0.09–0.64 ms within the base run | **No reproducible regression.** |
| Change Set load and Files rows | Non-archive sets add one file-name suffix check per Comparison; `Folders::rows` keeps the 10k-file navigator workload (`tui/chrome`) within noise | **Kept.** Archive loads read each changed archive whole (≤ 512 MiB) and decompress tar streams to fingerprint members; no large-archive workload exists yet. |
| Folder Summary counts | Counted on the shared document worker, 8 files per batch, keyboard rows first, cancelled on close or a new Change Set; at most two decompressed tar streams of ≤ 64 MiB are kept for member reads | **Kept**; not benchmarked. A workload with thousands of members is the next measurement. |

## 2026-10-11 code review fixes

| Change | Evidence | Status |
|---|---|---|
| Unified and padded frames visit only the visible rows of a long unchanged or changed run | `view_model_perf` (release, 100k lines): Unified scroll + frame 0.0007 ms; Shared 0.0077 ms both before and after (three runs each) | **Kept.** Shared mode sits near its 0.008 ms budget in both builds; the idle recheck on 2026-10-11 measured 0.0076 ms (`7a29bfa`) and 0.0077 ms (release), so it is the machine, not a regression. The 100k view-model build measured 0.75 → 0.83–0.94 ms in the same pairs, inside its 1.2 ms budget. |
| Commit messages cut at 16 KiB for the Focus message panel, which both frontends re-wrap while drawing | Bounds the per-frame wrap; `a_huge_message_is_cut_for_the_panel` | **Kept.** A wrap cache keyed by width and message remains the upgrade if long ranges show in profiles. |
| Back/Forward reads HEAD from the loaded refs | No `git rev-parse` per step once refs are loaded; startup restore still asks git | **Kept.** |
| GUI wheel during Markdown shaping drops the turn instead of shaping on the UI thread | Clicks and keys still build synchronously so they land | **Kept**; not measured. |

## 2026-10-10 TUI About logo clarity

Native Ghostty inspection, lifecycle checks and five alternating PTY pairs were
recorded. A saturation-only
adjustment was unsuccessful and removed. Reusing the existing graphics transport
for the GUI's 96×96 PNG is kept: the maintainer accepted about 0.3 ms extra per opening
and 0.6 MiB more process RSS for the detail. The cell fallback remains for other
terminals; physical display latency, macOS and long stress remain unverified.

GUI work used a private 1600×1000 Xvfb display with software rendering. Reproduce new trials with [the benchmark suite](benchmarking.md).

## 2026-10-17 incremental repository loading and PR reading

Linux release builds, same machine, shared-core fixture with 5,001 branches and 25 worktrees; medians of six session openings. The original executable was retained before product edits, along with source/executable identity and raw samples. These measure core readiness, not painting or GitHub latency.

| Measurement | Before | After |
|---|---:|---:|
| First comparison ready | 113.7 ms | 10.2 ms |
| First branch batch available | 105.2 ms | 34.8 ms |
| Full local inventory settled | 116.8 ms | 97.9 ms |
| Settled test-process RSS | 12.16 MiB | 10.76 MiB |
| Retained local branches | 5,001 | 20 |

| Experiment | Outcome |
|---|---|
| Read the first 20 branches before worktree status, then complete other metadata independently; separate local, inventory, GitHub API and PR fetch workers | **Kept.** Comparisons no longer queue behind inventory or network calls. Workers are reused; superseded inboxes and request tokens reject obsolete work. PR-ref cleanup stays on the fetch worker so it follows in-flight fetches. |
| Re-read the same branch batch while completing metadata | **Rejected.** The first version settled around 135 ms versus 117 ms before. Removing the duplicate scan brought completion below the baseline. |
| Page local branches, remote refs and PRs in batches of about 20 | **Kept.** Local filtering can request the full inventory; PR filtering covers loaded requests. `gh pr list` has no offset, so another PR batch refetches a larger prefix. |
| Shape a PR description synchronously during render | **Rejected.** Native formatting now runs on the background executor with source, width, theme, font and request identity checks; provisional content remains available. |
| Reuse the Markdown parser, reading style and native glyph shaping for PR prose and code | **Kept.** The final release test's 35,255-byte description prepares its render in 1.49–1.69 ms; formatting completes in 39.1 ms cold and 4.99–5.11 ms warm in the background. Cached render preparation is 0.020–0.023 ms. These are native-text test measurements, excluding canvas paint and physical presentation; absolute process RSS was 78.1 MiB, not a leak or baseline-memory claim. |
| Group the on-demand PR Markdown and cached page builders with `#[cold]` | **Kept.** Warmed, same-filesystem plain-TUI RSS overlaps at 10.6–10.8 MiB. Without the hints, the changed binary retained about 1.3 MiB more executable pages. Anonymous memory and thread counts were unchanged. Marking only the Markdown builder fixed plain comparisons; grouping the cached builders also removed a consistent extra 1 MiB in the conflict workflow and a 0.6 ms second-view slowdown. Final paired conflict RSS was 11.93 → 11.76 MiB; second-view timing moved in both directions. A temporary `MADV_RANDOM` diagnostic isolated executable fault-around; no preload, allocator or system configuration ships with the change. |

The full Linux suite collected 133 passing cases and 394 metrics, including a 10-second sustained mixed workload and three repetitions of the GUI interaction scenarios on a private software-rendered display. Its final comparison guard held the baseline's earlier "running" status; a separate post-collection comparison verified the completed baseline and matching configuration without changing either raw report. Evidence remains under `target/perf/2026-10-17-repository-{before-frozen,after,ux}`.

Three alternating, same-filesystem pairs did not reproduce the original search, blame and annotation outliers: Change Set search 5.07 → 4.83 ms, blame readiness 3.82 → 4.00 ms and annotation application 0.0449 → 0.0467 ms. The engine and content-test executables were byte-identical before and after, so their timing differences cannot be attributed to product edits. Initial TUI RSS comparisons also mixed a saved executable on tmpfs with a candidate on btrfs; the same-filesystem measurements and page attribution supersede those residency comparisons.

Final GUI startup pairs (three alternating pairs, warmed binaries on btrfs) measured first paint at 133 → 136 ms for the repository, 143 → 143 ms for Windows-1252, and 140 → 141 ms for Unicode. Fresh application-cache merge loading was 146 → 148 ms. Startup CPU medians were about 0.05–0.07 seconds higher on the small fixtures, with overlapping repository sample ranges. Repository settled GUI RSS consistently rose from 200.8 to 208.3 MiB. A separate settled `smaps` pair attributed about 6 MiB to one additional shared `memfd` mapping whose size matches a 1536×1000 RGBA window buffer; another roughly 1 MiB was heap and executable residency. This is software-rendered process residency, not measured hardware GPU memory or proof of a leak. **Accepted by the maintainer after checking the changes on Linux.** The TUI placement improvements and core startup gains do not erase this GUI cost.

Correctness checks cover load-more input in both adapters, remote paging, filtering unloaded branches, refresh during startup, slow GitHub listing alongside local navigation and PR opening, stale PR cancellation, persistent loading feedback, native link hit testing, Unicode/tab copying, End navigation and narrow layouts. PR tables retain compact text separators; images remain linked placeholders, and the timeline does not expose the file Preview's callout/image controls. Native macOS and live GitHub timing remain unverified.

## Kept: remove work and retain less

| Technique | Evidence / result | Trade-off and when to revisit |
| --- | --- | --- |
| Do not load/diff/highlight the hidden two-way comparison underneath a standalone merge; initialize Base syntax when shown | Part of `80ca1da`; large merge first paint 387 → 276 ms and three-second RSS 1341 → 469 MiB for the combined first pass | First opening of Base can require preparation. Revisit if Base visibility becomes the default or startup flow changes. |
| Materialize compact syntax spans, release syntax trees, reuse one parser worker | 100,009 Rust lines retain 5.2 MiB; visible 60-line span copies about 1.7 µs | More work before colors appear. Large multi-pane merge syntax finished roughly two seconds after text/diff first paint. Reassess for interactive editing, where retaining an incremental tree may pay for itself. |
| Delay large-file syntax until after first paint; cancel hidden/obsolete jobs and reject stale completions | Lower startup contention and redundant work; visible final syntax checked in screenshots | Cancellation is cooperative. Keep document identity/generation checks even if scheduling changes. Measure readiness separately from first paint. |
| Restrict per-frame processing to visible lines; binary-search block ownership instead of rebuilding full-file maps; retain line maps in layout | View-model scroll/layout around 7 µs; large allocations removed from repaint | The model timing excludes GPUI rendering. Preserve folded/independent/padded coordinate behavior when changing range lookup. |
| Move repository title Git queries out of repaint and refresh on activation | Removes a subprocess from a frequent render path | Labels can be briefly stale until background refresh completes. Revisit with explicit repository change notifications. |
| Bound reloadable document/diff caches by estimated 128 MiB / eight files | Reduces retained document data; active document remains pinned | This is not a process RAM cap. Revisit the estimate against actual residency, and retain undo/merge choices across eviction. |
| Store inverse patches instead of whole-file two-way undo snapshots (`f68feba`) | Ten one-character edits to a 100k-line file retain ten bytes of undo text plus metadata | Undo/redo reconstructs text when used and checks a full-version fingerprint. Preserve normalized CRLF/BOM behavior and stale-version refusal. This does not implement applied-block visibility/selective undo. |

The first-pass improvements were measured as a combined change. Do not attribute its entire startup or RSS improvement to any individual row.

## Kept: cheaper search and document switching

Search (`8c22c39`) uses an ASCII fast path, retains the existing Unicode fallback, produces match ranges lazily, and stops repository search at 10,000 hits. A common hit cap previously limited only output, after the expensive scan/allocation had already happened. Matching 100k lines fell from 42–49 to 4–8 ms; capped repository search from 150 to 3.6 ms.

Paint now looks up only visible rows' matches and excludes spans beyond the displayed prefix of a very long line. Full-text counts/navigation are retained. Long-line repaint with many matches fell from 50–51 to approximately 16 ms; initial query feedback also improved. Revisit long-line handling if horizontal display truncation is removed. Unicode lowercasing, expansion and byte offsets have regression coverage; an ASCII optimization must never accidentally replace those semantics.

Document loading/diff/refinement (`f3e4d98`) reuses one worker rather than distributing large temporary allocations among general-pool threads. Queued stale work is skipped, and diff deadlines hold weak references instead of extending evicted documents' lifetimes. Twelve comparisons / 88 switches fell from 1539 MiB peak and 1289 MiB settled to about 484 MiB. A separate 242-switch run settled around 475 MiB without the former upward trend.

This worker deliberately serializes document computation. A very expensive in-flight job can delay a newer one even when queued work is discarded. Reevaluate on slower machines, concurrent windows, and pathological diffs; measure content readiness as well as early loading feedback. Adding workers can improve concurrency but also multiplies retained allocator arenas and peak memory. No global allocator tuning was shipped.

## Kept: release renderer resources and accelerate blending

Image improvements (`76d846c`):

- Explicitly evict retired image textures from GPUI's atlas when changing files or replacing a blend/crisp enlargement. Dropping the CPU `Arc<RenderImage>` did not remove the corresponding renderer allocation. Six 4K comparisons settled at 2235 MiB before and 763 MiB after.
- Build the difference render texture only when Difference mode is first used. Startup for two 4K PNGs improved from 347 to 303 ms and 829 to 765 MiB. First entering Difference mode now pays that upload cost (about 74 ms in this environment).
- Borrow decoded pixels when computing differences, removing copies of both image sides.
- For equal-size images with at least 64 KiB of RGBA pixels, compute the 65,536 possible old/new channel-pair outputs once for each opacity and then perform table lookups. This preserves the existing floating-point rounding, including alpha. Blend computation improved from 151 to 32–36 ms. Small and unequal-size images retain the original path.

Channel lookup trades about 64 KiB of temporary table storage and setup work for less repeated arithmetic. Verify small-image crossover, memory bandwidth and unequal dimensions on other CPUs before expanding it. Regression tests exhaust all channel pairs at five binary-exact opacities using an independent integer-rounding oracle.

The CPU improvement did not eliminate display cost: 4K opacity feedback remained approximately 115 ms (previously 233–249 ms) under software rendering. An active image using every mode can still occupy about 1 GiB there. Native GPU measurements are needed before choosing the next image optimization.

## Rejected, insufficient, or superseded experiments

| Attempt | Observation | Decision / reevaluation trigger |
| --- | --- | --- |
| Rely on cache eviction alone while document work uses the general pool | Live cached objects were bounded, but repeated browsing still reached about 1.5 GiB RSS. Heap profiling recorded roughly 51 million allocations in the switching sequence. Multiple threads' allocation peaks remained resident. | Insufficient. Reuse the document worker and remove strong timeout references. Revisit pool sizing only with measured multi-window throughput and residency. |
| Set `MALLOC_ARENA_MAX=2` for the old executable | Exploratory settled RSS fell from 1289 to 844 MiB. Rapid-switch group p95s included 85, 158, 76 and 100 ms; corresponding default groups were much lower. The complete single-run samples are retained in the experiment JSON. | Not shipped. It reduced memory less than the worker change and introduced undesirable latency in this trial; it also targets glibc rather than macOS. Reevaluate only as a controlled allocator experiment on the target OS, with repeated latency distributions. |
| Walk contiguous RGBA bytes and do the original floating-point blend directly | Exploratory blend time improved from approximately 151 to 111 ms, still above the 80 ms CPU gate. The temporary code/raw output was not retained; this number is weaker evidence than the committed paired results. | Superseded by channel lookup (32–36 ms). Revisit compiler vectorization/SIMD if target hardware or image-size crossover makes the lookup slower. |
| Drop only the CPU image handles when switching images | CPU ownership looked released, but renderer atlas textures remained resident and browsing memory continued to grow. | Insufficient. Explicit atlas eviction is required for the current pinned GPUI version. Recheck the ownership contract after a GPUI upgrade. |
| Create the difference render buffer eagerly | Paid about 64 MiB for a 4K buffer even when the user stayed in two-up mode. | Replaced by lazy creation. Reconsider prewarming only if mode-switch latency matters more than startup/memory and evidence supports it. |
| Retain full syntax trees for inexpensive later range queries | Queries were convenient, but large-file/multi-pane residency was high. | Replaced by compact spans for a viewer. Incremental editing would change this trade-off; reevaluate then. |
| Collect every repository hit and truncate afterward | A nominal 10,000-hit limit still took 150 ms and allocated excess hits. | Replaced by terminating the scan at the cap. Preserve the streaming/cancellation contract in future search redesigns. |

These results do not establish that allocator tuning or a direct SIMD blend is universally bad. They explain why those approaches were not selected in this version.

## Correctness and measurement discoveries

- A conflict plus a common added EOF newline/blank line crashed merge refinement (`3ce1b3a`). The original timing fixture missed it. Small reproductions and the generated large merge now cover it.
- Keyboard merge acceptance updated state without requesting repaint (`01c7888`). Model tests passed, while a UI probe saw three-second repaint timeouts. All four accept shortcuts and undo now repaint. Keep both model and UI checks.
- The initial three-second RSS probe missed pre-paint high-water memory. The updated Linux collector samples before paint and includes kernel high-water data; macOS uses sampled peaks and labels that limitation.
- A fresh application cache is not a cold OS cache. A first launch in an ordinary session is not a cold-boot measurement.
- File navigation's first damage may show loading rather than completed content. Separate that metric from actual document readiness; do not call it total switch latency or FPS.
- Hard timing ceilings calibrated to one fast workstation are unsuitable as universal cross-machine pass criteria. The portable suite records metrics by default, with explicit optional absolute and baseline-relative gates.
- The old history benchmark used `/proc` plus a hardcoded page size and silently produced zero RSS off Linux. Its benchmark-only resident-memory query is now portable and reports unavailable readings explicitly.

## Not tried in these passes

GPU-side blending, explicit SIMD/integer blend kernels, alternative allocators, native hardware frame tracing, multi-worker scheduling tuned by workload, and a full diff-engine algorithm replacement were not evaluated. Do not list them as failed experiments. The existing `dist` profile uses thin LTO and one codegen unit, but these measurements used `release`; this pass did not establish a release-versus-dist performance difference.

Potential next experiments should begin with the remaining measured bottleneck or a new platform result: native 4K image interaction, content-ready latency behind a long in-flight diff, delayed syntax readiness, or the base renderer footprint. Keep correctness checks and memory alongside speed so an improvement in one metric does not hide a regression elsewhere.

## Shared-core extraction and terminal preview

Ratatui 0.30.2/Crossterm 0.29.0 are isolated in the terminal adapter. Existing GUI
dependency versions were deliberately held fixed; a GPUI upgrade was not tested.

| Decision / experiment | Evidence and outcome | Reevaluation trigger |
| --- | --- | --- |
| Share text loading, view commands and search through direct Rust calls; make GPUI optional | Kept. Five-pair startup medians remained within observed variation: small 99 → 99 ms, 100k text 169 → 168 ms, merge 271 → 272 ms. Standalone terminal dependency tree excludes GPUI. | Repeat the same baseline comparison after each further workflow extraction. Full GUI orchestration is not shared yet. |
| Reuse the existing document worker; poll jobs only while pending and wait on input while idle | Kept. Three terminal probes recorded zero idle bytes and zero CPU at collector resolution; final 500-input probe settled at about 50 MiB. This pass did not compare against a deliberately busy event loop. | Pathological diff readiness, concurrent sessions, slower machines. A running diff is not forcibly canceled. |
| Keep only the active decoded terminal comparison and remember explicit folds | Kept as the initial policy. Rapid-switch tests reject stale results and retain requested folds. Reopening a file reloads/recomputes it; fold state can still retain copied changed text across files. | Measure repeated-file readiness and fold-state memory before introducing a bounded cache or more compact fold identities. |
| Restrict terminal rendering to visible cells and the long-line prefix; binary-search highlight overlap | Kept. 100k-line scroll/render median about 0.18 ms; five-megabyte long-line search rendering 0.015 ms in the full suite, 0.028 ms in the final isolated run. No separate before/after implementation benchmark was retained for this choice. | Wider viewports, removal of the display cap, syntax colors, dense non-ASCII matches. |
| Copy fold text on every repaint-style command | Removed during review: scrolling-mode/navigation actions do not change fold state. Snapshot it only for fold/unfold. Correctness tests and final benchmarks passed; no isolated speedup is claimed. | Future editing/reload behavior that needs persistent block identities. |
| Treat the first flagged large-file search sample as a regression | Inconclusive initially: full-suite first-damage medians 18.44 → 27.83 ms. Controlled typing/clearing repeated 60 times per executable measured 23.92 → 23.81 ms for typing; component search was also stable. No search rewrite was justified. | Reproduce on another renderer/machine or with a query-specific case; preserve the original flagged data. |

That historical preview lacked syntax colors, image presentation, merge and write
workflows; the subsequent parity implementation adds them. The preview’s smaller
footprint cannot establish a speed ratio for equivalent complete workloads. Likewise, PTY output latency is not
physical-display latency. An IPC service, async runtime, alternate diff engine
and additional document workers were not evaluated in this pass.

## GUI/TUI workflow parity

These decisions extend the preview. Shared behavior uses direct in-process Rust
calls; frontend scheduling and renderer resources stay in their adapters. Keep
this distinction when reevaluating an extraction: sharing a model does not prove
its controls, asynchronous acceptance or presentation are correct.

| Decision / experiment | Outcome and evidence | Reevaluation trigger |
| --- | --- | --- |
| Shared comparison, editing, merge/version/conflict, repository/PR, image and search state/commands | Kept. Core byte/state tests plus real GPUI and terminal event/render tests cover adapter wiring. GUI atlas eviction, lazy Difference texture, bounded caches and polling cadence are preserved. | Any new command, changed behavior or frontend scheduling change; replay both adapters. |
| Read/prepare text or images once, share decoded documents through Arc | Kept. Tests assert session/visual ownership shares the same document; canceled results and closed views lose weak ownership. No isolated speedup is claimed. | Additional caches or a new rendering protocol that duplicates the same decoded pixels. |
| Paint only visible image pixels, syntax spans and search matches | Kept. Terminal raster work is viewport-sized rather than a full 4K blend allocation. Eleven alternating hit-lookup trials reduced gathering 100 visible lines from 0.3455 to 0.00538 ms; hit counts are equal. This measures lookup, not total frame time. | Larger viewports, different image sampling or unsorted hit storage. |
| Coalesce streamed blame into immutable 256-line Arc chunks and dirty indices | Kept. Eleven equivalent fragmented-history pairs: producer 3.53 → 3.38 ms, UI ingestion 0.0576 → 0.00393 ms, 100ms-consumer readiness 100.14 → 100.08 ms. Estimated line storage 65,536 → 41,080 bytes excludes allocator headers and commit/path storage. | Different producer/consumer cadence, huge fragmented histories or increased retained snapshots. |
| Bounded 32-message blame channel | Rejected. Backpressure made a 1,280-hunk history take about 3,904 ms under the preserved 100ms GUI poll cadence. Coalesced chunks bound retained updates without blocking the producer. | A genuinely event-driven consumer or a different producer protocol; benchmark end-to-end readiness. |
| One serial worker for both blame sides and commit details | Rejected during implementation: a slow side blocked the other. Separate reusable side/detail workers preserve concurrency and bounded thread reuse. No isolated paired timing was saved for this rejected trial. | Scheduler changes; compare both-side readiness and residency, not only thread count. |
| Compact inverse edits, retained Applied/Undone records and shared refresh remapping | Kept. Fully applied files omitted from a new Git diff remain navigable with Undo; insertion/reordering remaps history by group/path identity, with snapshot/fingerprint write guards. Core and real terminal refresh/write tests verify the correct path. | Renames across refresh, overlapping edits, stale disk writes and large repeated undo histories. |
| Compact fold IDs and bounded eight-file placement cache | Kept. Removed unused GUI document fingerprinting on file leave, which would scan entire documents for state the GUI does not consume. Ordinary empty history/remap paths allocate nothing. No isolated speedup claimed. | File eviction/reload and content changing under a saved fold identity. |
| Independent terminal views retain their own comparison | Kept. Four 4K views intentionally scale approximately linearly; closing views releases Arc ownership. RSS may retain allocator pages. This is a workflow requirement, not a bounded whole-process memory promise. | More simultaneous views, slower machines, or a shared bounded image/document cache. |
| Treat first full-suite component flags as established regressions | Inconclusive. An early eleven-pair run found view construction 1.73 → 1.69 ms; a later pre-review run pooled 1.25 → 1.52 ms with broad process-to-process variation, while its full suite measured 0.838 → 0.860 ms. Construction code is unchanged. Later alternating ASCII decoding measured 13.47 → 13.48 ms, failing to reproduce its large full-suite flag. Preserve favorable and unfavorable distributions; use the final product measurements and report alongside them. | Repeatable comparable samples on another machine; never silently discard unfavorable samples. |

Adapter checks found real defects that model tests alone missed: a terminal
popup painted before the repository was obscured; a view banner overlapped the
mouse-active repository header; F8 activation skipped stale-disk/head checks;
empty GUI refresh rendering indexed a nonexistent comparison; and terminal F3
prioritized Change Set search over ordinary Find. Reopening Find now recomputes
for the focused side and caret. Option recomparison preserves the active document, find/selection and
visual resources, including image mode/zoom. Merge previews expose the nested
comparison menu, completed Git operations return to history, and GUI staging
refuses unsaved binary choices. Refresh also needed shared selection/filter
plans, pinned initial commit specs and rejection of stale followed-history
results. Keep these regressions alongside benchmarks. A real probe also crossed local
midnight and exposed a collector assertion using Git’s committer/local date
against the app’s author/UTC blame date. The collector now uses the author
timestamp in UTC, with a timezone-boundary test; failed cases stop the suite
immediately while preserving their report.

A repeated GUI merge probe exposed a second collector defect: launching from
the checkout with Result equal to Base resumed an already resolved result. Its
"accept" action showed "No change here" instead. The approximately 1,614ms damage
sample was the 1,600ms toast expiry plus a frame; toast creation lacked an
immediate notify. A real GPUI key/notification test failed before the fix and
passed afterward. Corrected probes run disposable copies outside Git and verify
saved bytes for every choice and exact Undo. Historical GUI merge acceptance
samples from workload version 1 do not establish acceptance performance.

The final review also added a shared whole-version disk guard (binary, deletion
and symlink choices), which had been missing from the original GUI picker.
Loaders capture a streaming fingerprint with a 64KiB buffer off the UI thread;
activation checks metadata/link targets, writes verify content, and successful
writes reuse the chosen bytes for the new fingerprint. Repeated Save/Done
explicitly confirms overwrite; Reload discards choices and retains external
bytes. No full binary copy is kept just for validation. Adapter regressions
cover refusal, confirmation and reload, including a larger-than-buffer file.
Additional review fixes retain explicit commit selection against delayed stream
batches, reject stale follow results after Show in Graph, guard narrow commit
popups and display image path headers. These are correctness fixes; no isolated
performance improvement is claimed for them.

The complete rendering check then caught `--name` suppressing SVG detection.
Prepared comparisons now prefer the actual SVG filename for image detection,
while preserving the display-name extension fallback used by difftool temporary
files. The helper borrows paths; it neither rereads nor copies the input. Core
pixel assertions and terminal headers/file-switch/name regression checks cover
both detection and visible presentation.

The comprehensive suite adds producer/consumer blame, streamed search,
terminal lifecycle/merge/independent-view stress and current-cell PTY workflow
assertions. Native macOS, physical display timing and authenticated GitHub timing
remain unverified. CPU cell rendering and PTY byte feedback do not measure the
terminal emulator’s display latency.

The final Linux parity run passed 116 cases/364 metrics. Focused decode and image switches did not reproduce the
initial flags. GUI settled startup RSS stayed close to the saved preview. The
corrected merge paint median rose 12ms in one five-pair set, with overlapping
ranges and equivalent 114ms load-to-paint medians; this remains inconclusive.
Terminal first content improved 115 → 37ms and settled RSS 50 → 48 MiB, while
empty paint rose 0.43ms and peak RSS about 4.62 MiB. Preserve those distinctions
when reevaluating the 16ms busy poll and early document publication. A one-ms Unicode load flag had no dedicated paired rerun.

## GUI-like terminal chrome and crash follow-up (2026-10-04)

The GUI crash was obsolete Change Set search indices surviving a repository
selection change. A real GPUI test reproduced index 6 against a four-file list;
selection replacement now cancels search and clears its menus, including empty
selections. This is a correctness fix; no algorithm speedup is claimed.

Terminal toolbars use borrowed fixed labels and disabled reasons. Ordinary
positive-width text avoids duplicate grapheme processing in cell drawing;
controls and zero-width/combined sequences retain the explicit safe path.
The toolbar caches at most three cell rows, invalidating on labels, availability,
active/focused controls and geometry. It clears the cache on zero-size frames.
The File Navigator caches grouped rows on Change Set identity and holds a Weak
reference; rendering visits visible rows, with selection lookup only when it
changes. Its 10,000-entry benchmark exercises navigator metadata/rendering,
not 10,000 Git reads or document loads.

An initial toolbar build measured 0.187 → 0.202ms for 100k scroll/render in eleven
alternating pairs. Borrowed labels/text drawing reduced the gap (0.183 → 0.189ms
in another set). Bounded cell caching measured 0.187 → 0.172ms. These are separate
sets with overlapping ranges; use the final paired distributions for acceptance.
Repeated zero-size frames caught a stale cached-rectangle panic before release;
a regression fails without cache invalidation and passes with it.

The existing 16ms busy poll can delay a small job completing just after a fast
paint. A 100ms startup/input window now polls pending work at 1ms, then returns
to 16ms; settled terminals still block on input and do not periodically redraw.
Commit details and first-content probes check the outcome and idle CPU/output.
No worker/thread or document copy was added for this change.

The first commit-inspection comparison was misleading: background log text
could satisfy the old readiness check while the modal was still loading.
Workload version 3 requires its author/email line. Keep version-2 samples as
diagnostics and exclude that metric from performance conclusions. This
correction is covered by a collector regression; comparisons across workload
versions remain rejected. An earlier full comparison also rejected an omitted
renderer label; the actual cases passed, but that run is not labeled an accepted
comparison. No new allocator or diff-algorithm experiment
was attempted in this presentation pass.

The final presentation/crash run passed 120 cases, 368 metrics. First content
improved 37.73 → 20.07ms; loaded commit details 17.52 → 4.31ms using corrected
readiness. Input stayed near 0.42ms, and settled idle produced zero output and CPU
ticks at OS accounting resolution. The PTY RSS medians cross two observed bands
near 46 and 49 MiB; retain the overlapping ranges rather than asserting identical
memory from one median. Final viewport render measured 0.199 → 0.174ms in five
pairs, with overlapping ranges. Some tiny presentation paths cost a few more
microseconds; distinguish them from complete workflow feedback.

A compiler-inlining hint on the shared syntax color lookup was tried and rejected:
its five-pair 50-row syntax median remained 5.32µs (saved baseline 2.73µs), versus
5.28µs before the hint. The hint was reverted. Shared styling still costs about
2.8µs more in that isolated probe, with no corresponding full viewport slowdown
in the paired set. If profiling identifies token styling as a material cost on
another machine or viewport size, reevaluate it using both complete frames and
correct palette/diff-background assertions. Do not add caches solely to erase a
microbenchmark difference without showing an interaction benefit.

## Record a new experiment

For every trial, record:

1. Hypothesis and affected workload; baseline/candidate revision or patch.
2. Exact command, build/toolchain, hardware/OS, renderer, cache state, and fixture identity.
3. Raw distributions and memory timeline, correctness checks, and unrelated-path regressions.
4. Outcome: kept, rejected, superseded, inconclusive, or not run. State trade-offs and evidence quality.
5. Conditions that would justify reevaluation, plus a saved result/report link.

Keep exploratory notes separate from repeatable paired results. If the candidate code or raw trace was not retained, say so rather than presenting an approximate remembered result as a benchmark guarantee.

## About logo and Linux identity — 2026-10-04

About information and availability are shared compile-time data. Both adapters
expose the command while a comparison loads. There are no Git/version subprocesses,
network requests, new dependencies, build scripts or generated work at startup.
Linux packages install the original 1024×1024 macOS PNG and desktop entry; GPUI
sets `app_id = lenz` to match that entry, without loading a window icon at startup.
Desktop-launch repository selection remains outside this change.

The GUI embeds an 11.2 KiB 96×96 PNG, decodes it only on opening About, and drops
its RenderImage ownership and renderer atlas entry on dismissal. The TUI embeds 2,304 raw RGBA bytes and
renders 24×12 half-block cells, compositing transparency against the current
palette. Export is a development-only Pillow script, never part of Cargo or
runtime execution. The original icon and export provenance are retained.

The full-size raster in the TUI and eager GUI decoding were considered and not
implemented; they were not benchmarked experiments. They would add work/resources
without improving the small presentation. Future investigation could compare a
raw GUI raster against lazy PNG decoding if About opening becomes a bottleneck.
Do not treat this as a measured improvement from an eager/full-size prototype.

Validation and measured memory/timing results are recorded in the
About follow-up, including renderer/allocator
capacity retained after dismissal and any unfavorable comparison samples.

The first About build showed a paired TUI feedback regression (0.419 → 0.541 ms).
CPU sampling found refinement candidate polling repeatedly computing `side_top`
inside the block filter; it accounted for roughly half the sampled user cycles.
The viewport is immutable during that scan. Hoisting its two top positions out
of the loop removes repeated segment binary searches, with only two stack floats
and no cache to invalidate. This changes polling from O(blocks × log segments)
to O(blocks + log segments), preserving candidate filtering in Shared, Independent
and Padded modes. An eleven-pair probe with the optimization measured 0.449 →
0.341 ms feedback; the final executable measured 0.415 → 0.323 ms.
The suite now benchmarks polling with 5,000 mostly offscreen blocks in all modes.

No compiler inlining hint was added. A speculative About-code-placement
explanation was not established: the popup function was actually smaller after
the change, and CPU samples identified the existing shared polling work. The
initial burst-input profiler attempt did not drain all queued keys before quit;
its incomplete run is diagnostic only. The valid profile sends one acknowledged
input at a time. A copied historical workflow driver initially lacked its helper
import path; it was resumed with the helper path set. The missing-import attempt is retained as a diagnostic, not an application correctness failure. The incomplete burst trace was replaced by the acknowledged-input profile; this notebook records that rejected attempt.


Review also caught unbounded GUI End scrolling, unreadable narrow rows, nested
terminal modal input reaching version/save controls, and an empty launcher
retaining a synthetic document. Regression tests reproduced and fixed these;
empty viewers now reject invalid loads and display an empty state. GUI dismissal
explicitly removes the logo from the sprite atlas, matching existing image-view
cleanup. The Linux entry launches standalone About (`--about`) so it opens from
outside Git; a repository chooser remains unimplemented.

The first full collection with the standalone startup case failed after component
workloads because the collector did not import `tempfile`. That partial run is
retained; the import was corrected and a complete new run collected. It is not
counted as an application failure or accepted full-suite result.

Final eleven-pair feedback was 0.415 → 0.323 ms (330 inputs per build; exact final
TUI executable). Five startup pairs stayed close, and flagged cancellation,
view-close, merge and image timing ratios did not reproduce in fresh pairs.
Settled TUI medians were 46.40 → 48.80 MiB within overlapping 46–49 MiB bands;
the cause of those bands was not established. GUI popup reopening retained about
4 MiB of allocator/renderer capacity after dismissal. Neither Arc release nor a loose growth ceiling
proves all residency returns to its original value.


### macOS Dock icon — 2026-10-11

A bare `lenz` executable shows the generic "exec" tile in the Dock. Release packages now put
the GUI in an ad-hoc signed `Lenz.app` (`packaging/package.sh`, the Homebrew formula links
`bin/lenz` into it). macOS associates the process with the bundle even when it is started
through that symlink, from a terminal or as a git difftool, so the logo shows from launch to
quit (`NSRunningApplication` reports `bundleURL` = `Lenz.app` and the logo as its icon). Outside
a bundle (development builds) the GUI sets `NSApplication.applicationIconImage` from an
embedded 256×256 export (57 KiB) after each window's first frame has been presented (two
chained next-frame callbacks), so first paint does no image work; the generic tile still
shows before that and while the process quits, which is what the bundle fixes. Inside a
bundle the runtime path returns after one path check (0.1 ms). `NSRunningApplication.icon`
reports the file or bundle icon, not the runtime one; `LENZ_TIMINGS` prints
`lenz: dock icon set|not set in N ms`.

| Experiment | Outcome |
|---|---|
| 512×512 export | **Rejected.** Install 28.9–30.6 ms on the main thread after first paint (five launches); AppKit decodes at set time. |
| Runtime icon only, no bundle | **Insufficient.** The generic tile showed while launching and quitting (reported by the user); only an icon attached to the executable covers those. |
| Finder custom icon on the bare executable (`NSWorkspace.setIcon`) | **Works but rejected.** Reported correctly, but it lives in extended attributes that copies, tarballs and Homebrew can drop. |
| `Lenz.app` bundle with the existing `assets/lenz.icns`, launched through a `bin`-style symlink | **Kept.** Bundle recognised and the logo reported from the real `package.sh` tarball; the linker's ad-hoc signature does not cover a bundle (`codesign -v` failed), so the package signs the bundle (valid under `--strict`). First paint from the bundle 241–297 ms in two launches, in line with the bare build. |
| 256×256 export, the largest a Dock or app-switcher tile draws on Retina (the development-build path) | **Kept, pending approval of its cost.** Install 9.8–10.1 ms (five launches). Paired native startup, six alternating pairs: first paint small 233 → 228.5 ms, generated 100k 295.5 → 302.5; a ten-pair recheck of generated gave medians 311 → 316.5 with overlapping ranges (293–351 vs 297–328 plus one 498 outlier), consistent with no change since the work follows the first frame. Footprint +2.2 MiB on small (88.3 → 90.5) and +2.6 on generated (139.0 → 141.6), consistently: AppKit's retained icon. Input during the 10 ms after first paint waits for it; the macOS interaction latency suite does not exist, so that is unmeasured. Raw samples in `target/perf/dock-icon/`. |

The terminal frontend has no Dock presence; this is GUI platform presentation only.

## Crash diagnostics — 2026-10-04

Shared std-only diagnostics installs before adapter setup;
healthy startup performs no log lookup or filesystem/subprocess/network work.
A bounded 32-label ring tracks semantic operations without copying documents or
query strings. Backtrace formatting and private atomic writes occur on errors;
latest-report lookup runs only when requested, off the input/render thread.
Function symbols remain in distribution builds so abort-profile reports are useful.

| Approach or experiment | Outcome and evidence | Reconsider when |
| --- | --- | --- |
| Shared panic/error reports and explicit issue drafts | Kept. Both CLIs, real adapter menus, abort probes and terminal cleanup pass. GUI startup and 100k-line input pairs remain close to the saved baseline. | Any new adapter or worker failure bypasses the shared reporter. |
| Log lookup at startup; raw logs in issue URLs | Considered and not implemented or benchmarked. They add startup I/O or send private data before review. | A separately authorized reporting workflow needs different behavior. |
| Initial one-run performance comparison | Superseded by the post-review candidate and alternating probes. Retained adverse/inconclusive samples, including a slower history filter. | A timing flag reproduces in a controlled workload. |
| Focused history-filter and terminal startup replay | The filter gap reversed in 84 samples per build; the earlier paint gap shrank in 31 startup pairs. No speculative optimization was made. | Longer same-boundary runs show a consistent gap. |
| Decode, blame publication and queued view component ratios | Inconclusive. Repeated pooled medians remained higher with overlapping ranges; the reporter is not installed in those test processes. Retained all distributions and made no compiler/inlining change to chase them. | Actual application profiles identify an affected path and repeatable cost. |

The full-profile run covered the 49-case component/PTY subset with ten-second
mixed stress; GUI startup/interaction suite cases were skipped because Xvfb was
not on PATH. Supplemental probes used its discovered executable on owned private
displays. Native macOS and the complete GUI stress matrix remain unverified.
The initial component driver parsed stdout while metrics were on stderr;
measurements were recovered from the retained combined logs, and the saved
reproduction driver now reads both streams. This was a collection issue, not an
application failure. Earlier terminal smoke/reload driver failures and their
corrected reruns are retained in the evidence.


## Shared themes — 2026-10-04

Verification covered shared palettes/selection, custom files, atomic settings and
live Omarchy updates. Both adapters borrow immutable snapshots; parsing, discovery, writes and watches
belong to a bounded shared service. Documents, syntax, undo, images and lazy
About resources retain ownership through theme changes.

| Approach or experiment | Outcome and evidence | Reconsider when |
| --- | --- | --- |
| Immutable palettes with semantic overrides and shared selection | Kept. Six presets, strict user TOML, core command/persistence checks and real adapter input pass. | New semantic roles or a versioned format are needed. |
| Filesystem notifications plus parent/identity tracking | Kept. Directory and symlink swaps followed by target edits pass; failures explicitly use 1-second polling. | Additional filesystems or platform backends are supported. |
| Accepting read-access events | Rejected after actual stress exposed feedback reads: .24 CPU seconds in a .5-second settled interval. Filtering Access events restores zero sampled idle CPU. | A backend needs an explicitly different event classification. |
| Path-only watch registration | Replaced with device/inode comparisons. Without it, an atomic directory replacement detached the subsequent file-edit watch. | A non-Linux watcher needs another identity scheme. |
| Eager terminal event/thread startup | Replaced with initial palette resolution followed by worker/input activation after first paint; activation rescans. Five paired first paints are 2.038 → 2.169 ms. | Startup traces identify additional deferred work. |
| Terminal EventStream and theme-channel wait | Accepted as a known cost (2026-10-05). It wakes an idle terminal without redraw polling, but adds a reader-thread hop. Earlier focus samples .313 → .562 / .486 → .583 ms; latest .481 → .365 ms and resize .325 → .433 ms. The net effect varies; all samples remain retained. | A simpler single-thread multiplexing API can preserve supported input/resize/quit behavior. |
| Elapsed notification settling and deep ancestor discovery | Kept after final review reproduced premature fallback during notification bursts and no watches below deep missing roots. Last colors survive staged replacement; watch-count bounds apply to existing ancestors. Final live receipt median ~51 ms preserves the intended debounce. | Replacement contracts or notification backends change. |
| Release image-harness timing flag | Retained as a limitation: CPU-pinned 15-pair blend 33.384 → 41.204 ms. Production blend has identical normalized instructions and five real 4K GUI input pairs do not reproduce a slowdown. Unrelated image code is unchanged. | A production image workload reproduces the slowdown or compiler profiling isolates a correct fix. |
| Per-token environment reads in test helpers | Removed. Legacy tests cache once; production uses explicit snapshots. Paired 50-row syntax .0033 → .0027 ms. | Tests need a genuinely dynamic environment fixture. |
| Full-profile timing flags alone | Inconclusive. An unchanged engine executable's split timing moved ~30%; alternating repeats clear decoding/image/tab flags. GUI scroll-mode ratios reverse on repeat. Preserve all distributions. | Repeated same-boundary samples and actual profiles identify a reproducible cost. |

The 60-second GUI, production-PTY and TestBackend theme runs show memory plateaus
within their measured durations. GUI RSS retains initial resize allocations;
this is not GPU allocation measurement or leak proof. The full GUI matrix,
15-minute mixed stress and native macOS remain unverified. Collector target
selection and transient fixture-lock handling were fixed with Python regressions;
existing workload-version-3 metric endpoints remain unchanged.


## Pane input and contextual actions — 2026-10-05

Verification covered both adapters and guarded indexed-version discard. Shared commands and
write/undo safety stay in the core; hit-testing and presentation stay in adapters.

| Approach or experiment | Outcome and evidence | Reconsider when |
| --- | --- | --- |
| Preserve exact displayed positions and folding through equivalent recomputation | Kept. Raw anchors lose positions on collapsed rows; actual GUI option changes and core independent-scroll/folded-tail regressions retain the frame. | Layout geometry changes or a different scroll mode requires new placement. |
| Selecting the already-active option | Kept as a repaint without starting another diff. Actual GUI generation/frame assertions pass. | An explicit force-refresh command is introduced. |
| Rebuild display rows on every option change | Removed. Rebuild only when retained presentation differs. Earlier resident-memory differences did not reproduce consistently; no claimed memory gain. | A presentation transition requires rebuilding. |
| Copy all block ranges to verify compatible geometry | Replaced with a block-count/range fingerprint, computed during existing preparation. Retains no row/text copies. Last paired interaction RSS 297.5 → 297.1 MiB; allocator causality remains inconclusive. | A demonstrated fingerprint collision or changed geometry contract needs stronger equality. |
| Construct whole-change clipboard strings to paint disabled menu items | Removed. Shared metadata/range validation runs first; materialization happens only on activation. 100k-line dark/light menu cycles reach stable RSS for 20 seconds. | A new copy action needs another range rule. |
| Scroll beyond the final displayed row | Fixed in the shared view clamp. Initial blank-region flicker probes were invalid and are retained separately; End then Up and empty-state checks pass. | Layout/scroll semantics change. |
| Terminal feedback ratios from one trial | Inconclusive. Repeated alternating pairs reverse small differences; final first paint 2.296 → 2.195 ms and focus feedback .595 → .576 ms. No promised speedup. | Repeated actual workloads reproduce a regression with profiles. |
| Context menus through the Version Picker's parent list | Fixed modal precedence and preview layout. A real two-file modify/delete conflict checks list navigation, preview menu dismissal and unchanged file bytes. | New embedded workflows introduce another parent/child input boundary. |

The final production flicker run and PTY workflow checks use rebuilt executables.
Earlier startup/context and interaction collections precede the final cold menu
guard and terminal Version Picker cleanup; their binary identities and scope are
explicit in the evidence. Native macOS, the full startup matrix, fifteen-minute
stress and the baseline-failing broad PTY resize workload remain unverified.


## Horizontal text scrolling — 2026-10-05

Release verification covered separate comparison/merge offsets, horizontal wheel and Shift-wheel, and
Ctrl-Alt keyboard/menu commands. Geometry and width caches stay in the adapters.

| Approach | Outcome |
| --- | --- |
| One shared horizontal offset | Replaced. Native right-pane input changed 67 samples of the left source rectangle; separate offsets preserve every sampled left frame. Revisited 2026-10-22 at the maintainer's request: the comparison Sides share their position again in every Scroll Mode but Independent (`ScrollMode::shares_horizontal`), kept as two adapter offsets written together, so the left rectangle now moves with right-pane input by design; Merge panes stay separate. Not re-measured. |
| Width scans on every input/render | Avoided. Lazy per-source widths use an ASCII fast path, capped Unicode/tab projection and invalidation on document/result changes; ordinary first paint does no width scan. |
| Pixel/cell offsets reused for hit-testing and caret reveal | Kept. Real input tests preserve the other pane's selection and exact bytes through menus/questions, resize and reload. |
| Merge width invalidation coupled to visible file list | Fixed. A real two-file conflict test at 60 columns reproduced stale offsets after switching to a short file. Invalidation now also runs while the list is hidden; final production PTY stress and conflict workflows pass. |
| Ctrl-Alt arrows for horizontal movement | Kept. GUI Alt arrows already switch files; plain arrows retain caret/navigation behavior. Shared comparison menu labels/availability provide alternate controls. |
| Small GUI startup delta treated as noise after one pair | Rejected. Reversed order and residency components were collected. Pooled medians are 189→195ms and 240.6→242.1MiB; mostly file-backed pages, with overlapping anonymous samples. No claimed preservation or gain; the maintainer accepted the measured startup cost. |

Twenty-second native/PTY scroll workloads reach stable measured residency;
terminal idle output/CPU and restoration assertions pass. Native macOS, physical
display timing and fifteen-minute stress remain unverified.


## PR descriptions and comments — 2026-10-05

Verification covered shared paginated loading and both adapters' complete read-only details dialogs.

| Approach or experiment | Outcome and evidence | Reconsider when |
| --- | --- | --- |
| Shared request identity, pagination and cached section text | Kept. Replaced requests and partial/stalled replies are rejected; full bodies, refreshed text and deleted authors pass core and adapter checks. | A new remote operation needs different consistency rules. |
| Eager GUI dialog cache and inactive render reset | Removed after initial measurements. Dialog ownership is optional and drops on close. Final six-pair standalone paint is 125.5→126.5 ms, RSS 215.2→215.1 MiB; no claimed speedup. | Allocation profiles identify another cold-path cost. |
| Per-grapheme wrapping for ordinary ASCII | Replaced with byte chunking for ASCII, preserving grapheme-safe Unicode. Earlier timing overlaps compilation; no causal latency gain claimed. | Text layout requirements change. |
| Materialize all rendered rows on each event | Avoided. Shared text is cached by section/result, wrapped rows by content identity/width, painting by viewport. Same-section commands preserve identity. | Rich Markdown requires a bounded layout model. |
| Terminal full-details popup cost | Accepted as a known cost (2026-10-05). Final paired opening 1.550→1.906 ms; PR RSS 9.914→10.117 MiB. Repeated opening and 20.066-second stress reach constant sampled RSS. | User requests optimization or a larger fixture exposes a responsiveness issue. |

Darcula/Light native GUI and all six terminal palettes are covered; tiny/Unicode
and modal input checks pass. Real GitHub latency, native macOS, GPU allocations
and fifteen-minute stress remain unverified.


## Rendered Markdown — 2026-10-05

Release verification covered bounded shared preparation and both adapters' real controls, completed-tail
navigation, copy, file changes and independent-view lifetime.

| Approach or experiment | Outcome and evidence | Reconsider when |
| --- | --- | --- |
| Source-only eager Markdown work | Avoided. Cold tests assert no parsed pair or adapter layout; Source retains at most the active semantic pair after opting in and drops adapter layout/syntax ownership. | New cold allocations appear in profiles. |
| Repeated GPUI glyph scans for each wrapped visual row | Repaired after nine-event paragraphs exposed 16–19-second 1 MiB toggles. Temporary run metadata and borrowed native glyphs reduce this to 490 ms cold / 33–46 ms warm, preserving native lookup semantics and existing row shaping. | Native glyph ordering or wrapping semantics change. |
| Allocate per-glyph lookup vectors for every line | Avoided for monotone native runs. Prefix maxima are needed only for reordered glyphs; synthetic/native Unicode equivalence and 100k-glyph allocation-structure assertions cover this boundary. | Profiles show a significant reordered-run workload. |
| Default TestAppContext as proof of native shaping | Rejected. It used NoopTextSystem. Those timings remain explicitly invalid; corrected release measurements use the actual platform text system. | GPUI testing APIs change. |
| First XDamage as completed rendering | Rejected. Production feedback and native shaped-frame completion are separate metrics with different boundaries. | Production exposes a justified completion boundary without benchmark instrumentation. |
| Fallback status or stale Source tail as terminal readiness | Rejected through collector regressions. Current screen plus completed Rendered status plus real tail navigation are required; Preparing and fallback cannot pass. | Terminal status/pane layout changes. |
| Sandbox SIGWINCH failure as a product resize limitation | Rejected. Both builds produce resize output outside sandbox with verified controlling terminals; full native resize assertions pass. Combined resize+End remains separately labeled. | A supported native run reproduces failure. |
| Source timing differences from isolated pairs | Inconclusive. Repeated/reversed GUI and TUI samples vary in direction; no speedup or loose-ceiling preservation claim. | Repeated comparable profiles establish a concrete cause. |
| TUI RSS mixture median as preservation | Rejected. Per-band smaps exposes +1.61–1.64 MiB, mostly executable pages, with unchanged heap bands. This historical checkpoint preceded the final repair; the final baseline-relative costs were accepted on 2026-10-05. | An evidence-supported narrow optimization appears. |
| Fixed menu input while copy/Find state remains active | Collector corrected. Copy selection changes End semantics, and repository Escape opens History; retained failures and screenshots explain the corrected standalone Find/stress sequences. | Input/modal routing changes. |

The full pre-repair supported matrix passed 132 cases/389 metrics. Final repaired
workspace/core/actual GUI/all-theme TUI/Clippy/Python checks and affected release
GUI workloads were rerun; the TUI production binary is byte-identical. Native
macOS, physical display/GPU memory, fifteen-minute stress and external PR latency
remain unverified. Bounded RSS plateaus are not proof of leak freedom.

## 2026-10-05 final rendered Markdown navigation/context repairs

The final review's point-boundary, markup-marker, Source viewport, TUI reflow and
GUI context-pointer failures are repaired at runtime commit `40bcc45`.

- Shared source-point mapping with forward affinity fixes distant changes in a
  single paragraph; range-overlap queries remain for ranges. Reapplying semantic
  anchors after native layout replacement fixes reflow, and current Source top
  seeds mode switches without mutating hunk/selection state. Native GUI regression
  geometry must be explicitly smaller than the long paragraph: default window
  geometry initially allowed the distant change to remain visible and was not a
  useful red oracle. Actual platform fonts plus an 800×480 window reproduce it.
- Skipping linked-image empty leaves requires keeping their outer link source
  wrapper and metadata. Expanding all image ranges to entire paragraphs was
  rejected by the existing non-monotone inline-image test; restricting the wrapper
  to an enclosing link preserves source accounting and complete linked-image copy.
- Three paired/reversed medium workloads pass. A two-sample TUI n/p timing signal
  did not reproduce in four alternating 60-event pairs (240 events per build,
  pooled median 0.832→0.581 ms); do not infer a general speedup. GUI/TUI reflow and
  active Find samples overlap. No full unchanged 132-case suite was repeated.
- GUI/TUI production stress ran 60.57/60.54 seconds and 17/54 cycles. Last GUI
  closure samples still vary and TUI ends at 52.45 MiB; these bounded observations
  are not fifteen-minute evidence or proof of leak freedom.
- Source GUI first paint was 127→126 ms in six pairs, but RSS grew 215.6→218.2 MiB.
  Three-way smaps isolates the 2.734375 MiB baseline-relative cost to file/executable
  pages (previous repair → final is 2.4375 MiB);
  heap/anonymous ownership stays stable and cold Source tests still create no
  Markdown projection/layout/syntax. ELF `.text` grows only 2 KiB, which does not
  explain the residency amount. No speculative linker experiment was justified.
- Final TUI Source smaps cost is 1.74–1.76 MiB in the two allocator bands, mainly
  file residency. Preserve the prior binary's 1.61–1.64 MiB result separately;
  mixed-band medians remain misleading. **Both final baseline-relative
  costs were accepted on 2026-10-05: GUI ~2.6 MiB startup / 2.734375 MiB smaps and TUI 1.74–1.76 MiB.**
- GUI readiness now requires formatted prose plus the tail (oracle version 2).
  A real Source-tail negative screenshot is rejected. Initial negative-probe End
  requests landed on a blank EOF row; Up exposes the tail. Retain failed fixture
  setup separately from actual oracle verification. XDamage remains first feedback.

Final checks: 720 workspace, 453 core, 82 GUI, 144 TUI tests per six themes,
feature-specific offline PR, Clippy and 30 Python tests pass. Native macOS,
physical display/GPU memory, external PR timing and fifteen-minute stress remain
unverified. Cross-pane context selection retains its existing clicked-pane
anchor behavior; that interaction was not redesigned in this repair.


## 2026-10-06 security, reliability and compiler audit

Final runtime checks: 805 Rust tests, 23 explicitly ignored, 35 Python
tests and warning-free Clippy. GUI/TUI production builds remain separate.

| Experiment | Outcome / decision |
| --- | --- |
| Retain annotations for every visited file | Repaired. Completed annotations now follow the existing document cache; hidden pending jobs lose ownership. Direct tests prove cached-request reuse, eviction and same-key cancellation. Native twelve-file navigation peak RSS was 501.18→460.48 MiB. |
| Active-side-only annotations | Superseded before final validation because retaining completed annotations for already-cached text documents preserves warm reuse without another cache. No runtime regression is attributed to the discarded design: the exploratory collector's OCR/view boundaries differed. |
| Same-source option flicker | Two paired 60-second runs, including reversed order, recorded zero sampled source-crop mismatches. First-damage timing remains separate from physical presentation and completion. |
| 4K image memory rise in the short suite | Longer paired cycles reached ~1.21 GiB peak and ~983 MiB settled in both builds. Last-half cycle-end and final-idle slopes were zero. No ongoing growth was reproduced within 60-second runs. |
| Merge CPU at three seconds | Fifteen-second idle sampling shows startup work settling to a last-five median of 0% with occasional ~1% buckets. No persistent busy-loop repair was justified. |
| Shipping dist versus release | Existing portable ThinLTO/one-CGU dist wins 58/60 engine trial/workload medians; equal-weight geometric time ratio 0.852. Final production GUI first paint improves 88→84 ms small, 153→144 ms generated100k, 211→206 ms 4K. These are workload-specific results. |
| Opt-level 2 / target-cpu=native | Did not consistently beat ordinary dist: geometric engine time ratios 1.028 / 1.046. No flags added. Native CPU code is unsuitable as a general portable default. |
| Infer compilation cost from cached release versus fresh dist | Rejected. Build cache history differs; recorded build durations are not a clean-build comparison. |
| Eager different100k Char benchmark takes 7.43 s | Not the GUI/TUI path: product adapters use bounded visible refinement with a 100k-token guard. No speculative engine rewrite. |
| Initial changed-line-only search timing signal | Three additional alternating pairs reverse the initial difference; pooled six-launch medians are166.80→166.91 ms. No reproducible slowdown established; raw samples retained. |
| Source stress ending RSS rises 13.03 MiB in one pair | Did not repeat after restoring identical options in reverse order: final candidate319.14 MiB versus baseline318.92 MiB. Keep the initial observation; do not call either sample leak proof. |
| Startup residency | Three smaps pairs show ~1.4 MiB GUI and ~1 MiB TUI cost, primarily mapped executable pages; matched anonymous memory is stable/slightly lower. Accepted as a known cost (2026-10-06). |
| OCR failures | Excluded: whole-window OCR missed visible Difference; changed-line ECHO read ECHD. Dedicated chrome/source crops and exact per-side context checks fix the collectors. Failed runs are not application failures. |

The full132-case/389-metric suite precedes only final GUI menu/cache refinements;
shared core, engine, image decoding and the final TUI executable are unchanged.
Final GUI tests and affected navigation/repaint/startup workflows were rerun.
No changes were made to the pre-existing onion-skin lookup table. PGO, fat LTO,
alternative allocators, native macOS, physical frame presentation and GPU memory
remain unverified; absolute security and crash freedom are not established.

## 2026-10-06 repeated user-flow and parity follow-up

Final
checks passed 832 workspace Rust tests three times, 124 actual GUI checks,
485 core checks without GUI, 180 TUI checks, 43 Python checks, 23 explicitly
ignored release tests and Clippy. Production GUI/TUI builds remain separate.

| Experiment | Outcome / decision |
| --- | --- |
| Repeat shared scenarios through real adapter input | Four seeded histories × eight fixtures × 64 steps per adapter, fixed replay and pending-work races passed. Literal text/file/history expectations remain independent of the diff engine. This is bounded exploration. |
| Make clipped Help and recovery reasons reachable | GUI Help wraps/scrolls and owns modal input; TUI F2 Details exposes full notices and selected disabled reasons. Six appearances and narrow/tiny layouts passed; no new dependency. |
| Run the full native matrix | Initial run exposed GUI Find consuming PageDown after the Save guard. Explicit plain-navigation forwarding repairs it while modified document actions remain blocked. Fresh final matrix passed all 132 cases / 389 metrics; all 60 generated-100k Find-scroll inputs produced feedback. The failed run remains in the evidence. |
| Repository readiness from whole terminal text | Rejected after independent review: prior left-pane/sidebar content could falsely satisfy readiness. Distinct fixtures and current right-pane/title checks pass both adapters three times; a negative test rejects the old oracle. Native workload version is now 3. |
| Timeout only the parent process | Rejected after an actual detached child survived. Existing process helper now supports cooperative interruption and bounded cleanup of owned descendants with PID birth checks. Real-child tests prove parent cleanup and child release. Linux cleanup is verified; native macOS is unverified. |
| Paired Find/Help lifecycle, before final GUI navigation repair | 60 seconds per build: GUI 469 cycles, Find/Help medians 11.64/12.80→11.52/12.24 ms; late RSS 211.46 versus 210.36–210.94 MiB. TUI 365 cycles, medians 41.29/41.07→41.31/41.07 ms including 40 ms helper quiet time; late RSS 11.74→11.72 MiB. Bounded first feedback and residency are not physical presentation or leak proof. |
| Initial GUI startup +2 ms signal | Final three alternating pairs overlap: baseline 90–95 ms, current 91–95 ms; medians 92→93 ms and RSS 186.5→186.4 MiB. No reproducible regression established. Keep the earlier 93→95 ms samples separately. TUI header was 1.64→1.35 ms, RSS 11.64→11.68 MiB. |
| Fifteen-minute shared model/worker stress | Passed 3,980 asserted cycles over 900.2 seconds, 400 RSS samples, 315.08 MiB observed peak and zero last-quarter versus preceding-quarter growth. Seventy-two source hashes confirm unchanged shared/workload code after the final GUI navigation repair. This workload excludes rendering and disk writes. |

The final full matrix used three trials and a ten-second shared stress case;
the separately retained 900-second run supplies sustained shared-core evidence.
Final production comparison/repository/merge/versions/Markdown/images controls
passed three rounds in both adapters. Markdown readiness still requires formatted
prose and completed tail after accepting the new F2 status prefix; metric
boundaries are unchanged. Physical-display timing, GPU memory, screen-reader
behavior, live GitHub failures and native macOS remain unverified.

## 2026-10-06 GUI/TUI performance snapshot

No runtime change. A full release matrix (132 cases / 389 metrics, no skips) and the new
`scripts/tui-startup-bench.py` measured both adapters on the same fixtures for the README
comparison.

| Observation | Status |
|---|---|
| TUI first content precedes GUI first paint on every fixture (6.7 vs 95 ms small, 26 vs 156 ms generated 100k, 185 vs 270 ms merge) | Measured; different boundaries (PTY text vs paint callback), documented in the report. |
| TUI 100k-line text pairs settle after ~1 s and the 100k-line merge after ~2 s of background CPU | Not investigated; cause untraced. Process CPU is 0.9–2.1 s per launch. Syntax parsing (321 ms per 100k-line file in components) and inline refinement are candidates. Profile first, then measure settle time and input feedback during settling before changing anything. |
| Many GUI navigation first-damage samples cluster at 15–17 ms on Xvfb, while merge accepts and file loads measure 4–8 ms | Observation only; cause not isolated. Physical-display latency remains unmeasured. |

## 2026-10-07 responsiveness and memory follow-up



| Experiment | Outcome |
|---|---|
| Find the 16 ms first repaint after a key (GPU display) | **Root cause found, not fixed in Lenz.** The key reaches Lenz in 0.3 ms and its render and paint take 2.1 ms. gpui at Zed `v1.22.0` draws again on key release but presents only on its X11 refresh tick. |
| Force `MESA_VK_WSI_PRESENT_MODE` to `fifo`, `mailbox` or `immediate` | Rejected: no change. Presentation mode is not the cause. |
| Patched gpui X11 client: request a frame for windows that just received key events | **Confirmed the cause.** First repaint 16.1 → 4.4 ms median. Experimental build only; where to fix it was left open. |
| `malloc_trim(0)` on the syntax worker after each parse | **Kept.** glibc kept a freed tree-sitter tree's pages in the worker arena. Settled RSS on the 100k-line pair: TUI 166 → 53 MiB, GUI 353 → 237 MiB; on the merge: TUI 282 → 96 MiB, GUI 471 → 288 MiB. Times unchanged within overlapping samples; the trim costs 1–4 ms on the worker. |
| `MALLOC_ARENA_MAX=1` or `glibc.malloc.trim_threshold` instead | Rejected: they recovered only 12–14 of about 100 MiB. |
| Soak: 1,122 GUI and 1,100–4,400 TUI file switches across 12 large files | No monotonic growth. The GUI is flat around 350 MiB; the TUI's 200-round quartiles are 172.8–173.5 MiB, drifting by 0.7 MiB, which this length cannot distinguish from allocator noise. |
| Large history (1.48M commits): profile before changing | **Implemented as demand-driven loading** (see the next row). Before it: Lenz settles at 221 MiB (about 150 B per commit); git's `--topo-order` walk peaks at 1.1 GB and costs about 7 s of CPU. Proposal: backpressure-driven loading, pending a decision on showing a lower-bound count. |
| Startup git work for `lenz-tui log` | **Fixed: start the log stream with the window.** The commit list waited for the sidebar inventory: a dozen sequential git queries (branches, remotes, tags, stashes, worktrees, conflict state, ref labels) on the repository worker. `lenz-tui log` took 15 ms to show content. The core now starts `git log` when the repository window opens; the inventory fills the sidebar and labels when it arrives and no longer restarts the stream (regression test `log_streams_before_the_sidebar_inventory_and_only_once`). First rows in 6 ms on the 500-commit fixture, DuckDB and Linux (previously 15, 34 and 138 ms). The trace showed 5 identical `rev-parse --show-toplevel` calls (an earlier note said 10, counting other `rev-parse` queries); they are no longer on the path to first content and were left alone. |
| Demand-driven history loading by backpressure | **Kept.** The reader stops 50,000 commits past the view, and git blocks on its pipe. Linux kernel, `lenz-tui log`: 231 → 27 MiB RSS and 8.5 → 0.5 s CPU. GUI: settled 10.8 → 0.68 s, heap 308 → 103 MiB plus 15 MiB of git heap. A first version asked for unlimited history while HEAD was still wanted and never paused; the demand is now rolling. |
| GUI log opening read the sidebar synchronously on the UI thread | **Fixed** by moving the reads to a background task. Kernel first paint 321–334 → 215–228 ms. Parity debt: the GUI ran its own opening sequence. **Resolved 2026-10-07:** it opens through `RepositorySession::open_loaded` and polls the session; log first paint (500 commits 111 → 110 ms, DuckDB 138 → 140, kernel 219 → 221) and settle (DuckDB 431 → 431 ms, kernel 674 → 672) unchanged (before, after, settle before, after). Sidebar sources on the session as well: 500 commits 111 → 109 ms, kernel 219 → 219 ms (before, after). With every repository workflow on the session, log key response is unchanged: selection 8.85 → 8.73 ms, up 8.71 → 8.45 ms, 60 samples each (before, after). |
| Key response on Wayland (headless sway, `WAYLAND_DEBUG` key → commit) | **Not affected.** Lenz 3.3 ms. The 16 ms delay is gpui's X11 backend only; the fix goes upstream (patch) rather than into a fork. |
| Duplicate `git status --untracked-files=all` on repository open (about 65 ms each on the 96k-file kernel checkout) | **Rejected: no visible effect.** Queuing the Uncommitted-changes job before the sidebar inventory left the time until the local diff appears unchanged (about 40 ms in both builds, `lenz-tui log`). The remaining cost is about 130 ms of background git CPU per open on very large trees; passing one status result between the two jobs would need new parameters through `repo::worktrees`, `local_changes` and `list_git`. Revisit on slow or cold storage. |
| Syntax parsing on a pool of three threads instead of one | **Kept, trading peak memory for speed.** Highlighting completes sooner: GUI 100k pair settled 1,089 → 701 ms, merge 2,246 → 1,063 ms; TUI 934 → 504 ms and 2,061 → 915 ms. Concurrent parses raise the peak (TUI merge 271 → 642 MiB) and settled residency by 2–22 MiB. Soaks level off without monotonic growth (TUI 67–71 against 56–60 MiB after 3 s idle), and more memory for speed was accepted on that condition. |
| Difference image built on demand instead of kept (two 4096×4096 PNGs) | **Kept.** One fewer 64 MiB buffer in both frontends and no image built at load: GUI first paint 216 → 177 ms and RSS 768 → 704 MiB; GPU-display PSS 364 → 300 MiB; TUI 92 → 64 ms and 202 → 138 MiB. A serial build made the first switch to Difference 8 → ~37 ms; parallel row bands bring it to ~8 ms. Not done: the GUI's duplicate RGBA/BGRA Sides (128 MiB), which would move pixel ownership out of the core. |
| Synthetic `Expose` to Lenz's own X11 window after key and mouse-button input (gpui presents at once after `Expose`) | **Kept.** No gpui change. First repaint after a key: GPU display 16.3 → 4.7 ms; software display 25.4 → 13.2 ms (external damage probe), browsing 15.6 → 5.8 ms. The suite probe's software-display Page Down stays at ~15 ms: rasterisation at its larger default window (9.3 / 13.2 / 15.0 ms at 1024×640 / 1440×900 / 1536×1000). Merge View Page Down 15.7 → 9.7 ms. Wayland unchanged (3.5 ms). Relies on gpui presenting after any `Expose`: re-measure GPU key response after each gpui upgrade. A newer gpui does not fix the delay (Zed `main` has the same structure), and gpui has no public present call. |
| Changed words drawn in the theme's plain foreground instead of syntax colors (GUI) | **Kept; a readability fix with no measured cost.** On the dark theme, syntax colors on a changed-word fill measured 1.1–1.9:1 contrast (strings 1.1:1); the GUI's native repository check could not read such a word, and failed on every build. Changed words now use the plain foreground; the fill stays, and the terminal, which underlines changed words without a fill, is unchanged. Key response on the GPU display: Page Down 4.78 → 4.72 ms, next change 4.74 → 4.75 ms, 174 samples each (before, after). The native check also reads the pane three ways, since no single preparation reads all of its screens; the full native suite passes 3 rounds in both frontends. |

## 2026-10-08 security, reliability and performance audit



| Experiment | Outcome |
|---|---|
| Dense local token ids for every gap `compare_smart` diffs (gix sizes its tables by the largest id) | **Kept.** A file with many small changes was quadratic: 200k lines with 20k changed lines 0.63 → 0.15 s; 600k lines with 60k changed 0.88 s (3.5–3.9× per doubling before, 2.1–2.8× after). Hunks are identical by construction; the corpus and IntelliJ suites pass. |
| Count punctuation in `refine`'s token cap, as `merge::cost` does | **Kept.** A single line of 8M brackets passed the cap as "2 words"; the delimiter matcher then ran an LCS over it. |
| Compare each distinct pair of mapped Markdown spans once per projection | **Kept.** Hunks inside one multi-line image placeholder each compared the whole placeholder: O(hunks × placeholder) on the worker. |
| Binary search of a Markdown block's runs per TUI cell; word selection on the clicked line only | **Kept.** Linear scans of up to 48k runs per cell, and O(document) segmentation per double-click. |
| Conflict stages read through one batch stream with a bounded read (8000-byte head over `LARGE`, rest streamed past) | **Kept.** A multi-gigabyte conflicted binary no longer aborts on allocation. A first version used a second `--batch-check` process and cost +9–16% on `tui/merge/conflict-list-open`; the single-stream version replaced it (see the audit's benchmark table). |
| `GIT_TERMINAL_PROMPT=0` / `GIT_OPTIONAL_LOCKS=0` as env on every git spawn | **Rejected.** Any `Command::env` makes std copy the whole parent environment per spawn: 467–493 → 555–561 µs for `git --version` with 248 variables, no difference with a minimal environment. Replaced by the `--no-optional-locks` argument everywhere and the prompt guard on the PR fetch only. |
| Bidi-control replacement in both frontends, worker `catch_unwind` | Robustness changes; no measurable cost. |
| `overflow-checks = true` in release and dist | **Kept, user-approved tradeoff.** Engine benchmarks off/on back to back: word 34.4 → 35.0 ms, char 57.7 → 58.3 ms, merge 63.6 → 64.0 ms, tokio files 45.4 → 46.7 ms, split 17.2 → 17.3 ms; 0–5%, about 1.5% on average. A wrapped range in a save path now aborts with a report instead of writing wrong bytes. |
| Repository-config driver overrides (`git config --show-scope` once per directory) | **Kept.** 0.78 ms median per repository directory, once per process. |
| Cap combining marks at 30 per character when drawing | **Kept after measuring.** cosmic-text 0.19 shaping, 9,900 marks on one character: 7.5 ms a line, 381 ms a screenful of 50; at 30 marks 0.6 ms. Check cost on ordinary text 1.1 ns per character. |
| GUI rendered Markdown layout shaped on a background task with its own `WindowTextSystem` | **Kept.** Resize first feedback on the medium fixture 51.6 → 6.4 ms median (worst 171 → 8.7 ms), repaired collector, two interleaved rounds. The window's own text system was rejected for this: a background insert into its per-frame layout cache would shift the indices gpui replays layouts by. |
| Markdown GUI collector: switch presentation through the View group | **Fixed.** The menu reorganisation moved Source/Rendered under View; the old Home, Down, Return opened Search, so the OCR check failed on every binary. |
| GUI rendered Markdown: rebuild the layout only after the pane widths hold for 100 ms | **Kept.** Removes the per-frame 53–152 ms reshape during a resize drag; measured in the background-layout row. |

## 2026-10-08 settings file and last position



| Experiment | Outcome |
|---|---|
| Look up the repository root and saved place on every TUI open | **Rejected.** Two `git rev-parse` per `App::open`, including plain-file comparisons and each new view: first paint 2.9 → 6.6 ms, opening 12 views 0.5 → 10.2 ms. Now only eligible launches look, once. |
| Detect an operation in progress with `conflicts::list` (`git status`) before reading the place | **Rejected.** About 65 ms on a 96k-file checkout for every plain launch. Replaced by `git ls-files -u` after a valid place file is found. |
| Settings and place after the fix, quick profile | **No reproducible regression.** Engine metrics on untouched code agree within 1% across five runs. Interleaved A/B of the TUI binaries: first paint 2.64–3.13 ms baseline vs 2.76–3.63 ms branch, key feedback 0.36–0.77 vs 0.37–0.62 ms. |
| 4K onion blend between runs | **Inconclusive, not a code effect.** Unchanged code measured 21–40 ms on the baseline commit and 29–41 ms on the branch across runs; memory-bandwidth bound under background load. |
| GUI font metrics as functions of the configured size (about 195 uses) | **Kept.** A `OnceLock` read per call; GUI tests and the quick profile show no change at the default size. |

## 2026-10-08 toolbar, command palette and Focus

| Experiment | Outcome |
|---|---|
| Commit offset as a parallel `Vec<i16>` beside the log rows | **Rejected.** +6.4 MiB resident and +4.9% layout on a million commits. |
| Offset packed into the row's time (7 low bits, quarter hours) | **Kept.** Layout unchanged; +2.4 MiB remains from the larger parsed `Commit`. **Accepted by the maintainer** as the cost of showing commit times. |
| Toolbar slots, header switch and scope built every frame | **Kept.** First-damage after page-down / next-change matched the baseline in alternating runs (5.6–6.3 ms both). A first, sequential run suggested +30% and did not repeat. |
| Raised popup colours computed from the theme on each paint | **Kept.** Only while a popup is open; no measurable effect. |

## 2026-10-10 About logo in foot



| Experiment | Outcome |
|---|---|
| Precomputed indexed artwork, Sixel encoding only while About is visible, bounded Linux launcher detection for generic TERM | **Kept.** Detailed artwork in foot, no new dependency or capability-query wait. About opening adds 0.3–0.4 ms in repeated pairs; final Lenz RSS +0.08 MiB. Native Lenz/foot residency plateaus through repeated display. |
| Use `Terminal::clear` to remove Sixel pixels on dismissal | **Changed.** Its cursor-query round trip is unnecessary with a hidden cursor; clear the backend directly and repaint the previous frame. |
| First private repeat-open check | **Invalid harness.** Foot's terminal replies were not forwarded across the nested PTY. Corrected harness passes; no application change was inferred from that failure. |

## 2026-10-09 Markdown reading view, part 1

| Experiment | Outcome |
|---|---|
| Obsidian extras in the shared parser (highlights, tags, callouts, front matter, wiki links, footnotes) | **Kept.** About +1 ms on a 6–7 ms medium parse. |
| Reading style: proportional body font (Noto Sans here) two pixels larger, centred ~70-character column | **Accepted by the maintainer** (2026-10-09). Preview layout +29% (medium 233 → 300 ms, small 117 → 150 ms, background task). |
| Liberation Sans instead of Noto Sans as the body font | **Measured, not applied.** About −35 ms on medium: smaller OpenType tables make HarfBuzz plan compilation cheaper. |
| Reuse the shaped line for rows that need no wrapping | **Kept.** −10…15 ms on medium; the line was shaped twice, also in the baseline. |

## 2026-10-09 Markdown Inline view



| Experiment | Outcome |
|---|---|
| Inline arrangement (word diffs of changed blocks) on the projection worker for every Preview | **Kept.** Side by side unchanged (medium ~300 ms, small ~150 ms). |
| Inline layout in one pane | **Kept.** 5–8% faster than side by side: one pane shaped. |
| Remembering an Inline scroll position from the first new-side text instead of the top row | **Rejected.** Was a guess at an End failure that turned out to be the workload's leftover caret; no measurable or behavioural effect. |

## 2026-10-09 Markdown reading view in the TUI



| Experiment | Outcome |
|---|---|
| Rows carry document side, change mark and an inline label tuple | **Changed.** +0.5–1 MiB peak on the medium fixture (rows 72 → 88 bytes). |
| Boxed row label | **Inconclusive alone.** Peak still ~+1 MiB. |
| `u32` block and marker indices on rows | **Kept.** Rows back to 72 bytes; peak 39.4–40.3 vs 40.8–41.0 MiB baseline, settled lower; timing unchanged. |
| Refine every change block while the Preview is shown (batches of 256 on the worker), not only near the Source viewport | **Kept.** Word outlines everywhere; medium Preview completion unchanged (275–283 ms warm). |

## 2026-10-09 TUI toolbar and palette catch-up



| Experiment | Outcome |
|---|---|
| Toolbar items, their disabled reasons and the pane-header switch rebuilt from the shared catalog every frame | **Kept.** Frame cost unchanged (image ~224–243 µs, text ~83–89 µs per full frame, both builds); startup, key and resize first bytes and memory within noise. |
| images-4k first content +3 ms in the interleaved suite | **Inconclusive → not reproduced.** Every round ran the baseline first; 20 alternating launches put the branch 3 ms faster. |

## 2026-10-09 TUI parity gaps



| Experiment | Outcome |
|---|---|
| Copy the comparison's cells every frame in repository views, for the 200 ms swap hold | **Rejected.** About 7 µs on a ~150 µs frame (≈5%). |
| Copy them only while a Change Set is pending | **Kept.** The event loop always draws after the click that asks for one; zero cost otherwise. |
| Hold the history log's last visible rows (one `Vec<Line>` clone per frame), like the GUI's held frame | **Kept.** Startup, key and Preview timings unchanged. |
| Markdown images decoded in the background, drawn as half-block cells or kitty placeholders | **Kept.** No Preview timing change; images within the shared `IMAGE_BYTES` budget. |
| Apparent +1 MiB resident at startup in every case | **Inconclusive → not reproduced.** File-backed executable pages of one copy of the binary; re-copied, the gap was +0.1–0.3 MiB on small cases. |

## 2026-10-09 Overview and Focus (phases 1–2)



| Experiment | Outcome |
|---|---|
| Focus toolbar (more buttons, a per-frame whitespace label), Selection Title per frame, PR tabs in the header switch | **Kept.** Startup and settle unchanged; interaction differences in the first run did not repeat in three alternating rounds. |
| Apparent +1.6 MiB resident at startup in every case | **Inconclusive, not attributed.** Stable per binary but non-monotonic across bisect builds (core-only commit +2.7 MiB, toolbar commit +0.6 MiB); matches the earlier file-backed-page effect. |

## 2026-10-10 GUI/TUI toolbar and Source highlighting parity



| Experiment | Outcome |
|---|---|
| Shared edit-toolbar visibility; TUI changed-word backgrounds and merge-state colors | **Kept.** Three registered-suite repetitions and native GUI/TUI workflow replay pass. Small GUI first paint 106 → 107 ms; 100k-line TUI first content 28.21 → 28.56 ms in eleven alternating pairs. |
| Initial TUI +2.86 ms and small GUI +5 ms opening differences | **Not reproduced.** Eleven-pair repeats overlap; GUI repeat mean is lower. No startup slowdown inferred from the initial batches. |
| Source scrolling memory stress | **Stable sampled RSS.** 30.31 / 30.38 seconds, 369 / 370 cycles, 55.86 / 55.99 MiB. Candidate timing overlaps the tail of correctness checks; no isolated stress timing or long-term leak claim. |
| Merge peak median +5 MiB | **Inconclusive as regression evidence.** Five-pair sample ranges overlap substantially (554.52–593.55 vs 561.73–594.20 MiB); no retained allocation added. |
| Broad terminal benchmark resize repaint | **Inconclusive harness.** Fails on the saved baseline. Query-aware paired opening/input probes and native resize/dismissal checks pass. |
| Native collector's old menu/Markdown labels, PR alias and repository oracle | **Corrected.** Workload version 4 targets current controls, named headers and completed fixture content; Python collector tests and all six production scenarios pass in both adapters. |

## 2026-10-10 Conflict-only icon toolbar follow-up



| Experiment | Outcome |
|---|---|
| Remove ordinary comparison edit buttons, retain commands/shortcuts; gear-only Diff and conflict edit icons | **Kept.** Core/adapter CI, six-theme icon mouse targets and narrow constructor checks pass. |
| Floppy-disk emoji for Save | **Replaced.** GUI merge one-second RSS/peak 298.4/816.0 → 303.0/820.1 MiB in five alternating pairs, consistent with added font work. |
| Monochrome arrow-to-bar Save icon | **Kept.** GUI merge first paint 271 → 268 ms, one-second RSS 301.4 → 298.8 MiB, peak 816.0 → 816.5 MiB in five alternating pairs. |
| Early TUI 100k-line RSS difference | **Not retained growth.** The 0.5-second boundary catches transient syntax allocations; four three-second exact smaps pairs give RSS 56.061 → 55.980 MiB and anonymous PSS 44.855 → 44.840 MiB. |

## 2026-10-11 First native macOS run of the current build

Apple M2, macOS 27.0, `perf-suite.py --profile full --native`, at 5c50c62 plus the Mac test fixes. The suite passed; the startup merge case now passes a common base.

| Experiment | Outcome |
|---|---|
| TUI RSS on 100k-line inputs about 5× Linux (generated 253 vs 55 MiB, unrelated 394 vs 56, merge 578 vs 115; small and 4K images match) | **Freed-memory hypothesis rejected.** `malloc_zone_pressure_relief(NULL, 0)` after each syntax parse (the macOS counterpart of the glibc `malloc_trim` call) left startup RSS unchanged in both frontends (GUI generated 100k 345.6 → 346.1 MiB, merge 680.9 → 677.0; TUI resident-100k 107.1 → 107.2), so the memory is live. TUI merge cancel-settled fell 140.1 → 88.9 MiB, but GUI merge first paint rose 385 → 413 ms, load 162 → 185 ms and CPU 2.60 → 2.83 s, and TUI PTY workflows 6–17%, in one three-sample run two hours apart. Reverted. The TUI tabs bench's initial RSS is 72 MiB on macOS vs 7.4 MiB on Linux; unexplained. |
| TUI repository workflows 5–17× Linux (repository-ready 220 vs 24 ms, blame-ready 50 vs 2.9, search-stress 36 vs 6.5) | **Known macOS cost, after 0.0.1.** Each git process costs ~20 ms there and these workflows start several; batching inventory queries or a persistent git process would address it. |
| TUI comparison first frame ~30 ms on macOS vs ~6 ms for repository/About and on Linux | **Kept.** Second Mac run: GUI load 20–38% lower in every file case (small 47 → 29 ms) and first paint 6–8% lower (small 251 → 235 ms); TUI first frame not re-measured. strace on Linux: four git processes ran before the first frame of a file pair (`--version`, the config trust query, one `rev-parse` per argument), ~1.5 ms each here and ~20 ms each on macOS (`git --version` measured 21 ms there). The two `rev-parse` calls are one `cat-file --batch-check` now, and none of the four runs when no directory above holds a repository: outside a repository the Linux first frame came 1.6 ms after exec with no git process. Inside one, three remain. Drawing a frame before parsing would remove them all; not done for 0.0.1. |

## 2026-10-11 macOS memory: what `ps` counted, what libmalloc keeps, and the syntax arena

Apple M2, macOS 27.0 (26A428), release builds: `f361ada` (before) and branch `mac-memory` (after), fixtures from `target/perf/mac-full-2`. Tools: `footprint`, `vmmap`, `heap`, `malloc_history` under `MallocStackLogging=full`, `proc_pid_rusage`, plus C microbenchmarks against libmalloc. Raw tool output, microbenchmark sources, paired samples and executable hashes are under `target/perf/mac-memory/` (`inspect/`, `ab/`, `bin/SHA256SUMS`). The Linux reference is the same fixture and source built for linux/arm64 in Docker (`rust:bookworm`, glibc 2.36) on the same machine, read from `/proc`; the earlier Linux desktop numbers in the previous section were not re-measured.

### Where the 254 MiB went (TUI, generated 100k pair, 3 s after first frame, before)

| Component | MiB | How it was identified |
|---|---:|---|
| Live heap: 186k blocks, 39.6 MB (185k per-line `String`s of the two documents 8 MB; texts, line indexes and diff vectors ~30 MB in blocks ≥ 200 KB) | 38 | `heap`, `malloc_history -callTree` |
| Dirty pages in partially used small regions (fragmentation around the live blocks) | 23 | `vmmap` |
| Dirty pages in 46 *empty* 4 MiB small regions: freed, about half of each region still dirty, never reclaimed (unchanged at 20 s) | 76 | `vmmap` per region, `footprint` |
| Freed pages libmalloc marked reusable (`MADV_FREE_REUSABLE`): counted by `ps`/resident size, excluded from the footprint, never reclaimed without memory pressure | 123 | `footprint` "Reclaimable" |
| Clean binary text and shared libraries | 5 | `footprint` |

`phys_footprint` 122 MB, lifetime peak 256 MB, `ps` RSS 254 MiB. The peak (`malloc_history -highWaterMark`) was 244 MB, 175 MB of it tree-sitter nodes (`ts_parser__reduce`, 1.97 M allocations) and 24 MB other malloc. The same binary's live data is the shared core's: nothing differs structurally from Linux.

Linux, same fixture (Docker arm64): VmRSS 57.2 MiB = 46.6 MiB anonymous heap in glibc's worker arenas after `malloc_trim` (`[heap]` itself 0.1 MiB) + 8.9 MiB binary text + 1.6 MiB libraries; VmHWM 266 MiB. Same peak, same live set; the gap is entirely what each allocator keeps after the parse trees are dropped.

### Experiments

| Experiment | Outcome |
|---|---|
| Is the metric comparable? All collectors read `ps -o rss` on macOS (`perf_support.process_stats`, the in-process `perf::rss_mib`). C microbench, 2 M × 88 B malloc/free: `ps` RSS 186 MiB after the frees, `phys_footprint` 2.0 MiB; a freed 64 MiB buffer stays at 65 MiB in both (large cache). | **Kept: workload version 5.** macOS `rss_mib` is now `proc_pid_rusage` `ri_phys_footprint`, `peak_rss_mib` the kernel's lifetime maximum (previously sampled), `resident_mib` keeps the `ps` value. In-process probes use the same definition through `lenz::memory_probe` (Linux: VmRSS). |
| `tui/tabs/initial` 72 MiB on macOS vs 7.4 on Linux | **Explained and kept: sample moved.** The test freed its 64 MiB 4K pixel buffer just before sampling; libmalloc's large cache keeps it charged. `MallocLargeCache=0`: initial 72.2 → 8.2 MiB, settled after dropping all views 525 → 13.4 MiB (two runs each, identical). The cache is set at libmalloc initialisation (environment only) and `malloc_zone_pressure_relief` returns 0 against it, so there is no in-process control; the sample now precedes the fixtures. Image workloads on macOS therefore still carry freed 64 MiB buffers in the footprint until memory pressure. |
| `malloc_zone_pressure_relief` (previous section, not retried in Lenz) | **Rejection stands, for a different reason.** In every microbench it returned 0 and changed neither the resident size nor the footprint: libmalloc had already marked the freed small pages reusable, and nothing it does reaches the dirty halves of the empty regions or the large cache. |
| Dedicated malloc zone for tree-sitter via `ts_set_allocator`: persistent zone, or one zone per parse destroyed afterwards | **Rejected.** Footprint after two concurrent 100k parses: default 92.5 MiB, persistent zone 94.0, per-parse zone destroyed 91.8 (all 2 M tree-sitter allocations counted inside the zones). `vmmap` showed the destroyed zones' empty regions under the default zone afterwards; the C check of `malloc_destroy_zone` (215 → 2 MiB) did not carry over. |
| libmalloc retention microbenches: free order (forward, reverse, shuffled, strided), cross-thread free, concurrent threads, interleaved survivors, mixed sizes with realloc | **Inconclusive on the mechanism.** Uniform patterns return everything (footprint ~2 MiB); freeing on another thread kept 17 MiB; interleaved survivors pin every page as expected. None reproduced the half-dirty empty regions, which only tree-sitter's teardown produced: deterministic, 83–89 MB after the *first* teardown of two concurrent parses, 24–40 MB after a second round reused the pages. Sequential parses: 15.6 MiB after two parses but 760 vs 450 ms, a latency tradeoff not taken. |
| Per-parse syntax arena: tree-sitter allocates from an anonymous mapping that `parse_now` unmaps when the parse ends (`crates/app/src/syntax_arena.rs`). Measured first: one parse mallocs 89 MiB with 88.5 MiB maximum live and 3 MiB of reallocs, 2 M frees all at teardown, so a bump allocator adds no peak. | **Kept.** Five alternating pairs per case on an idle machine, release builds, physical footprint 3 s after the first frame or paint (run 2; run 1 agreed within 1% on every memory figure and is kept as `ab-run1/`). TUI, `tui-startup-bench.py` (`resident_mib` is the old `ps` number for reference): generated 100k 121.8 → 57.9 MiB (resident 255 → 68), unrelated 100k 134.2 → 95.5 (396 → 106), merge 321.0 → 96.4 (580 → 107); small, images, repository, About unchanged (±1%). Peaks unchanged (generated 256 → 264, +2.9%: the mapping is fresh pages where libmalloc reused some). First frame and first content within ±2% on every case. GUI, `startup-bench.py --native`: generated 100k 209.2 → 139.0 MiB (resident 346 → 155), unrelated 223.4 → 177.9, merge 411.0 → 202.4, small 88.6 → 88.2, 4K images 457.8 → 457.6; load 47–50 ms either way; first paint generated 289 → 287 ms, unrelated 342 → 339, images 320 → 312, small 220 → 232 (+5.5%, no syntax parse in that case) and merge 382 → 392 (+2.6%) — see the ten-pair recheck below. In-process: `syntax/parse` 278.7 → 270.0 ms (−3%), `syntax/query-compile` unchanged, `tui/resident-100k` 107.2 → 100.4 (now a footprint). The removed `ps` fork inside timed sections shows as `tui/tabs/close-ready` 5.96 → 0.50 ms and `open-4k-ready` 65.4 → 55.9; those were measurement cost, not rendering. Linux (Docker arm64, glibc) generated 100k TUI, three runs each: 54.7 → 54.7 MiB at 3 s (glibc already returned the tree through `malloc_trim`; the call stays), VmHWM 267 → 276 MiB (+3%, the mapping is fresh pages). Unrelated pair 53.5 → 53.5 MiB (peak 408 → 405), merge 120.8 → 113.1 (peak 640 → 633), two runs each. After the change the Mac merge (96 MiB) sits below Linux and the generated pair matches it (58 vs 55); the unrelated pair still reads 95 MiB on macOS against 53 on Linux, so some other transient burst of that case (its word and character refinement) is retained the same way and is not yet attributed. |

Ten-pair recheck of the two GUI cases whose first paint moved in the five-pair run: small 222.5 → 215.5 ms (−3%, samples 211–230 vs 213–233) and merge 379.5 → 379.5 ms (375–389 either way), memory 414 → 185 MiB; the earlier +5.5% and +2.6% were noise, as the small case (no syntax parse) suggested.

Still open:

- **The libmalloc mechanism** that leaves about half of a first teardown's pages dirty is unexplained; it is now avoided for parse trees, not understood. Any other large transient burst (a huge diff's working arrays, decoded images) can still show it. The microbenchmark sources are kept under `target/perf/mac-memory/inspect/` for the next attempt.
- **libmalloc's large cache** keeps freed buffers of several MiB charged until memory pressure: the terminal independent-views test settles at 518 MiB after every 4K view is dropped (13 MiB with `MallocLargeCache=0`), and the GUI's 4K image cases carry it too. The only control is the environment at process start; a re-exec is not worth it for a cache the kernel reclaims under pressure. Image memory on macOS therefore compares to Linux only at the peak.
- **Peaks are unchanged**: tree-sitter still builds ~90 MiB per 100k-line side and the three-way merge parses three sides at once (TUI peak 588 MiB). Smaller peaks need a different parse strategy (ranges, or a leaner tree), which changes highlighting semantics and is not for 0.0.1.
- Measured on one Mac; the Linux desktop in the previous section was not re-measured, and the Linux reference here is an arm64 container on the same machine.
- `scripts/ci-local.sh` on this Mac: the GUI adapter step failed eight `gui_` tests on the unchanged tree. Fixed in the follow-up commit: five native-text tests built gpui's macOS platform on a libtest worker thread ("Mac platform not created on main thread"), and three compared `/var` temp paths with git's `/private/var` toplevel. The gate now finds Xcode's Metal compiler when `xcode-select` points at the Command Line Tools.

## 2026-10-11 macOS: a cancelled `git mergetool` merge exited 0

The release blocker from the first Mac run reproduced on the current build: with
`mergetool.lenz.trustExitCode true`, quitting an un-Done Merge View (⌘Q, or the quit request
`NSRunningApplication.terminate` sends) made `git mergetool` report success and stage `f.txt` with its
conflict markers (no unmerged index entries left, a `.orig` beside it). A plain `lenz merge` quit
exited 0; the README promises 1.

Cause: gpui's macOS `quit` sends `[NSApp terminate:]`, and AppKit ends the process with `exit(0)`
after `applicationWillTerminate`, so `gui::run` never returns and its
`ExitCode::from(merge_view::exit_code())` was unreachable there. The tracked code was right (1 until
Done); only the process exit lost it. Linux returns from the event loop, so it was unaffected. The
same unreachable tail held the repository chooser's restart inside the chosen repository, so on
macOS choosing a repository would close Lenz instead (same mechanism; not driven by hand, since
choosing needs input automation this Mac does not grant).

Fix: `gui::finish_on_terminate` registers a quit observer on macOS whose future runs the launch's
finishing step (the merge code, or the chooser's re-exec) and exits with its code. gpui collects quit
observers' futures, tears down the windows and then awaits them, so this runs exactly where AppKit
would have exited; the PR-refs cleanup observer does its work synchronously before any future is
polled. `scripts/check-mergetool-cancel.py` drives a real `git mergetool` in a disposable repository
and quits Lenz as ⌘Q does: it failed before the fix (exit 0, path staged) and passes after (exit 1,
path still unmerged, file unchanged). Also checked after the fix: plain `lenz merge` quit exits 1, a
comparison quit 0, the chooser quit with nothing chosen 0. Not automated: Done (needs input), and a
chooser choice. The TUI returns its code from `main` and has a unit test for the cancelled merge.

## 2026-10-11 macOS: libmalloc's large cache and a mapped-block allocator

After the syntax arena the unrelated 100k TUI pair still read 95 MiB on macOS against 53 on Linux.
`vmmap` 3 s after the first frame: 42 MB live in all zones, about 20 MB fragmentation in partly used
small regions, and 31 MB in four "Malloc Large (empty)" regions, freed multi-MiB working buffers.
The independent-views test's 518 MiB after closing every 4K view is the same cache. A C check: four
freed blocks totalling 104 MiB stayed fully in the footprint, and `malloc_zone_pressure_relief`
(`NULL` or the default zone) released 0 bytes. Only `MallocLargeCache=0` at process start controls it.

`crates/app/src/large_blocks.rs` is a macOS global allocator in both binaries and the core test
binary: blocks of 1 MiB and up with up to page alignment are `mmap`ed and `munmap`ed on free; at most
four freed mappings and 16 MiB in total are kept for reuse by an allocation of the same page-rounded
size; everything else goes to the system allocator. Zeroed large allocations take fresh pages untouched.
All workspace tests run on it (662 core, 257 TUI). Paired release runs, five alternating pairs each,
against `2e85730`; raw samples and executable hashes in `target/perf/large-blocks/`:

| Experiment | Outcome |
|---|---|
| Re-exec with `MallocLargeCache=0` | **Not tried.** Every launch would pay an exec and dyld again. |
| 1 MiB threshold, no reuse | **Superseded.** TUI unrelated 95.4 → 59.3 MiB, generated 57.6 → 47.8, merge 96.1 → 81.6; four 4K views closed 518 → 5.7; `tui/resident-100k` 100 → 53; merge cancel settled 106 → 44. But merge accept/undo/redo 35.2 → 43.5 ms (+23%), merge first content +10%, closing a 4K view 0.5 → 2.0 ms, sustained cycle +4.4%, 4K decode +4.3%. |
| 4 MiB and 16 MiB thresholds | **Rejected.** Merge timing near baseline (+3%, +2%) and the 4K win kept, but TUI startup memory rose above baseline at 4 MiB (generated 57.6 → 71.8, merge 96.3 → 115.4): a `Vec` growing across the threshold leaves its old block in libmalloc's cache. |
| 1 MiB threshold with a 16 MiB, four-slot same-size reuse cache | **Kept (user-approved tradeoff).** Startup footprint: TUI unrelated 95.4 → 66.7 MiB, merge 96.6 → 92.0, generated 57.7 → 56.2; GUI unrelated 179.7 → 152.2, merge 204.2 → 194.2, generated 141.0 → 142.7; small and images unchanged. First paint and first content within ±3% everywhere; GUI merge load 171 → 177 ms. In process: four 4K views closed 518 → 6 MiB, merge cancel settled 106 → 60, merge stress 120 → 89. Costs: merge accept/undo/redo 35.7 → 40.9 ms (+15%), closing a 4K view 0.5 → 2.2 ms (unmapping 64 MiB). Peaks unchanged except TUI merge +2.6%. The committed build (two `expect`s replaced, so not byte-identical) was rechecked with ten more pairs: `tui/merge/open-100k` 159.6 → 166.9 ms (+4.6%; ranges 155–174 vs 162–173), accept/undo/redo 35.7 → 41.3 ms (+16%; 35.2–35.8 vs 40.9–41.7), merge cancel settled 106 → 52 MiB, 4K views closed 518 → 6 MiB. |

Remaining merge cost is most likely `Vec` growth across 1 MiB (each doubling maps and copies where
libmalloc can grow in place); growing a mapping in place with a fixed `mach_vm_allocate` after it is
the untried next step. The first paired run hung for 30 minutes on a baseline TUI stuck exiting
(`E`): a macOS process exiting with unread terminal output waits for it to drain, and the probe had
stopped reading. `TerminalProbe.close` now drains while it waits and hangs up the terminal as a last
resort; a collector test reproduces the hang.

## 2026-10-12 lenz-content extraction (code-reader ticket 01)

Decoding, line offsets, highlighting and the syntax arena moved unchanged into `crates/content`; `lenz` re-exports them. Full suite, Linux (Ryzen AI MAX+ 395), `--ui off` for lack of Xvfb, before = `main` 12b682b in a separate worktree, after = the extraction, run back to back on an idle machine (`target/perf/01-before`, `target/perf/01-after`, workload version 5 for both).

| Finding | Evidence |
|---|---|
| No change from the extraction itself | `syntax/parse` +1.8%, `syntax/query-compile` +3.5% (three samples each, overlapping), `syntax/retained` identical, `syntax/visible-spans` −19% (microsecond-scale), `decode/*` −2%; TUI PTY peak/settled RSS +0.5%/+0.2%; stress RSS timeline −0.5%. n=3 TUI workflow rows moved ±5–28% with overlapping ranges, and `tui/merge/cancel-queued-200` was already bimodal before (0.4, 0.6, 164 ms). |
| `image/4k-blend` +24% is code layout, not code | Six alternating runs of each test binary reproduced it (before 20–28 ms, after 29–35 ms). `perf stat`: identical instructions (4.654 G) and page faults (~4.4 k). `images::blend` is byte-identical in both binaries but starts at 16 vs 48 mod 64. Rebuilt both with `-C llvm-args=-align-loops=64`: before 28.4–34.4 ms, after 29.7–33.7 ms. The baseline's layout was the fortunate one; any unrelated change can move it. |
| Small TUI test-binary residency shift | `tui/about/reopen-resident` 10.9 → 11.4 MiB in six alternating runs (≈ +0.3–0.4 MiB); the shipping TUI's PTY RSS did not move beyond noise. Not attributed further. |

Unverified: GUI startup and interaction workloads (no private Xvfb installed), native macOS. On macOS the moved syntax/decode workloads and the arena memory test now run on the system allocator rather than `LargeBlocks` (workload version 6).

## 2026-10-12 shared themes (code-reader ticket 02)

Themes moved into `lenz-content` with caller-supplied paths and overrides; no theme logic changed. Full suite, Linux, `--ui off` (no Xvfb), before = ticket 01's tip 81b90d6 in a separate worktree, after = the extraction, back to back (`target/perf/02-before`, `target/perf/02-after`, workload version 6).

| Finding | Evidence |
|---|---|
| No change from the extraction | TUI render, chrome, About, PTY and memory rows within noise or overlapping; theme colour helpers now cross a crate boundary without LTO in `release`, with no visible render cost. Stripped `lenz` +136 KB (67.03 MB). |
| `image/4k-blend` +37% and `search_session.paint_100_visible_lines_linear` +10% are layout | Both in untouched code. Blend: identical instructions (4.654 G); `images::blend` starts at 0 mod 64 before, 48 after (48 was also the slow layout in ticket 01). With `-C llvm-args=-align-loops=64` in both trees: blend 29.1–34.9 vs 29.5–35.0 ms, paint 0.320–0.374 vs 0.336–0.367 ms. The same before-binary source measured slow in ticket 01's after-run from a different directory, so the 4K blend median is layout-bound to ±30% on this machine. |

Unverified: GUI startup, interaction and theme-switch timings (no private Xvfb); native macOS.

## 2026-10-12 shared pane presentation (code-reader ticket 03)

`lenz-gpui` takes the comparison pane's numbered-row painting, line shaping and pointer hit-testing; the
display transforms (tabs, hidden characters, mark cap, long-line cut) move to `lenz-content::display`. Full
suite, Linux, `--ui off` (no Xvfb), before = 818a972 in a separate worktree, after = 2ffea44 and again at
15d9514 (one scan for a drawn line's cut instead of two), back to back (`target/perf/03-before`, `03-after`,
`03-after-final`).

| Finding | Evidence |
|---|---|
| No change from the extraction in the measured workloads | Every row overlaps the baseline or moved within noise. The GUI paint path itself is not measured without a private display. Stripped `lenz` 67.04 MB (+5 KB). |
| `syntax/parse` +4% in the first after-run is layout | `lenz-content`'s test binary: 279–285 vs 291–303 ms in five alternating runs, identical instructions (4.840 G), L1 i-cache misses 2.0–2.2 M → 6.0 M. Tree-sitter's code moved when `display.rs` was linked in; no parse code changed. |
| `merge/open` +12% (n=3) in the final run is noise | Six alternating runs: 149.1–161.6 ms before, 149.6–162.7 after, same instruction counts. |

Unverified: GUI startup, interaction and paint timings (no private Xvfb); native macOS.

## 2026-10-13 shared fuzzy ranking (code-reader ticket 04)

`navigator::score`/`rank` moved to `lenz-content::fuzzy` unchanged (plus `score_lowered` for the reader's
pre-lowercased inventory). Full suite, Linux, `--ui off`, compared with `03-after-final` (identical prior code,
measured the day before on the same machine): every row overlaps except `syntax/parse` +4.4% (282–292 → 295–302
ms) and `syntax/visible-spans` −6.6%, both in `lenz-content`'s test binary. This is the layout effect recorded for
ticket 03: adding a module to that crate moves tree-sitter's code, with identical instruction counts. The shipped
binaries are linked differently and their workloads did not move. Unverified: GUI timings (no Xvfb), native
macOS.

## 2026-10-13 compact declarations (code-reader ticket 08)

`lenz-content::highlight` gained `declarations_while`, run on the existing three syntax workers (the worker
loop is now generic over the job's result; panic, reply and `release_freed_memory` behaviour unchanged). Only
compact records leave the arena; the ignored test
`declaration_extraction_returns_the_trees_memory_to_the_system` runs alone in `ci-local.sh`. Full suite,
Linux, `--ui off`, `08-after` against `04-after`: interquartile-separated shifts were `tui/tabs/close-ready`
0.94 → 6.7 ms and `tui/tabs/open-4k-ready` 52.5 → 66.9 ms (image tabs; no syntax code involved). Interleaved
reruns of the `d5971fb` and `e1a2b2b` test binaries gave the same values for both (open 62–72, close 6.0–8.2
ms), so this is machine state since the baseline, not the change; a later baseline should be re-recorded.
`image/4k-blend` +72%, `search_session.paint_100_visible_lines_linear` +11% and
`tui/pty/workflow/repository-ready` +10% stay within the `03-before` range (the layout effect recorded for
ticket 03). The follow-up `f5b7bc5` touches only declaration queries, which no suite workload runs.
Unverified: GUI timings (no Xvfb), native macOS.

## 2026-10-13 shared single-image decoding (code-reader ticket 17)

Single-image detection/decoding moved to `lenz-content::image` and texture/checkerboard/atlas-release helpers
to `lenz-gpui::image`; Lenz keeps comparisons. `image`/`resvg` were already built (same versions, a subset of
GPUI's features), so nothing new compiles. A fresh baseline (`17-before`, same machine, the earlier image-tab
drift gone) against `17-after`: improvements or overlapping rows, except three interquartile-separated shifts,
each checked with the previous and new release binaries interleaved: `image/4k-difference-image` +15% (n=3;
interleaved 3.76–4.45 vs 3.89–4.54 ms: noise); `tui/scroll-render-100k` +6% (interleaved 0.186–0.191 vs
0.191–0.198 ms, a consistent ~2%, with identical instruction counts, 3.512e9 both: code layout, as recorded for
ticket 03); `tui/pty/settled-rss` +5% (bimodal ~50.6 or ~53.3 MiB in both builds: high in 6 of 10 runs before
and 5 of 10 after: noise). The follow-up sharing `looks_binary`/`detect_file` changes no measured path.
Unverified: GUI timings and atlas memory (no Xvfb), native macOS.

## 2026-10-13 shared single-document Markdown (code-reader ticket 18)

Single-document parsing, source mapping, front matter, local-image validation/decoding and code-block syntax
moved to `lenz-content::markdown` (`parse` now refuses a source over `bytes_per_side` itself, as `parse_pair`
already did per side); callout/removed/highlight colours to `lenz-content::theme`; `ReadingStyle`, block
shaping and indentation to `lenz-gpui::markdown`. Lenz keeps pairing, source diffs, projection and Preview
state. `pulldown-cmark` was already built for the app (same version, now without default features in
`lenz-content`), so nothing new compiles. Full release suite, two candidate runs (`18-after`, `18-after-2`)
against `17-after`: the rows over +10% differ between the two runs (`blame/HEAD~100/first`, `image/4k-diff`,
`syntax/visible-spans` in the first; `tui/tabs/*`, `tui/images/decode-diff-4k`, `search_session` first batch in
the second), and each overlaps an earlier run's samples: `image/4k-diff` 35.8–42.4 ms vs 33.9–41.2 ms;
`blame/HEAD~100/first` 2.19–3.02 vs 2.35–3.73 ms; `tui/tabs/close-ready` is bimodal (~1 or ~7 ms, all ~7 at
`17-before`, mixed at `18-after-2`). No code on those paths changed: noise. Unverified: GUI timings (no Xvfb),
native macOS.

## 2026-10-14 find options and the shared matcher

Find, merge find and Find in Change Set moved from the hand-written case-insensitive matcher to
`lenz-content::find::Matcher` (`regex`, already linked through tree-sitter) for case / whole-word / regex
options. In-process find over 100k lines, release, interleaved with the base build (`bc8f5bf`):

| Experiment | Outcome |
|---|---|
| Every query through `Regex::find_iter`, one iterator chain (`Filter<Chain<Flatten…>>`) per line | **Rejected.** `search/find/needle` 4.8 → 8.4 ms, `e` 8.2 → 14.6, `absent` 3.9 → 2.4 (three alternating rounds agreed). The per-line call and the adapter chain dominated short lines with many matches. |
| ASCII literal queries on ASCII lines compared byte-wise (the old algorithm), in the same chain | **Rejected.** 6.2 / 6.0 / 11.3 ms: the chain (`Filter`/`Chain`/`Flatten` over `Option`s) was ~30% of the profile. |
| One concrete `Ranges` iterator (enum: ASCII bytes, expression, done) still holding `regex::Matches` | **Rejected.** 5.7 / 4.7 / 9.5 ms: the large `Matches` value made every line's iterator big. |
| Same iterator, expression searched with `find_at` from a stored offset | **Kept.** 4.9–5.0 / 3.9 / 8.4 ms vs base 4.8 / 3.9 / 8.2 (noise); regex `search/find-regex` 11.0 ms (new). A test pins the byte path to the expression's matches. |
| Literal queries compile their expression only for the first non-ASCII line (a Unicode case-insensitive expression takes about 1 ms to build) | **Kept** (no measurable effect on the suite's workloads, which are ASCII; saves the build per keystroke and per Change Set start). |

Full suite (`--profile full`, three runs) against the base: `search_session.complete_96k_lines` 3.6 → 4.9 ms
(+30%), reproduced in three interleaved rounds; the profile shows the background search's per-line matcher
(4.4% → 6.4% of samples). Accepted by the user as a deliberate tradeoff (4.9 ms absolute). Other rows over 20%
moved both ways and did not reproduce: `tui/pty/input-first-byte` +93% in the suite, but 0.49–0.64 ms new vs
0.54–0.82 base with the order alternated; `startup/*/load_ms` 3 → 4 ms is the 1 ms resolution;
`ui/merge/undo-keep-base` +33% beside `accept-theirs` −19%. Unverified: native macOS.

## 2026-10-15 sticky scroll: declarations from the highlight parse

Sticky scroll asks the background highlight parse for declarations (the tags query on the same tree) while it is on
(the default). Release build, same machine, against `main` (230393a):

| Experiment | Outcome |
|---|---|
| Tags query compiled lazily inside the parse's syntax arena | **Rejected.** The query's memory lived in the arena and tree-sitter aborted (`query.c` pattern map assertion) in the `syntax/*` workload; the query is now compiled before the arena, as `declarations_while` already did. |
| Declarations in the same parse (100k-line Rust, folds on) | **Kept.** `syntax/parse` 271–275 ms, `syntax/parse-declarations` 380–399 ms (three runs): about 110 ms more, off the UI thread, only while sticky scroll is on. Tags query compile on first use: Rust 14 ms, TypeScript 35 ms, Python 5 ms. |
| Sticky lookup per frame (11,112 declarations) | **Kept.** `syntax/sticky-lookup` 0.38–0.40 µs (binary search plus nesting depth). |

Full suite (`--profile full`, three runs) against the `main` run of 2026-10-14: rows over 15% moved both ways
(e.g. `ui/merge/keep-base/first-damage` −39%, `tui/merge/cancel-queued-200` +98% at 0.3 → 0.6 ms). The startup CPU
rows (`startup/warm/small/cpu_seconds` 0.54 → 0.75 s) did not reproduce with the order alternated (three rounds, three
runs each): small files 0.42–1.06 s on both; 100k-line Rust first paint 179–199 ms on both, background CPU median
2.28 → 2.43 s and RSS +1–2 MiB (the declarations). The background parse cost was accepted by the user as the
feature's tradeoff. Unverified: native macOS.


## 2026-10-17 shared Reader and Diff features

Shared appearance preferences, bounded in-document find behavior and the GPUI
find control retain separate application cores. The [verification report](shared-features-performance.md)
records source/executable identities, original and rebuilt baseline comparisons,
40 alternating timing pairs, private-display checks and 30-second find stress.
Timing intervals included zero; anonymous-memory changes were near zero against
the original binaries, while file-backed RSS varied between baseline sets.

The shared font-size probe initially reproduced Reader ignoring preferences, then
passed at 13/26/32. Review found clipped large-font context menus; viewport bounds,
selected-row scrolling and accumulated pixel-wheel movement passed red/green
adapter checks and native 800×600 inspection. A stale-Cargo-artifact benchmark
attempt was rejected and retained separately; forced workspace release rebuilds
and immutable executable copies corrected the experiment. Long stress, cold
starts, native macOS and physical display latency remain unverified.

## 2026-10-18 Reader TUI workspace integration

The existing reader terminal adapter now uses local workspace content and the
same core command catalog as the reader GUI. The [integration report](code/tui-performance.md)
records the full local gate, staged launcher PTY checks, raw identities and
separate GUI/TUI comparisons. Five alternating GUI startup pairs and three scroll
pairs showed comparable timing and process memory: large-text content marker
148.29 → 148.63 ms, scroll intervals 48.001 → 47.999 ms on lavapipe.

Against the standalone reader with its older content dependency, seven terminal
startup pairs measured first frame 2.87 → 3.20 ms and source content 6.55 → 6.76 ms.
The different shared-content versions prevent attributing that small early-frame
shift to the crate move alone. Sixty-second page-navigation runs both completed
740 cycles at about 0.50 ms median feedback; candidate post-stress RSS was
35.89 MiB versus 37.46 MiB. Neither short stress nor RSS proves leak freedom.

The first collector attempt rejected a corpus directory named `*.rs` while
hashing source identity. Its failed log was retained; hashing only files passed
a red/green regression test and the complete collector suite. Native macOS,
hardware rendering, installed providers, long sessions and release packaging
remain unverified by this integration.


## 2026-10-18 Reader navigation and provider recovery checks

The reader TUI now shares GUI navigation and contextual commands; both readers expose language-server
status and recovery. The [reader verification record](code/tui-performance.md#2026-10-18-menu-parity-and-lsp-recovery)
retains focused release comparisons and their limits. Source-content feedback, sustained input, GUI scrolling
and memory were comparable in these short checks. An apparent small-source GUI startup increase did not
repeat across nine further alternating pairs. Moving the terminal brand into the footer confounded the old
first-brand endpoint; a controlled repeat measured the shared tab label and source content instead. No
performance tradeoff was accepted. Native displays, macOS and long sessions remain unverified.


## 2026-10-18 in-app Java runtime repair

Java runtime discovery, validation and settings writes use the existing core workers on demand. The
[reader verification record](code/tui-performance.md#2026-10-18-in-app-java-runtime-repair) retains the full
local gate, identical GUI/TUI settings replay, staged-launcher Java 21 → Java 26 repair, source preservation,
and production executable comparisons. Runtime selection does not install software or infer compatibility
with every JDT LS version. Short private-display/PTY checks do not cover long sessions or native macOS/Windows.

The first GUI feedback collector expected F2 to dismiss its own modal; it correctly received no paint. The
collector was corrected to send Escape. A subsequent comparison included a cold shader-cache launch; that
run is retained separately and a warmup pair precedes the final seven alternating pairs. No performance
tradeoff was accepted.


## 2026-10-18 per-directory Reader workspaces

The shared core restores tabs and reading positions from the existing per-project places store. Checkpoints
happen on directory switches, normal window/view closure and application quit; the existing saver performs
writes, and inactive tabs remain metadata until activated. The [verification record](code/tui-performance.md#2026-10-18-per-directory-workspaces)
retains the local gate, core/adapter checks, TUI → GUI → TUI replay, native private-display close/reopen and
staged-launcher checks with fifty saved tabs. Source files remain unchanged.

The round-trip test caught stale GUI state when relying solely on entity destruction. Explicit close/quit
checkpoints and an awaited background quit flush fixed it. A first inline workspace representation enlarged
the completion enum; boxing that optional metadata kept the normal completion path compact. No new idle
worker was added. Forced termination, long sessions and native macOS/Windows remain unverified.

## 2026-10-18 Reader GUI app menu

Reader reuses its shared command catalog for a grouped ☰ app menu and the Diff-style Commands footer control;
TUI command labels expose the same groups. No new service, dependency or document copying was added. The full
local gate and actual input parity checks pass. Private production screenshots cover dark, light and narrow
layouts. See the [verification record](code/tui-performance.md#2026-10-18-code-gui-app-menu-and-commands-control)
for raw evidence, configuration, limitations and timings.

Seven alternating production GUI pairs measured first feedback 172.60→173.50 ms and Commands response
24.79→26.06 ms; four additional pairs measured 166.96→172.54 ms and 24.21→23.03 ms. Response differences reverse
and startup samples overlap. GUI RSS is about 0.7–0.9 MiB higher across these runs; the detailed repeat locates
most of the difference in file-backed pages (97.32→97.91 MiB), with anonymous RSS 53.62→53.56 MiB. Exact RSS
preservation is not established; mapped-file attribution remains unverified. The new app-menu stress has flat
RSS for 19 open/View/back cycles over 15.24 seconds. Terminal content, input and RSS remain close to baseline.
These are short software-rendered Linux checks, not native macOS/Windows or long-session/GPU-memory proof.


## 2026-10-18 Reader Search Everywhere

Files, local syntactic symbols and actions now share bounded core ranking and one tabbed popup in each adapter.
Shared snapshots and existing cancellable workers avoid loading candidate documents, starting language servers
or adding idle services. Revision checks reject stale clicks; live action availability is validated centrally.
The [verification record](code/tui-performance.md#2026-10-18-reader-search-everywhere) retains the full local gate,
actual adapter input replay, native private-display double Shift, enhanced-terminal reports, production captures,
raw timing/memory samples and source/executable identities.

A focused review found deleted symbols surviving when no indexing job remained; declaration invalidation and
completed rescans now refresh the popup, with a red/green sole-source-deletion regression. An initial native
GUI OCR check missed visible category labels; cropped/upscaled recognition corrected the collector, with no
application change. Temporary event traces were removed and the diagnostic debug executable rebuilt clean.

Seven alternating GUI pairs measured first captured image 180.75→175.32 ms, F10 response 25.38→24.93 ms and
RSS 175.83→175.95 MiB; ten terminal pairs measured content 7.51→7.18 ms, arrow feedback 0.487→0.479 ms and
RSS 13.42→13.52 MiB. These normal-reading samples show no reproducible slowdown. The 100,000-file rank-only
probe costs 4.02 ms; the new Files rank plus row metadata costs 4.38 ms, and All plus 10,000 declarations costs
4.57 ms. These worker outputs differ, so they do not establish an adapter-latency regression. New-popup stress
runs last about fifteen seconds in each adapter; their RSS ranges are bounded for the observed interval, not
leak proof. Native macOS/Windows, physical display latency and long sessions remain unverified.


## 2026-10-18 LSP version context and runtime repair

Mise's selected Node installation lacked Pyright, while Reader showed only stderr's generic final hint.
Bounded raw head/tail capture now retains the cause and recovery hint, with resolved executable and failure-only
runtime/version evidence. The existing Java chooser now also repairs Pyright by selecting an already-installed
Node runtime, without installing tools or changing global mise settings. The [verification record](code/tui-performance.md#2026-10-18-lsp-version-diagnostics-and-runtime-repair)
retains red/green diagnostics, the full gate, final core/clippy checks, GUI/TUI settings replay, actual production
repair and version display, private dark/light/narrow captures, and an initialize/shutdown handshake for the
reported workspace. Provider settings preserve custom launch options, unknown JSON and disabled states.

Version checks now have bounded output and a three-second deadline; no periodic probe or idle worker was
added. Runtime discovery and validation are requested explicitly, and error probes happen after failure.
The first seven GUI pairs and a four-pair repeat showed approximately five milliseconds more startup time;
twenty additional startup-only alternating pairs measured 174.64→173.47 ms, so the difference did not reproduce.
F10 response differences reverse across runs. Terminal content/input remain comparable. GUI file-backed RSS
is about 0.57 MiB higher in the twenty-pair repeat, while anonymous/shared memory is comparable; exact RSS
preservation and mapping attribution are unverified. Chooser stress has a bounded sampled range in both
adapters, with terminal RSS flat for the final twenty of 120 cycles and GUI RSS stable over fifteen cycles.

A fixture script rewrite/execute race was corrected with atomic publication of fresh script inodes; no
production retry was added. Initial PTY harness failures (duplicate TERM and tightly sequenced Escape/F2) are
recorded and excluded from measurements. Native macOS/Windows, hardware latency, full project analysis and
long-session leak behavior remain unverified. Runtime probes may delay a failure report by up to three seconds.


## 2026-10-18 Reader popup scrolling, caret arrows and Recent Files

Popup wheel input now changes only the viewport, plain source arrows move the shared caret, and Ctrl+E reuses
file-search rows with icons, paths and fuzzy matching. Recent ranking stays bounded to saved MRU entries and
is immediate to preserve Ctrl+E, Enter; project name searches retain their cancellable worker. No dependency,
idle service or full-document copy was added. The [verification record](code/tui-performance.md#2026-10-18-reader-popup-scrolling-caret-arrows-and-recent-files)
retains the full local gate, red/green regressions, combined GUI/TUI input replay, normal releases, private
production dark/light/narrow checks, terminal restoration, raw samples and source/executable identities.

Review caught source-byte row lengths being used for expanded tabs/inlay hints during unwrapped cursor
movement, and a dismissed Places popup reusing its old viewport. Focused regressions cover both fixes. Native
collectors initially confused OCR/background tab text and persistent clipboard notices; semantic Enter/copy
assertions and popup-only checks corrected those assumptions. Failed evidence remains separate.

Seven alternating GUI pairs measured first image 178.42→175.96 ms, F10 feedback 25.00→25.34 ms and RSS
175.82→175.91 MiB. Ten terminal pairs measured first source 7.64→7.76 ms and RSS 13.63→13.68 MiB. Down feedback
was 0.502→0.415 ms, with the requested semantic change from scrolling to caret movement. These samples remain
close to baseline; exact preservation is not proven. New Recent stress lasted 15.35 seconds/34 cycles in GUI
and 15.02 seconds/73 cycles in terminal, with RSS ranges 182.73–182.88 MiB and 9.742–9.797 MiB respectively.
These short bounded samples are not leak proof. Native macOS/Windows, GPU memory and hardware display latency
remain unverified.

## 2026-10-21 Reader trackpad input and LSP indexing progress

Quarter-row trackpad events were rounded to zero, while Ruby LSP's reported percentages were discarded.
Accumulating movement per pane/document and retaining structured work-done reports fixes those reproduced
client defects. Tests also caught fractional overscroll delaying reversal at EOF/right and preview-tab ID
reuse carrying movement into another document. Both footers expose progress; no server coverage was reduced.
The complete local gate and the real Ruby LSP 0.26.11 fixture pass.

The [verification record](code/performance.md#trackpad-scrolling-and-indexing-progress-2026-10-21) retains
raw identities and the limits of the measurements. Final alternating GUI first content measured
126.78→126.65 ms; the software-rendered scroll probe remained near 48 ms with text and a loading Ruby
project. Fifteen-second text scrolling used 7.51→7.52 seconds of reader CPU and settled at 166.04→165.15 MiB.
An intermediate +4 MiB RSS difference did not reproduce with the final executable or idle-memory attribution.

An initial Ruby fixture lost the installed user gems through XDG isolation; explicit GEM_PATH and a
server-presence assertion corrected it. Early semantic/terminal artifacts with uncertain provenance were
excluded and rebuilt from the unchanged source. These checks do not prove faster server indexing or native
macOS responsiveness. The reported Rails app, native platforms, long sessions and GPU memory remain unverified.

## 2026-10-21 shared GUI wheel handling

Wheel conversion, Shift redirection, fractional steps and overscroll discard now live in `lenz-gpui::scroll`.
Reader and Diff share the input arithmetic while their existing cores retain document state. Diff's source
view already kept fractional positions, but discarded small vertical movement on diagonal events; a
red/green regression now exercises both sides in all four modes. Its PR description and Reader use the
same step/clamp helpers. A repaint regression also caught Reader clipping the last partial column; one
ceiling in columns now governs input and render limits. No dependency or background worker was added.

The [verification record](code/performance.md#shared-gui-wheel-input-2026-10-21) retains the full local gate,
actual adapter checks, repeated production timing, RSS attribution and ten-second scroll stress. Final
wheel feedback measured Diff 27.04→27.23 ms and Reader 25.53→26.11 ms; the earlier repeat moved in the other
direction. An earlier 2.8 MiB Reader RSS difference reduced to 0.46 MiB in the final repeat, predominantly
file-backed pages; exact preservation remains unproven. A failed private-display connection attempt was
excluded and repeated with one owned display for the batch. Native macOS/Windows, physical trackpad and
display latency, long sessions and GPU memory remain unverified.

## 2026-10-21 large Rails initial indexing

A pinned Discourse 3.5.0 checkout (8,294 Ruby files, 277 installed bundle gems) reproduced substantial
one-time indexing with Ruby LSP 0.26.11. The [experiment record](code/performance.md#large-rails-indexing-experiment-2026-10-21)
retains exact exclusions, phase measurements, raw samples, executable/source identity and limits.
Without Ruby timing hooks, selected project and tooling-gem exclusions reduced readiness from 13.20 to
10.67 seconds (19.2%) and peak summed driver/server RSS from 747.8 to 648.0 MiB, across three samples per case.
Every sample completed the full local declaration index and a real cross-file definition check.

Dependency parsing dominated; disabling linting does not remove RuboCop/Parser source from navigation
indexing. Pausing Lenz's crawl had a modest server gain but delayed local symbols from 8.60 to 22.04 seconds
and increased peak RSS in the instrumented matrix, so it was rejected. Disabling automatic decorations was
inconclusive; YJIT did not improve startup in the uninstrumented confirmation (13.45 seconds).
An initial recursive-glob check corrected `/**` to `/**/*.rb` before exclusion measurements. Ruby timing
hooks inflated the apparent gain, which is why the final claim uses the confirmation without those hooks.

Exclusions remain opt-in because they remove navigation coverage; no runtime defaults or user settings
changed. These are short Linux shared-core/provider runs, not frontend latency or sustained stress.
Native macOS, the reported private Rails project and Rails add-on boot remain unverified.

## 2026-10-21 practices from other LSP editors

[Primary-source research](research/large-workspace-lsp.md) found explicit tooling-gem exclusions, scoped
configuration, correct roots and server reuse in other editors. The narrower [follow-up trial](code/performance.md#editor-lsp-practices-trial-2026-10-21)
kept Discourse’s default project-file coverage while excluding eleven tooling gems: Ruby LSP readiness measured
13.33→11.94 seconds (10.4%), peak summed core/server RSS 724.5→682.8 MiB, across three fresh sessions per case.
All samples completed the full local index and navigated into both an application concern and ActiveRecord.

A 30.082-second, 1,062-switch run retained one Ruby LSP process and returned nonempty file symbols after
every switch (median 28.86 ms). Lenz already implements this reuse; no new orchestration was added. A
preflight dependency check needed to wait for the actual asynchronously opened target, rather than only
request/loading state. The measured driver does so. Project-scoped initialization settings remain a useful
gap; global exclusions would also affect unrelated projects. Watcher exclusions and server-owned caches
cannot be assumed to reduce Ruby LSP's first index. Tooling exclusions remain opt-in, and source navigation
into those gems is lost. Native macOS, the private app, Rails add-on boot and long sessions remain unverified.

## 2026-10-21 project-scoped language-server tuning

Per-workspace initialization/configuration settings now live in user-owned provider configuration, with
shared F2/menu actions for the measured Ruby tooling-gem profile and complete project reset. Only the
saved provider/root restarts; atomic writes preserve unrelated settings, failures leave servers intact,
and held-completion tests cover switching projects and cancelling an in-flight symbol search. Actual
GUI/TUI input replay covers both appearances and tiny layouts; the full local gate passed.

The [verification record](code/performance.md#project-scoped-lsp-settings-2026-10-21) retains all twelve
standalone release Rails samples: default readiness 13.86→13.78 seconds; the new binary's
scoped tooling profile 12.92 seconds (6.2% below its defaults), with peak summed RSS
761.3→673.6 MiB. Every sample completed the full local declaration index and application/Rails-gem
navigation assertions. A 30.043-second, 1,020-switch run retained one Ruby LSP process.

An initial combined Cargo build enabled GUI test dependencies and unified GUI image features into the
core example, making its release workload incomparable. Executable sizes exposed the issue. Those
measurements and investigation repeats are retained as diagnostics; production GUI, standalone core and
terminal builds were separated and all accepted workloads repeated. Review also caught a cancelled Symbols
request retaining its spinner; a red/green regression now finishes the result with a retry message.

Saved GUI binaries initially measured first image 156.81→163.63 ms, and a repeat retained that startup
difference. A fresh build of the exact pre-change source snapshot did not reproduce it: six alternating
pairs measured first image 165.06→162.13 ms and wheel feedback 26.15→25.12 ms. Both source identities and
comparisons are retained; preservation against the older executable remains unproven. No deliberate startup
tradeoff was accepted. Terminal source receipt measured 6.50→6.62 ms. Ten-second GUI and five-second terminal stress retain durations and RSS
ranges in the record. No default coverage was reduced; opting in loses navigation into the excluded
tooling gems. Native macOS/Windows, Rails add-on boot, the private app, GPU memory and long sessions remain
unverified; short Linux timing/RSS samples do not establish exact preservation or leak freedom.

## 2026-10-21 persistent declaration and Ruby gem cache prototypes

The [experiment record](code/performance.md#persistent-index-prototypes-2026-10-21) compares two isolated
prototypes on the pinned large Discourse fixture. Saving the existing local declaration records and validating
every source file reduced full local coverage from 7.364 to 0.168 seconds in the component probe. In five
real Session pairs, local completion improved 7.895→0.488 seconds, Ruby LSP Ready 14.451→13.550 seconds,
and sampled CPU use 22.67→13.95 CPU-seconds, with all local candidates and application/Rails-gem navigation
checks preserved. This selects content-validated local persistence as the next implementation direction;
no production cache or server-startup policy was added.

The single-root prototype's driver peak fell about 273→43 MiB, while combined startup peak RSS increased
732.6→759.3 MiB due to Ruby LSP's measured residency. The two 30-second stress trials did not retain the same
combined-peak increase; overall memory preservation remains unproven. Cold creation has one sample only.
Production lifecycle, schema/digest validation, bounded storage, cancellation and adapter checks remain work;
no performance tradeoff has been accepted.

Metadata-only validation saved about 31 ms but returned stale names after an equal-size, preserved-mtime edit,
so it was rejected. Parser-gem record replay reduced instrumented server readiness 15.30→14.43 seconds,
but an application reopening Parser::Base produced duplicate singleton-class records after replay. The simple
replay design is rejected; dependency records need current-project ownership/resolution. Initial harness
failures involved normalizing lazy empty mixin lists and reusing a mutation fixture; corrected checks retain
the actual prototype counterexamples. Native platforms, frontend display/scrolling, Rails add-on boot,
the private app and long sessions remain unverified. Raw sources, identities, samples and check logs remain
under `target/perf/index-cache-prototypes/`; both production adapters and runtime sources were preserved.


### 2026-10-21 branch switching and cache identity follow-up

The [follow-up record](code/performance.md#branch-switch-follow-up-2026-10-21) exercises an actual disposable
A → B → A checkout. All local-index results match fresh extraction, but returning to identical A source
still reparses 3/4 files because the prototype requires inode/mtime equality. The next cache design should
reuse by content plus parsing context, with bounded recent versions and a separate current inventory.
This improvement is proposed, not measured or implemented.

A cache-disabled shared-core probe also reproduces an existing stale-listing gap after synthetic watcher
overflow or a directory-only event: a rescan discovers paths but retains old records for surviving files.
Exact-file notifications repair the index. Fixing coarse/full invalidation and validating live checkout
races takes priority over changing cache formats or adding parallel hashing. No production code changed;
macOS notifications and live Rails/LSP branch switching remain unverified. Evidence is retained under
`target/perf/index-cache-followup/`.

## 2026-10-21 live checkout and watcher recovery

The shared core now repairs the stale declaration listings reproduced above: directory changes invalidate
descendants, overflow rebuilds known declarations, and completed scans reconcile added/deleted files with
Ruby LSP. Open searches rerank, obsolete work is rejected, and both adapters' actual keys/results are tested.
Registration validates missed changes without discarding unchanged records. No persistent cache was added.

The [verification record](code/performance.md#checkout-and-watcher-recovery-2026-10-21) exercises actual
Discourse A → B → A checkouts twice while suppressing precise OS events. The changed subtree recovers local
symbols in 0.104–0.109 seconds; simulated overflow takes 7.537–7.584 seconds for the full background rebuild.
Exact definition checks exposed Ruby LSP's deferred indexing of changed open documents. An internal
document-symbol refresh and a disk-state report after closing a dirty document repair that gap without
replacing navigation requests or restarting the server. The final live test and full local gate pass.

Four final default-coverage pairs measured local completion 7.262→7.377 seconds, Ruby LSP Ready
12.830→12.610 seconds and peak summed RSS 740.1→740.9 MiB. An earlier equivalent-settings comparison had
a larger Ruby RSS shift; identical-configuration repetition did not retain it. GUI first image/wheel feedback
measured 156.04→154.83 / 26.61→24.17 ms; TUI first source bytes 6.04→6.20 ms. The after binary completed
1,041 successful switch/symbol cycles in 30.036 seconds with one server. Exact preservation and leak freedom
are not established; detailed ranges and the larger after stress maximum remain in the report.

Rejected experiments include treating watch registration as a rewrite (local startup 8.204→9.430 seconds),
requeuing the currently running chunk during unchanged rescans, and accepting stale Cargo artifacts from
the prior prototype as a baseline. Separate clean source builds, registration handling and a queue regression
test resolve those issues. Fuzzy symbol matching was also an inadequate checkout oracle; exact definitions
exposed the real Ruby defect. Native macOS/Windows, GPU memory, Rails add-on boot and long sessions remain
unverified. Bundle/runtime changes may still need a manual server restart. Raw evidence is retained under
`target/perf/watch-invalidation/`; the bounded content-version cache remains the next separate experiment.

## 2026-10-21 configurable content reuse and index rebuild

The [implemented comparison](code/performance.md#configurable-index-reuse-and-rebuild-2026-10-21) now uses
bounded, content-validated declaration persistence in the shared core. Cache reuse and Ruby tooling exclusions
default on, both have GUI/TUI opt-outs, and **Rebuild Project Index** drops local saved/current declarations,
refreshes open documents and restarts the project's servers. Explicit Ruby overrides retain precedence.
Source bytes and settings survive rebuild; server-owned workspace/build caches remain server-managed.

Four alternating release rounds measured previous → both-enabled reopening local completion at
**8.122→0.523 seconds**, Ruby LSP Ready **14.046→11.611 seconds**, sampled CPU **22.84→12.41 CPU-s** and
peak summed RSS **726.6→682.0 MiB**. First uncached local completion stayed **8.054 seconds**, with Ruby Ready
at **12.070 seconds**. Cache-only mode retained a combined-memory cost: **750.8 MiB** initially and
**762.9 MiB** in a four-round investigation repeat, versus **727.7 MiB** for the repeated previous binary.
Most combined variation is in Ruby; warm driver residency also increases. The report preserves this tradeoff.

Actual A → B → A checkouts twice validate declarations and exact old/new definitions. Simulated event loss
recovers local coverage in **0.516–0.526 seconds**, with Ruby checks finishing by **5.557–5.831 seconds**.
Explicit drop/rebuild then takes **11.335 seconds**. A 30.020-second after run completes 1,038 successful
switch/symbol cycles with one server. Storage tests cover bad checksums, extractor changes, same-size edits,
recent versions, late writers, cancellation and bounds. The full local gate passes, including actual adapter
input in both appearances and narrow layouts.

Initial GUI and TUI startup differences prompted repeats. The six-pair GUI repeat and sixteen-pair isolated
TUI startup check did not retain them; twelve pinned TUI pairs also had essentially equal startup/CPU times.
Raw original/repeated results and short stress ranges remain in the report; exact preservation and leak
freedom are not claimed. A real pending-symbol spinner defect found during review was reproduced and fixed;
the initial interrupted timing collection is excluded from final claims. Native platforms, physical scrolling,
GPU memory, Rails add-on boot and long sessions remain unverified. Evidence is under `target/perf/index-settings/`.

## 2026-10-21 cross-language saved-index validation

The [cross-language record](code/performance.md#cross-language-index-validation-2026-10-21) validates the
unchanged implementation on all nine other declaration grammars. Three samples per setting on generated
1,200-file corpora measure complete local coverage at **164–347 ms with reuse off**, **174–360 ms for first
cache creation**, and **21–23 ms with saved declarations**: **7.6–15.7×** faster reopening. First creation adds
5–16 ms here; this cost and the synthetic nature of the workload are explicit. Short process samples do not
establish precise memory preservation.

Ten real-server scenarios pass: Rust, Python, Java, Kotlin, JavaScript, TypeScript, TSX, HTML, CSS and a second
TypeScript run through tsserver. Exact definitions, file symbols, A → B → A content changes with synthetic
overflow, rebuild, opt-out and unchanged source bytes are checked. Java/Kotlin answer the checked definition
while background indexing continues; semantic indexing speedups are unproven. Newer Rust/Java/VS Code server
versions retain the existing compatibility notices because this is focused coverage.

A normal core regression compares all declaration fields and syntax-error counts with fresh extraction,
including Unicode/CRLF, persisted reopen and same-size/preserved-mtime edits. The first test wrongly assumed
a compact Kotlin fixture would be warning-free; the corrected oracle preserves the extractor's warnings.
All large fixtures are warning-free. A probe build was corrected to use the public action catalog without
changing runtime APIs. The full local gate passes. No runtime behavior changed, new dependency was installed,
or user configuration was edited. Raw evidence is in `target/perf/index-languages/`; native macOS, full-server
completion, production dependency graphs, physical scrolling and long sessions remain unverified.

## 2026-10-21 further indexing experiments

The [three isolated prototypes](code/performance.md#further-indexing-experiments-2026-10-21) leave production
runtime and frontend executables unchanged. On Discourse, content-aware notifications reduce watcher-overflow
recovery through exact semantic checks from **5.503 to 0.809 seconds**. Three alternating rounds cover two
A → B → A cycles each, exact old/new definitions, no-op overflow, missing fingerprints and explicit rebuild.
Full hashing costs about 267 ms; even directory-scoped discovery/comparison costs 71 ms and regresses normal
directory recovery from **0.548 to 0.636 seconds**. The extra directory scan is rejected. Next integration
should reuse inventory/index reads, with background validation and conservative fallback; fingerprints must
also stay current after ordinary edits to avoid stale results on a later checkout back.

A borrowed-UTF-8 cache-hit path reduces isolated warm local indexing **250→237 ms** and sampled peak RSS
**44.6→38.2 MiB**. First cache creation is 29 ms slower; nine smaller language corpora show mixed shifts
around 22–23 ms. Defer this small gain. The candidate passes 340 core tests (16 existing ignored), all 12,077
Discourse candidates against fresh extraction, and encoding/offset/line-boundary checks. Fingerprint checks
cover preserved-mtime edits, rename and missing snapshots. Initial probe build errors were corrected in
isolated examples; their logs are retained.

A serial-client helper retains a whole headless Session and real Ruby server across new client processes.
Reopen medians improve **12.329→0.956 seconds**; disconnected checkouts, an injected reconnect edit,
lockfile/config changes, rebuild and expiry pass. Initial startup stays 12.6–12.8 seconds. A 29.906-second idle
interval holds **699.1 MiB** and uses **0.09 CPU-seconds**. Expiry at 35 seconds stops Ruby but leaves a
**66.8 MiB** helper until shutdown. This remains an opt-in design candidate, with simultaneous-view isolation,
full runtime identity, crash recovery and daemon security unproven.

Raw sources, binaries, identities, configs, 92 cache samples, nine recovery scenarios and both helper
scenarios are retained in `target/perf/index-next/`. Only README/report/notebook text changes in the normal
tree. The prior full application gate is not rerun; local links and whitespace are checked. Native macOS,
frontend integration, physical scrolling, GPU memory, multi-project limits and long sessions remain unverified.

## 2026-10-21 integrated content-aware overflow recovery

The [shared-core implementation](code/performance.md#content-aware-overflow-recovery-2026-10-21) retains the
digests already computed by saved declaration indexing and compares them through the existing background
reads after overflow. It adds no extra directory walk, read or content hash. Ordinary file/directory updates retain
their behavior; precise edits, registration during recovery, overlapping overflows, missing reads and index
limits have conservative handling. The existing saved-index opt-out disables filtering, and explicit rebuild
still clears records and restarts the project's servers. Both frontends use the same implementation.

Initial paired release results on Discourse improve overflow recovery through exact semantic checks
**5.560→1.137 seconds** (about **4.9×**), ordinary directory recovery **0.576→0.501 seconds**, and no-change
overflow **5.658→1.244 seconds**. Three further pairs confirm overflow **5.761→1.053 seconds**. Both sets
verify two A → B → A cycles, exact local/server names, old/new definitions, deletion, opt-out and rebuild.
Complete-scenario sampled CPU falls **83.67→69.06 CPU-s** initially and **87.72→75.06 CPU-s** in the repeat.
Pre-restart summed peak RSS falls in both sets. Full-scenario RSS rises initially (983.48→1,049.51 MiB),
then reverses in the repeat (1,066.57→1,043.61 MiB); the report preserves both. Per-file fingerprint storage
adds roughly 1–2 MiB to measured reader residency within the existing accounted index bound.

Cold startup is not improved. Initial Ruby Ready timings rise 11.318→11.598 seconds cold and
11.043→11.189 seconds warm; four more pairs give 11.417→11.558 cold and 11.034→10.932 warm. Separate opt-out
startup improves local coverage 7.638→7.560 seconds while Ruby Ready rises 11.360→11.575. Rebuild and
post-opt-out timings in the longer sequence also prompted investigation. A real protocol trace proves the
same initialization, request counts and one identical 16,405-path opt-out notification; the delay mainly
precedes the final symbol query. The shared development host has unrelated active work, so small timing/RSS
differences are not attributed confidently to the change. All unfavorable samples and traces are retained.

Two final pairs on fixed CPUs retain the overflow gain (**6.930→1.404 seconds**) with ordinary directory
recovery **0.680→0.592 seconds**. Opt-out times are 8.783 / 10.950 seconds before and 10.157 / 10.339 after;
the second pair reverses the previous difference. Baseline variation contains both candidate times. This
does not establish a consistent opt-out regression, or prove exact preservation on an exclusive host.
Fixed-affinity full-scenario RSS is 1,072.11→1,069.47 MiB. The robust benefit is avoiding redundant overflow
work; native-machine startup and long-session resource behavior still need validation.

A deterministic completion-order test caught duplicate notifications when registration caused a follow-up
scan; preserving the baseline through that final inventory fixed it. Another test's initial candidate count
forgot Gemfile; the assertion was corrected. Exact-file events now remove their own pending entry directly,
without scanning the whole fingerprint map. The complete local gate passes. One existing GUI navigation test
timed out in an earlier gate, then passed alone and in the full rerun without a code change; its cause remains
unconfirmed. GUI/TUI input, startup and short stress checks pass; 30-second real-Ruby switch/symbol loops
retain one Ready server and complete 1,059 / 1,086 cycles.

Raw evidence, final production binaries, before/after source identities, configs, failed runs, protocol logs
and collectors are under `target/perf/overflow-content/`. Native macOS/Windows, physical scrolling, GPU memory,
long sessions and real-server speed in other languages remain unverified. The lighter UTF-8 cache path and
retained-server helper remain deferred.

## 2026-10-21 Language Servers menu refinement

Moved the diagnostic paragraphs behind one shared **Server Details…** action and reused each adapter's
scrollable snapshot popup with fixed Copy/Close controls. F2 and the application menu use the same compact
management actions; the full report remains available through Find Action. The [validation report](code/performance.md#language-servers-menu-and-details-popup-2026-10-21)
records the local gate, input/render parity, screenshots and paired release measurements. Initial GUI menu
feedback was a few milliseconds slower, but four repeated pairs reversed the difference; there is no
consistent menu regression in these samples. Other startup/input/RSS measurements are similar. Raw evidence
is in `target/perf/server-menu/`. Native macOS, GPU memory and long sessions remain unverified.

## 2026-10-21 Markdown control in the file header

Moved Preview/Source into each Markdown pane's header in both adapters, retaining the shared toggle and
keyboard/menu access in narrow panes. [Validation](code/performance.md#markdown-file-header-control-2026-10-21)
covers actual input, split-pane isolation, labels, themes, tiny layouts and paired release workloads.
Initial wheel feedback was slower; a second four-pair set reversed the difference, while ten-second scroll
throughput is nearly identical. Startup and sampled memory are similar. Raw evidence is under
`target/perf/markdown-control/`; native macOS, GPU memory and long sessions remain unverified.


## 2026-10-21 Lazy library browsing

Dependency definitions now reveal a shared Libraries tree. Directory reads are lazy, bounded and cancelled on
collapse, with request identities rejecting late results; sibling navigation keeps the existing project and
its declaration inventory. The [validation report](code/performance.md#dependency-libraries-in-the-explorer--2026-10-21)
covers the complete gate, actual GUI/TUI input and render parity, live Ruby LSP, virtual-source reopening,
limits and release comparisons. A 1,000-file library revealed in 0.811 ms in the core, while its unopened
10,000-file subtree stayed unread. The ten-second loop completed 21,907 collapse/expand cycles.

An intermediate build added about 3.8 MiB of mapped executable pages in GUI samples; smaps showed steady heap
use. Marking infrequent library mutations cold returned sampled RSS to the baseline range in repeated runs;
the final ordinary comparison measured 168.78→168.88 MiB. Extracting tree activation from the shared command
dispatcher into a cold function reduced an intermediate TUI executable-page increase. Final GUI startup was
13.18 ms slower, with a CPU-affinity control reducing the difference to 5.16 ms; earlier runs changed sign.
Capture/scheduling variability leaves the startup effect inconclusive. TUI RSS varied between samples,
including while syntax work was still running. Four final stress pairs measured 15.82→16.24 MiB median
terminal RSS, with nearly all the remaining increase in mapped executable pages and anonymous mappings
6.094→6.098 MiB. The cold extraction reduced but did not eliminate that increase. Failed collectors and all raw
samples remain in `target/perf/library-tree/`. Native macOS/Windows, GPU memory and long sessions are unverified;
virtual sources can only list visited files, and unfamiliar package layouts fall back to a nearby manifest or
containing folder. This work does not speed up a language server's own indexing.

## 2026-10-23 Reader snapshot migration

The core now owns indexed immutable current snapshots and a bounded view facade, with explicit
compatibility Document paths retained for the adapter/analysis migrations. The [measurement report](research/code-snapshot-migration-performance.md)
records source/executable identities, raw evidence, the full local gate and the measured performance decision.
Editing remains disabled at this stage.

Sharing original text removed a duplicate allocation; reusing decoded line-index ranges later removed
roughly 300 KB of copied offsets and 231 allocations for the larger fixture. Per-dispatch registry scans
were removed, cache accounting moved to ownership transitions, and prefix/storage accounting was fused.
The smaller exact-size/prefix cleanup passed stress but did not resolve opening latency. Earlier
shared-target samples are excluded after a real build collision; a concurrent-validation timing batch
is also excluded. All failed, intermediate and inconclusive samples remain retained.

Two clean final pairs still measure 6–8 µs extra core first-text time for 50,400 bytes and 24–88 µs for
945,000 bytes. Tail direction varies; this is not a preservation claim. The user explicitly accepted the documented opening and typical cached-operation costs on 2026-10-23. The final
180.15-second core stress passed with settled VmRSS 17.02→17.23 MiB, against baseline 17.93→17.96 MiB;
these are short-run process-memory observations, not leak proof. Ten alternating cached-operation pairs reproduced roughly 14 ms tails
in both baseline and candidate; their exact cause remains untraced. Typical cached-operation medians
rose by about 0.2 µs while p95 improved. The full local gate passed at `39a830c`, including actual adapters,
collectors and release checks. Native macOS, physical display latency, complete rendering/input migration
and long-session behavior remain unverified by this prerequisite.

## 2026-10-23 GUI snapshot windows

The [scoped GUI report](research/code-snapshot-gui.md) records the migration from
whole-line compatibility reads to current snapshots, shared resumable layout and
bounded native fragments. Paint, hit-testing, decorations and documentation anchors
share the same native origins. The 32-case focused adapter batch passes; the complete
08–10 integration gate remains pending.

A 50,000-mark source exposed966 retained window references after display suppression.
Omitting empty pieces/windows and coalescing hidden source intervals fixed retention
without losing the caret boundary or original copy bytes. The first long-emoji oracle
was invalid because an unbounded native prefix accumulated `f32` rounding; a painted
literal/glyph-width comparison now verifies exact selection. A combined constructor
check also caught and fixed the skipped failed-SVG source fallback. An early command
without the GUI test feature ran no adapter tests and is excluded from evidence.

A sequential release pair suggested 1–2 ms slower GUI startup, but eight alternating
pairs measured 142.45→142.39 ms and did not reproduce it. Ten-tab settled RSS measured
172.50→172.03 MiB; both 15-second scroll runs produced 314 frames and matching 48 ms
intervals on the owned Xvfb/Mesa display. No GUI performance exception is taken.
Raw samples and exact executables remain under `target/perf/editor-implementation/08/`.
The optional native-width diagnostic is version 1 and is excluded from production.
Its sampled default font did not reproduce the proposed positive-column/zero-advance
pair; TestPlatform glyph metrics differ from native ones. Native macOS, physical
presentation, GPU memory, arbitrary fonts and long sessions remain unverified.

A later owned native GUI/PTY probe found a shared Go To reveal gap: the correct
Unicode source byte is selected and copied, but its native glyph position is not
scrolled into view. Shared source-byte goals with identity-validated native feedback
fixed it; a narrow 600×300 replay with 100 trailing spaces paints and copies `END`
with unchanged source bytes. The identical final oracle rejects the old executable.
Full-window OCR initially missed visible text beside emoji; version 4 uses a normalized
source-band crop and was checked against both positive and negative screenshots.
Raw evidence is under `08/native-goto-oracle-v4/` and `08/native-goto-old-oracle-v4/`.
The final focused GUI batch passes 44 tests, and strict GUI Clippy passes. The complete
08–10 gate remains pending.

## Snapshot rendering/analysis integration — 2026-10-24

Tickets08–10 completed a phased local gate with focused repairs for Unicode/tab/hint
vertical movement and adapter fixtures. The unconstrained linker run was cancelled
before tests began because of I/O pressure; four-job resumption retained compiled
caches. No new stress or optimization loop followed. The user accepted the recorded
syntax-readiness, stress-memory and TUI startup/GoTo costs. Successful, unsuccessful
and inconclusive experiments, exact sources and raw evidence are linked from the
[batch report](research/code-snapshot-batch-08-10.md). Measurements precede the final
narrow caret repair. Native macOS and physical-display/GPU claims remain unverified.
Full validation and stress will run once after each batch of several tickets.


## Reader drafts, Save and captured queries — batch 11/12/13/16/18 (2026-10-24)

The [detailed performance report](research/code-editor-batch-11-18-performance.md) compares exact
`bb162738`/`bb2f55f4` release binaries. Correctness is recorded separately in the
[batch report](research/code-editor-batch-11-18.md). **The user accepted this batch’s documented costs on 2026-10-24; later costs require their own decision.**

| Approach or experiment | Outcome and evidence | Limit or next step |
|---|---|---|
| Snapshot-only current ownership, exact-read Save provenance and captured query continuations | Required behavior is implemented. Core first text adds 35/405 µs at 50,400/945,000 bytes; TUI content 7.900→12.075ms, cached switches 5.020→6.362µs. Load provenance adds raw hashing/ending and identity work; exclusive attribution is unproven. | Do not infer preservation from unchanged syntax readiness or GUI cadence. |
| Mixed three-minute core campaign | Candidate passes guards but completes 992,200 iterations versus 1,239,150 in the passing baseline repeat, with settled/peak RSS 20.39/34.61→22.07/36.31MiB. | Throughput reduction and CPU cost remain part of the pending decision. |
| Original baseline memory-trend failure | Retained: 18.86→21.24MiB and late close-together samples all above earlier maximum. Only that failed workload was repeated; the repeat passes unchanged assertions with similar throughput. | Neither allocator retention nor a leak is proved; no threshold relaxation. |
| Treat single-line native probe as scrolling | Rejected. Logical-end handling issues `Scroll(0)`; retained intervals describe repaint. Workload-v2 uses 2,000 lines and 30 distinct source crops per mode/build. Actual native intervals remain near 48 ms on the software display. | Physical frame pacing and CPU headroom remain unverified. |
| Average TUI residency bands into one preservation claim | Rejected. Original settled 37.73→46.42MiB; short zero-stress replays retain candidate 37.59 and 46.10–46.41MiB bands. Phase rows put most of the difference in anonymous residency after Find/reload. | Live owners versus allocator residency remains a follow-up; do not label RSS a leak. |
| Infer frontend editing performance from new command timings | Rejected. Candidate-only Session harness records 200 edits (median 15.65 µs), Undo/Redo 25.36/24.36 µs and one 0.807 ms Save, with exact disk/history/weak-release assertions. | Production typing/IME remains gated; one tmpfs Save is not storage-throughput evidence. |
| Initial auxiliary harness/parser errors | Preserved and corrected: one missing Arc conversion blocked compilation; an empty kernel-status field suppressed first diagnostic phase rows. Canonical PTY checks and the corrected parser self-check pass. | No passed stress was repeated for either correction. |

Timing ended before documentation work and other builds resumed. All sources, executables, commands, durations,
raw samples and failed attempts remain under `target/perf/editor-implementation/merge-11-18/performance/`.
Native macOS/Windows, physical presentation, GPU allocations, real providers and long sessions remain unverified.


## Existing-file editor foundation — batch 14–23 (2026-10-24)

The [batch report](research/code-editor-foundation-batch-14-23.md) compares accepted baseline
bb2f55f4 with private activation candidate fce466f. The full correctness gate is complete in phases,
including the real Ctrl-Q documentation regression repair. Native X11 default input/WM_DELETE and
ordinary Linux TUI input/Save/dirty-close checks pass. Mac/Wayland GUI remain gated.

One quiet campaign used prebuilt artifacts and identical fixtures. Startup/navigation and existing
three-minute core-stress throughput remain similar. One 180-second growing-history edit workload per
build, followed by complete independent Undo/Redo, passed: candidate cycles fell 5.8%, and every
matched cycle window showed higher edit medians (+0.048–0.319 ms). Eight TUI launches show +0.676 MiB
median startup RSS. The user explicitly accepted these costs on 2026-10-24 and activation merged; no optimization is claimed.

Editing and GUI RSS changes are mixed and allocator/history dependent. Lower peak editing RSS and
lower terminal post-workflow RSS do not establish an efficiency improvement. Candidate-only preedit
callback medians are 0.128/0.140 ms at 50 KiB/1 MiB, with four tagged warmups excluded from 32 samples;
these are synchronous callback wall times, not worker, paint or native display latency. Deferred
profiling is in the backlog; no extra stress was run. Raw samples, failed gate attempts, configuration
and hashes remain under target/perf/editor-implementation/merge-14-23/performance/.

## Integrated editor Linux review and campaign — 2026-10-27

The [Linux release report](research/code-integrated-editor-linux-validation.md) compares accepted
foundation `fce466f` with final Reader runtime `ff217fc` (fixture-only follow-up `d6fd35b`), and retains
canonical Diff/core/TUI measurements from `b072d0e`, whose production subtrees are unchanged by the
later Reader-only optimization. Native macOS tickets 03/24 and full acceptance ticket 68 remain open.
The report records the pending end-of-epic cost decision; passing correctness is not that decision.

| Approach or experiment | Result | Interpretation or limit |
|---|---|---|
| Weak diagnostic source identities and ordered publications; root/repository request epochs | Deterministic stale-identity/publication regressions fail before repair and pass afterwards. Template parity targeted repeats and the full gate pass. | Do not attribute every old timeout to these repairs. |
| Atomic recovery discard/new-Save coalescing and synchronous watch registration before background inventory | Reproduced ordering and watch gaps are repaired. Save As/recovery fixtures wait for real worker/startup barriers; each passes 128 repeated checks, and actual TUI rename passes 128 at concurrency eight. | No timeout increase; preserve exact pending-write, recovery and disk assertions. |
| Release marker baselines/HEAD blobs with inactive documents | Weak ownership and forced large-buffer GUI/TUI eviction assertions pass. | Cache accounting alone had missed ownership outside the cache. |
| Skip saved-marker materialization only for exact source metadata identity, and skip line-diff work for equal text | Valid red test observed one conversion instead of zero; final 15 marker tests and fresh full gate pass. Final 180-second Reader PTY RSS is 40.00/50.10 MiB settled/peak, versus baseline 45.70/51.00 MiB. | Initial candidate was 83.25/91.23 MiB; short before/after probes retain its residency band. No trimming or history cap was added. |
| Treat post-Reload baseline publication as idle | Rejected: the first candidate emitted one 139-byte marker update after the old readiness boundary. A delayed-output PTY fixture reproduces it. Schema 4 reports settling separately and repeats both executables. | Candidate preparation is 1.293 seconds/25 bytes; subsequent strict half-second idle is zero bytes/CPU. Do not relabel preparation as idle. |
| Final-owner server teardown with serialized child/stop handoff | Active/stopping-child regressions and real JDTLS checks pass without observer cleanup of its server. | Two Gradle cases leave a separately identified Gradle daemon; fixture cleanup is retained. |
| Ruby changed-draft completion immediately after didChange | Upstream reader/worker ordering reproduced with wire diagnostics. Waiting for changed-snapshot diagnostics repairs the fixture; five serial real replays pass. | No production workaround or enlarged deadline. |
| Demand only an already-running Ruff cancellation notice | Rejected: cancellation can safely occur before launch. Assertion accepts exactly both guarded outcomes; strict release Clippy and five actual provider replays pass. | Only test assertion/comment changed after the full runtime gate. Original failure is retained. |
| Accept a single higher GUI editing RSS sample as a leak or dismiss it as allocator noise | Neither conclusion is supported. Exact-byte production GUI editing passes; alternating source/executable pairs investigate the measured cost. | Allocation ownership and long-session leak behavior are not established by RSS. |
| Compare full-history verification totals after different cycle counts | Rejected as a matched timing claim. Candidate completed 13,176 versus 12,727 cycles; common-cycle byte counts and five timing windows are compared. | Full retained history/oracle intentionally grows. Peak RSS rose 94.73→111.89 MiB even though pre-Undo RSS was lower. |
| Treat test process wall time/child samples as production completion latency | Rejected. Real provider fixtures retain setup/network/observer work and coarse descendant discovery. | Short children may be unobserved; Java/Gradle child processes can dominate memory. |
| Reuse completed canonical records after interruption | Source/executable/fixture hashes checked before continuing with the public suite's exact commands/settings. Interrupted repetition excluded and rerun; all 133/391 comparable records complete. | Interruption and continuation stay distinct in provenance. Reader's interrupted final reading stress is restarted in full. |
| Increase deadlines for older flakes | Not done. Several fixtures receive actual request/worker/render barriers; other old failures do not independently reproduce in targeted repeats. | Repeat passes are evidence, not a proved root cause. Trace any recurrence. |

The fresh final full gate passed on `ff217fc`: strict default/adapter workspace lint, three panic-site
ratchets, workspace/integration/doc checks, six serial memory checks, 261 Diff GUI, 199 Reader GUI,
201 adapter parity and 75 Python collector tests (74 passes, one platform skip), plus standalone
terminal releases and unchanged stripped-size limit. All failed/intermediate/inconclusive experiments,
raw samples, source/executable identities, configuration and actual durations remain under ignored
`perf-results/editor68/`. These are retained local evidence, not public download links. No Mac/Wayland,
physical presentation, hardware GPU or fresh Kotlin pass is implied.

The final reading stress completed 900.20 seconds/987,042 iterations. First/final settled-window
medians were 32.14/33.22 MiB, versus baseline 29.65/29.20 MiB; the roughly 4 MiB final residency
increase remains a cost. Integrated editing completed its full 900 seconds with exact draft/disk,
completion/Undo and temporary-comparison release assertions. Template materialization, actual
external formatter traces, the optional real-Tokio supplement and focused alternating comparisons
are complete, and final artifact/corpus checks pass. The unchanged absolute performance gate
remains failed: view construction also fails on the baseline, and the original candidate regex search
exceeded its 25 ms ceiling; all results and the alternating search comparison remain explicit.

Ordinary GUI editing repeats measured 162.69→207.71 MiB settled RSS medians. The original
55.69 MiB increase, repeated ranges, cached-operation increases, canonical mixed-throughput reduction,
growing-history memory peaks and candidate-only SQL/provider/template costs remain in the report.
One GUI repetition timed out waiting for Undo paint feedback; it is retained and excluded, and only
failed/incomplete repetitions were continued unchanged. Its cause remains unisolated; trace recurrence.
Allocation attribution and infinite-session leak freedom are not proved. **The single cost decision
is pending; no explicit performance exception is accepted yet.**


## Editor polish and bounded Markdown work — 2026-10-29

The editor polish pass adds optional shared Vim editing/relative gutters, token-preserving offline JSON
formatting, explorer speed search and Markdown source/preview styling in both adapters. The full local
gate, actual input/render parity tests and private-display exact-byte workflows pass. One outline-navigation
test timed out in an intermediate gate; its isolated repeat and two subsequent full gates passed without
changing the timeout. Native macOS and physical display latency remain unverified.

The initial full-document CommonMark source pass was rejected: a ~200 KB fixture retained about 8 MiB
extra in parser-worker anonymous heaps and 6.64 MiB extra settled GUI RSS. An additional allocator trim
did not recover it. Reusing the inline tree-sitter grammar for every small paragraph reclaimed memory
but increased parse time from roughly 46 to 152 ms, so that approach was also rejected. The final approach
uses the existing block tree to parse only individual paragraphs needing inline styling. Original byte
positions survive blockquote/list continuation prefixes; a synthetic prefix prevents heading/table inline
code from being reinterpreted as a code fence. Multiline emphasis, separate paragraphs, references and
heading/table code spans have regression coverage. Temporary parser work scales with the largest paragraph;
this is not a fixed memory ceiling for a pathological single-paragraph document.

Three alternating release pairs on ordinary 10k-line Rust editing measured GUI window mapping
320.57→310.64 ms, 30 Enter/Undo cycles at 5.25→5.21 CPU seconds, settled RSS191.67→191.85 MiB and
peak RSS197.55→195.74 MiB. Three standalone TUI schema-4 pairs measured first content5.12→5.25 ms,
pooled input feedback0.726→0.699 ms and settled RSS19.38→19.42 MiB. Each terminal stress ran about
10.01 seconds/123 cycles with exact bytes, idle-output and terminal-restoration assertions. These small
sample differences do not establish a material ordinary-editing regression or a speedup. GUI mapping is
not complete paint; PTY feedback is not displayed pixels; RSS is not GPU memory or leak proof.

Raw samples, discarded experiments, debugger attribution, source/executable identity, collector configuration
and actual durations are retained locally under ignored `perf-results/editor-polish/` and
`perf-results/wayland-editing/`; the detailed local report is `perf-results/editor-polish/report.md`.
New-feature-only probes cover 10-second JSON and cached-tree workloads and grouped Vim Undo/Redo;
these are not before/after comparisons. A long-duration soak, real-provider latency and native macOS were
not rerun for this pass.

Final Markdown release pairs measured parse45.86→46.59 ms and GUI settled RSS192.23→192.57 MiB
(peak200.17→200.90 MiB), removing the earlier repeatable6–8 MiB residency increase. Richer highlight
arrays retain120,000 additional bytes for the ~200 KB fixture. Type-through-Save observation was
841.16→841.90 ms with fixed injection delays; GUI mapping303.52→320.67 ms includes20 ms observer
polling. Control-parser timing also varied, so no precise submillisecond regression or speedup is claimed.
Final full-gate and native-input checks passed on the rebuilt executable. The raw final comparisons are
`parser-final-comparison-*` and `wayland-editing/markdown-final-*` under `perf-results/`.

## Integrated terminal tabs — 2026-10-29

Lenz Code's Linux adapters now present independent terminal sessions through the shared core.
Alacritty 0.26 supplies the terminal model and portable-pty 0.9 creates the PTY; a core-owned,
nonblocking reactor bounds input, history and dynamic cell/control payloads. The stock Alacritty
event loop was rejected during source inspection because of unbounded pending input and blocking,
child-only shutdown. No shell, parser, history or terminal worker exists before the first terminal opens.

Review and real-PTY regressions caught shutdown cases that compilation alone missed: detached
cleanup could lose the last process exit, Quit All could stop an earlier window before every window
consented, and an already-finished reactor could strand a close acknowledgement. The final close
protocol waits asynchronously for all consent and then for owned-session termination/reaping.
GUI history shortcuts, mouse motion/outside releases and file-menu context identity have actual
adapter input regressions. The final complete `scripts/ci-local.sh` gate passed, including actual
Diff/Reader GUI tests, TUI and combined parity checks, memory checks and collector tests.

Retained baseline `2ef2732` and final release binaries ran on the same machine. The terminal panel
stayed unopened for ordinary comparisons: three paired GUI editing runs (100,000 lines, 30 rounds),
five TUI pairs (10,000 lines, 30 inputs and ten-second navigation stresses), and a separate production
GUI mapping observer (three measured pairs after three warmup pairs).

| Measurement | Baseline | Candidate |
| --- | ---: | ---: |
| GUI warm window mapping | 96.87 ms | 96.05 ms |
| GUI edit first XDamage, pooled median | 3.250 ms | 2.222 ms |
| GUI settled RSS median (range) | 248.75 MiB (247.36–252.12) | 250.80 MiB (243.72–251.50) |
| TUI first content | 5.789 ms | 5.535 ms |
| TUI input first bytes, pooled median | 0.741 ms | 0.703 ms |
| TUI settled RSS median (range) | 19.77 MiB (19.16–20.09) | 20.02 MiB (19.08–20.27) |

The initial mixed GUI/TUI runs suggested slower candidate TUI feedback; two isolated TUI pairs did
not reproduce it. All five pairs remain in the summary. Neither the timing changes nor overlapping
RSS ranges establish a speedup, allocation attribution or absence of a smaller cost. TUI idle CPU and
output were zero in every measured idle interval. Mapping uses 2 ms polling and is not completed
paint; XDamage/PTY feedback is not physical display latency. A production startup attempt using the
marker-based collector was invalid because the retained baseline intentionally has no perf-probe
markers; the external mapping observer preserves production binaries and records different semantics.

Separate feature-only probes use small disposable documents and different producers; they are not
before/after comparisons with the editor fixtures above. Production GUI output ran 15.028 seconds
with 15 resizes, 28 tab switches and five exact quiet-tab command effects while output continued.
Parent peak/settled RSS was 193.14/178.70 MiB; saturated CPU averaged 72.29% of one core and settled
CPU was zero. Command effects include fixed synthetic typing delays. The production TUI ran
15.000 seconds/18.36 million completed changing lines, opened a second session in 19.06 ms and
completed its command in 15.94 ms during hidden-tab output; parent RSS ended at 20.52 MiB with
no final-quarter growth. Its half-second idle output and CPU were zero, and confirmed exit restored
termios and ended owned work. Both adapters preserved exact fixture bytes.

The release core probe completed 60.001 seconds, 3,726 checked frames/wakes, 595 resize/selection/
scroll cycles and eight start/stop cycles. Parent RSS started at 8.30 MiB and reached 19.95 MiB sampled
peak, VmHWM and settled; parent/producer CPU was 39.56/59.85 seconds. Grid/history/input bounds,
reaping and an unrelated sentinel passed. The first ten-second schema-1 run omitted cleanup from its
peak; it remains explicitly incomplete. Schema 2 separates sampled RSS peak from VmHWM and includes
cleanup. The TUI collector's first attempt failed before launch on an environment duplication; it is retained.

The production private-display acceptance also checks paste waiting for Enter, independent directories,
an exact local DuckDB result, installed Codex/Claude/psql help and real window-close confirmation with
HUP/TERM-ignoring work. Native harness retries corrected private mise trust prompts, an incorrect close
action and focusing before mapping; no application code was changed for those harness corrections.

Raw samples, collectors, source/executable identities, configurations, all failed/inconclusive attempts
and the detailed report are local ignored evidence under `perf-results/terminal/` and the sibling
`terminal-backend-release-stress-schema2-60s*` files. The normal TUI dependency graph and both TUI
binaries' symbol tables contain no GPUI. Native macOS/other-platform ownership is unavailable pending
verification; native Wayland terminal input, physical presentation, GPU memory, authenticated agent
conversations and a longer soak remain unverified. RSS and these short runs are not leak proof.

A review found that terminal parsing stopped after every byte of 128 or above, so each UTF-8
continuation byte paid a parser stop, a response-queue lock and a combining-mark check. It now stops
after each complete non-ASCII scalar, which keeps the combining-mark bound. A standalone release probe
replays the same split pattern on Alacritty 0.26 with 120×40 grids and 4 KiB chunks. It does not run
the Lenz executable and excludes the control filter and frame projection. Medians of seven runs:

| Output | One advance per chunk | Per-byte stops | Per-scalar stops |
| --- | ---: | ---: | ---: |
| ASCII lines (21.2 MB) | 198.9 MB/s | 194.0 MB/s | 192.4 MB/s |
| Box drawing, CJK and combining marks (24.0 MB) | 329.6 MB/s | 50.2 MB/s | 119.7 MB/s |

**Kept.** The ASCII difference is within run-to-run variation. Stopping only after zero-width
scalars would need decoding in the filter loop and was not tried. Snapshots for hidden tabs, which allocate
a `String` per cell at up to 60 Hz, remain an untested candidate.


## Terminal interaction refinements — 2026-10-29

Terminal tabs now accept per-session names through the shared command catalog and both adapters.
The name is captured by terminal identity, validated centrally, and survives Stop/Restart without
changing the shell, directory or history. GUI header widths use display columns; a real adapter
regression first reproduced CJK-name clipping and then passed with the shared Unicode-width helper.
Selected LF/CRLF endings occupy one display-only cell in both editors; source, endpoints and exact
clipboard bytes stay unchanged. Search Everywhere uses a solid caret at the native shaped advance,
with clipped overflow reveal. TUI Double Shift rejects held-repeat events and uses the host's
modifier reporting; Ctrl+K remains the fallback.

Terminal Find uses Ctrl/Command+F in the GUI and Ctrl+F in the TUI, Enter/Shift+Enter, and Escape.
The core owns the literal query and lifecycle; a bounded worker matcher searches exact base and
combining scalars across soft wraps. Alacritty's existing search was rejected after a confirmed
combining-mark omission. A private streaming KMP matcher uses O(query-length) storage, no history
copy, and no scans on output or repaint. New output/resize invalidates coordinates until Enter.
Review also caught path drops reaching shell input behind Find; the core now refuses them while
Find owns input. Natural shell exit remains searchable; explicit Stop/failure retains only a frame.

The final full `scripts/ci-local.sh` passed after the width fix, including strict Clippy, panic
ratchets, core/workspace, actual Diff/Reader GUI, TUI/parity, memory and Python collector checks.
A production private-display probe passed ten checks, including Rename/Find, selected empty-line
and Double Shift screenshots, exact SQL, offline CLI help and owned-work cleanup. Screenshots were
visually inspected. Raw reporting-host bytes also passed through the final TUI's actual crossterm
parser: left/right Shift taps work, repeats do not, source bytes stay unchanged, Ctrl+K works and
terminal modes restore. This is not proof of every host emulator or SSH/multiplexer path.

Retained releases bracket the follow-up work; source manifests distinguish their working trees
although both share HEAD `2ef2732`. Final GUI warm mapping measured **97.00→96.13 ms** over three
pairs after three warmup pairs, with overlapping ranges. Across five TUI pairs, first content was
**5.321→5.611 ms**, input first bytes **0.974→0.760 ms**, and settled RSS **22.09→19.83 MiB**.
These observations do not establish a speedup or exclude smaller costs. Mapping is not first paint;
PTY bytes and XDamage are not physical display latency.

GUI edit feedback needed several controlled follow-ups. The original 1 ms polling observer showed
a small shift; socket-ready schema 2 retained the input, XDamage endpoint and byte assertions but
removed polling quantization. Its three edit differences were +0.061–0.192 ms, with mixed Undo/Redo.
A same-binary A/A edit pair was tight while Undo varied strongly. Schema 3 then held the exact
absolute fixture path and rebuilt private XDG state between processes, recording reports elsewhere.
The pre-width candidate repeated at **1.097 and 11.867 ms**; retain the large result. Frame-phase
aliasing is a plausible explanation, not a demonstrated cause. Schemas/fixtures are not pooled.
The final same-path ABBA block used the rebuilt binary, 100,000 lines and 30 measured rounds:

| Run | Edit first XDamage | Undo | Redo | Settled RSS |
| --- | ---: | ---: | ---: | ---: |
| Baseline A1 | 1.320 ms | 2.156 ms | 8.828 ms | 204.60 MiB |
| Final B1 | 1.007 ms | 1.966 ms | 8.856 ms | 204.01 MiB |
| Final B2 | 2.620 ms | 2.147 ms | 8.799 ms | 210.20 MiB |
| Baseline A2 | 2.556 ms | 6.235 ms | 10.269 ms | 226.07 MiB |

**Precise GUI edit-latency preservation remains unverified.** Final differences vary in direction;
this series establishes neither a dependable slowdown nor a speedup. No speculative production
optimization was made to chase observer/process variation.

Final GUI output stress ran **15.031 seconds**, with tab switches, resizes and exact quiet-tab
command effects: **75.04%** parent CPU of one core, **195.61/182.06 MiB** peak/settled RSS, versus
baseline **75.98%** and **194.78/180.43 MiB**. Settled CPU was one tick over the two-second interval
in both. TUI output stress retained three 15-second pairs. An initial CPU increase on unequal
throughput did not recur: normalized repeat CPU per million completed producer lines changed
approximately **−1.3% and +0.4%**, with near-equal throughput. All TUI idle output/CPU was zero,
final-quarter RSS was stable, bytes and ownership/mode restoration passed. Single GUI pairs and
these short runs do not establish allocation attribution or leak freedom.

The release Find model probe recorded 480 queries over 10,000 history lines and 32 maximum-retention
queries (512×256 viewport plus 128 history rows, eight marks/cell, 1,021-byte repetitive-prefix miss).
Maximum-retention median/max search was **2.911/3.275 ms**; RSS **56.45→56.57 MiB**. A separate
**10.033-second** output probe recorded 623 query-feedback samples at **20.138 ms median** and
an exact input acknowledgement at **10.139 ms**. Its 10 ms polling and coalesced output mean feedback
is not completed-search or display time. Final quiet matching and shutdown passed.

Local ignored evidence is under `perf-results/terminal-polish/`: detailed report, raw samples and
commands, all failed/inconclusive attempts, source/executable identities, prior binaries, native
screenshots and review repro. Earlier samples remain tied to their earlier binary; final GUI mapping,
fixed-path editing and terminal-open stress use the rebuilt final binary. Native Wayland/macOS,
physical presentation, GPU memory, authenticated agent conversations and longer soaks remain
unverified. Linux terminal ownership and host modifier-reporting limits still apply.

## 0.0.7 release comparison (2026-10-29)

Paired release runs against `v0.0.6`; tables and commands are in the
[0.0.7 performance report](code/release-0.0.7-performance.md).

| Experiment | Outcome and evidence |
|---|---|
| Throttle results-row rebuilds during a streaming project search to once per 50 ms, counting folder children without a hash set | **Kept.** TUI search-to-done on 9,000 matches in 1,800 files ~21 → ~18 ms; Down+frame unchanged (0.12–0.13 ms). The final rebuild still runs on completion, so the finished list is exact. |
| Attribute the search-to-done increase since 0.0.6 (TUI 6.5 → ~17 ms, GUI 7.7 → ~26 ms) | **Attributed, not fixed.** A profile shows SHA-256 per searched file, used for stale-result identity added with editing; present before the results-panel changes. Revisit with a cheaper identity (size + mtime first, hash on mismatch) if large-project search latency matters. |
| Replace that SHA-256 with XXH3-128 (`twox-hash`, already in the lock) | **Unsuccessful; reverted.** Three release runs each: TUI search done 17.0–18.3 ms, GUI 23.8–26.2 ms, the same as with SHA-256 (17.0–19.0, 25.3–25.9). Only files with matches are hashed, and the hash is not what costs the time; the earlier attribution was wrong. A whole-process profile of the probe is dominated by tree-sitter query setup and ratatui drawing, not the search workers, so the remaining increase is not attributed. Samples: `perf-results/release-0.0.7/results-panel/*-xxh-*.jsonl`. |
| Terminal input stalls under a burst (QA: the caret stopped mid-burst and caught up on the next key) | **Fixed.** A PTY probe typing 3,000 characters in one write stopped after exactly 1,024 and released 1,024 more per later key: crossterm 0.29's mio source reads one 1 KiB chunk per edge-triggered readiness and returns its first event, leaving the rest unread. Its `use-dev-tty` source polls level-triggered (`filedescriptor`, already in the lock; `select` on macOS). The burst now completes in 0.62–0.85 s with no output gap over 2 ms. Three paired collector runs, old then new: input feedback median 0.78–1.01 vs 0.78–0.95 ms, stress input 0.80–0.85 vs 0.80–0.86 ms, first content 23.3–23.9 vs 23.5–24.1 ms, settled RSS 50.0–50.2 vs 50.8–51.8 MiB (about 0.8 MiB more). A burst still redraws once per key (about 3,800 keys/s); coalescing input before a draw is untried. Samples: `perf-results/release-0.0.7/input-burst/`. |
| Lenz Diff 0.0.7 against 0.0.6 (full suite, two alternating pairs) | **No reproducible regression above a millisecond.** Five of 391 metrics rose 10% or more in both pairs, all under 0.5 ms except one GUI merge-undo outlier (6.4 → 29.7, then 7.5 ms). A 0.0.7 build without `use-dev-tty` measured with the standalone TUI and workflow collectors overlaps 0.0.6 and 0.0.7 on input, resize, startup and RSS; only the history filter stayed about 0.15 ms higher with the new source (inconclusive). The release report has the table. |
| Attribute GUI startup +18 ms and TUI 100k-line RSS +14 MiB | **Attributed to the integrated-editor branch.** A TUI binary from before the results-panel changes already measured 22.6 ms first content and 50.2 MiB settled. The TUI memory cost was accepted earlier; the startup cost is reported for this release, not bisected across the 726 commits. |
| GUI edit collector against 0.0.6 | **Not comparable.** 0.0.6 cannot edit, so the collector fails with no paint feedback; 0.0.7 values are recorded alone. |

## 0.0.8 release comparison (2026-10-30)

Paired release runs against `v0.0.7`; the table and commands are in the
[0.0.8 performance report](code/release-0.0.8-performance.md).

| Experiment | Outcome and evidence |
|---|---|
| gpui v1.24.2 → v1.26.2 with the dependency updates (Code and Diff GUI, Code and Diff TUI, two alternating pairs) | **No regression of a millisecond or more.** Code GUI open 158.6–160.8 ms against 159.6–160.0; Diff GUI first paint 252–253 against 250–251.5 ms; Code TUI first feedback +0.7 ms and Diff TUI input +0.3 ms in both pairs, recorded and not pursued; Code GUI edit-run peak RSS +1–3 MiB. |
| Diff terminal first-paint marker | **Collector fix.** Since 0.0.7 the header names the comparison, so `tui-bench.py`, `markdown-bench.py` and `crash-reporting-check.py` waited for a `Lenz` that never came; they now wait for the status line's `F2 details`. Both builds were measured with it. |
