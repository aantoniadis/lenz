# Bottom commit panel: verification

Linux release comparison on 2026-10-17 against `9f440be4`, using preserved
executables from before the layout change. In Focus, the commit message now spans
the bottom beneath Files and the diff. **Commit** hides it completely and restores
it independently of Files, through the same shared action in both adapters.

## Correctness

`scripts/ci-local.sh` passed: workspace/core/TUI tests, 253 GUI adapter tests,
Reader GUI tests, clippy, panic budgets, Python collector checks (50 tests, one
skipped), and the release size gate (67,950,264 bytes stripped). The TUI was also
built separately in release mode.

Adapter checks cover full-width geometry, pointer selection/copy, keyboard entry
and exit, Summary focus, hide/restore, independent Files visibility, modal input
isolation, and resize/scroll clamping. The 400-pixel GUI footer toggle is tested
with an actual pointer event. TUI rendering covers three appearances and sizes
down to 1×1. Private-display production checks verified light/dark GUI and TUI
rendering, hide/restore, and the narrow GUI footer. Fixture bytes stayed intact.

## Measurements

Disposable repository: two commits, a 3,000-line Rust file changed every 80 lines,
a second small file, and an 80-line commit message. Configuration was isolated.
GUI probes used private Xvfb with Mesa lavapipe; terminal probes used an owned
120×36 PTY. GUI interaction windows were 1536×1000; stress windows were 1200×800.
PTY timing retained `NO_COLOR=1` consistently; appearance checks enabled color.

Startup used eight alternating pairs after two warmup pairs. GUI startup measures
the built-in first-paint marker for an explicit revision comparison, before Focus;
TUI startup measures first log comparison content. Interaction follows entry into
Focus, with the commit body verified visible. GUI feedback ends at first XDamage,
TUI feedback at received bytes; neither establishes completed background work or
physical display latency. Interaction rows are medians of per-run medians (three
GUI pairs, eight TUI pairs).

| Measurement | Before | After |
|---|---:|---:|
| GUI first paint | 138 ms | 141 ms |
| GUI startup settled RSS | 200.8 MiB | 200.4 MiB |
| GUI alternating Page Down/Up, no selection | 11.89 ms | 11.93 ms |
| GUI Find next, changed-line `MARKER` query | 11.50 ms | 11.54 ms |
| GUI settled RSS after interaction | 228.4 MiB | 227.7 MiB |
| TUI first content | 19.23 ms | 18.90 ms |
| TUI alternating Page Down/Up | 0.486 ms | 0.451 ms |
| TUI settled / peak RSS | 15.33 / 21.56 MiB | 15.35 / 21.75 MiB |

Repeated palette toggling and scrolling ran for 34.7 seconds per GUI build
(seven rounds), with RSS 208.0→209.3 MiB before and 191.7→193.0 MiB after.
TUI runs lasted 30.1/30.0 seconds (103 rounds each), with RSS 15.6→15.6 MiB
and 15.8→15.9 MiB. These short runs do not establish leak freedom or a general
memory improvement; RSS excludes hardware GPU memory.

## Investigations and limits

- Sequential Page Down initially appeared slower: the larger old diff viewport
  reached the end earlier, making later inputs cheap redraws. Alternating direction
  keeps movement in the measured workload; its metric is versioned separately.
- A broad `e` query plus page keys measured about 6.5→11.2 ms. Screenshots revealed
  different selected hits and rendering: expanded unchanged lines before, folded
  highlighted changes afterward. Page keys move the search caret using viewport
  height, so identical keys do not imply identical work. This result remains a
  confounded observation. Changed-line `MARKER` navigation is comparable above;
  expanding all unchanged lines also brings both builds' `e` navigation back to
  approximately 6.5 ms. These controls support a layout-dependent workload
  explanation, not a general Find slowdown. Exact hit sequences still differ.
- Caching commit-panel height once per frame did not improve the suspect workload
  and was reverted. No speculative cache ships. Plain-scroll profiles were
  dominated by software Vulkan rendering with similar total sampled cycles.
- Earlier probes that did not enter Focus were excluded. The combined scenario's
  whitespace/highlight toggles can start background recomparisons; isolated Find
  probes avoid those transitions.

Raw samples, source/executable SHA-256 identities, scripts, screenshots, profiles,
and logs remain in local ignored `target/commit-panel-evidence/`: `startup.jsonl`,
`interaction-*.jsonl`, `marker-*.jsonl`, `expanded-*.jsonl`,
`terminal-and-stress.json`, `identity.json`, and `ci-final.log`. The application
source patch matches the successfully gated build. The height-cache experiment
is retained separately as rejected evidence. Reproduce with the fixture and
drivers there, following the [benchmark guide](benchmarking.md).

## Hardware GPU and 15-minute follow-up

The follow-up used hardware Vulkan through RADV on an owned headless Sway
compositor and Xwayland server. The user's display received no synthetic input.
Xvfb was excluded from this hardware trial: merely selecting the hardware ICD
opened a DRM descriptor but failed presentation and fell back to CPU rendering.
The accepted runs verified graphics-engine activity in the application process's
DRM descriptors throughout, deduplicating descriptors by DRM client identity.

Eight alternating startup pairs after two warmup pairs measured log first paint
at **137 ms before / 134 ms after**. These use a different display and opening
scenario from the earlier software-rendered startup row; do not compare the
absolute values across those trials.

Each build then ran for 15 minutes in one GUI process and one TUI process. A round
hides and restores the message through the command palette, then alternates Page
Down/Up. Every tenth round resizes narrow and back. GUI screenshots periodically
verify message absence/restoration; each scroll must change pixels in the diff.
The TUI reconstructs visible cells to verify the message on every toggle and
checks terminal restoration on exit. Both assert unchanged fixture bytes.

| Sustained workload | Before | After |
|---|---:|---:|
| Hardware GUI actual duration / rounds | 901.1 s / 688 | 900.6 s / 688 |
| GUI RSS, first → final sample | 104.34 → 105.23 MiB | 104.56 → 105.55 MiB |
| GUI kernel peak RSS | 105.23 MiB | 105.55 MiB |
| GUI final-quarter RSS increase over preceding quarter | 0.25 MiB | 0.25 MiB |
| App DRM VRAM, first → final sample | 81.04 → 81.04 MiB | 81.04 → 81.04 MiB |
| App DRM GTT, first → final sample | 13.00 → 15.70 MiB | 13.00 → 15.70 MiB |
| GUI first observed diff-pixel change, median / p95 | 13.16 / 15.72 ms | 13.04 / 14.82 ms |
| TUI actual duration / rounds | 900.0 s / 2,584 | 900.0 s / 2,592 |
| TUI RSS, first → final sample | 15.54 → 15.55 MiB | 15.64 → 15.80 MiB |
| TUI kernel peak RSS | 20.73 MiB | 17.70 MiB |
| TUI final-quarter RSS increase over preceding quarter | 0 MiB | 0 MiB |
| TUI feedback, median / p95 | 0.529 / 0.934 ms | 0.455 / 0.787 ms |

All four runs passed. Additional short hardware-rendered checks verified light
and dark appearances, the bottom panel and hide/restore on both Xwayland and
native Wayland; screenshots were inspected. The 15-minute GUI trials above used
Xwayland, not native Wayland.

GPU allocations plateaued after small warmup steps; RSS
showed small steps of similar magnitude in both GUI builds. Application logs
contained no device-loss, panic, or out-of-memory diagnostics. Process CPU during
the sampled GUI interval was 53.1 / 51.3 seconds; graphics-engine activity was
12.0 / 13.4 seconds. These are single-run diagnostics with different visible
layouts and uncontrolled GPU clocks, not an energy or GPU-throughput comparison.

The hardware GUI feedback endpoint polls captured diff pixels after virtual
keyboard input. It includes input transport, polling and image readback; it is
an upper bound on first observed pixel change, not the earlier XDamage metric,
completed presentation or input-to-photon latency. The 688 inputs within one
process are not 688 independent process trials. TUI stress overlapped the GUI
trials; other benchmark processes did not. The physical GPU is shared with the
desktop even though displays and input are isolated.

DRM VRAM/GTT values are per-client accounting and can include shared buffers;
they are not exclusive physical allocations and must not be added to RSS.
These runs test sustained interaction and resource reuse, not maximum GPU load
or all application workflows. Fifteen minutes cannot establish leak freedom.

Follow-up evidence is under local ignored `target/commit-panel-evidence/gpu-long/`:
`identity.json`, `hardware-startup.json`, `long-{gui,tui}-{before,after}.json`,
`summary.json`, logs, screenshots, and the private-compositor/input drivers.
Source bytes match the previously gated application patch and executable hashes
match the preserved baseline/candidate identities. No product code changed.

Native macOS, native Wayland long-duration stress, physical presentation latency,
and longer soaks remain unverified.
Existing platform results elsewhere are not new measurements of this panel.
