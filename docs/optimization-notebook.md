# Optimization notebook

> Dated measurement reports and their raw evidence were retired from the tree before release; they remain in the private development history.

Evidence and decisions from the 2026-10-03 Linux performance work. Keep unsuccessful and superseded approaches here so a later platform, renderer or workload can justify another trial. Results are specific to the machine and measurement method; they are not universal performance claims.

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
| One shared horizontal offset | Replaced. Native right-pane input changed 67 samples of the left source rectangle; separate offsets preserve every sampled left frame. |
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
