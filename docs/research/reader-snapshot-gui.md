# Reader GUI snapshot windows

The GUI reads source text through the current `View` snapshot and the shared resumable
projection/layout services. Editing remains disabled. This report covers the GUI slice
of migration 08; the integrated 08–10 local gate is still pending.

The subsequent native Go To gap is fixed in runtime source
`1094aef84eecf33b869ea519f5d5ad0813b7cd0a`. The same native replay now paints `END`
after 2,000 family emoji, selects/copies its exact bytes and leaves the file unchanged,
including 100 trailing spaces in a 600×300 window. The old executable still fails
that identical replay. The complete 08–10 gate remains pending.

## Behavior and bounds

Paint, pointer conversion, selection, find tints, syntax/semantic styles, links,
documentation anchors, folds and sticky lines use absolute source coordinates. Native
fragment origins are shared by paint and hit-testing. Cold horizontal/wrapped seeks
continue through the existing Jobs service; they do not finish a complete prefix on
the UI thread. The adapters do not materialize current text for these operations.

A shared source-byte reveal goal is resolved through the same bounded native walker.
The core validates source, selection, metrics and request identity before choosing
horizontal movement with a three-column context margin. Native-overwide wrapped rows
can pan horizontally; ordinary wrapped targets unwind that pan. Exact native extents
include the drawable EOF caret, and opaque viewport stamps reject stale clamps.

Geometry rejects a changed snapshot or source-metadata identity, hints, folds,
declarations, viewport, scroll position, pane layout or relevant chrome. In particular,
scrolling to another already cached line cannot reuse the preceding frame's hit map.
Image pixels and failed-SVG source fallback retain their existing presentation policies.

The shared [logical layout report](reader-snapshot-layout.md) describes the 4,096-byte
wrapped-body ceiling, 2,048-column continuation-indent limit and pathological grapheme
fallback. The GUI shapes bounded native fragments: projection work 4,096 bytes plus at
most 257 bytes of grapheme context; native paint/decoration helpers assert a 6,144-byte
input ceiling, including bounded indentation. Copy reads original snapshot bytes.

Empty projected source windows are compacted into source-interval/display-boundary
mappings. They do not become empty pieces or join the retained grapheme's window list.
A real 50,000-mark source followed by `END` retained 966 window references before this
repair; the regression now requires at most 64, reaches `END`, and preserves the
caret boundary inside the suppressed source. This is a cache-ownership check, not a
process-memory measurement.

## Correctness evidence

Focused GUI constructor/input/render checks exercise long-line horizontal selection,
wrapped End/Up and exact clipboard bytes, native emoji boundaries, obsolete reload
metadata release, stale scroll geometry, suppressed-source caret mapping, hints,
sticky lines, images/SVG fallback, empty views and narrow layouts. The combined
initial combined run passed 32 tests. The final focused batch passed 44 with 117
filtered out, including native Go To/Find/copy, wrapped pan and stale font geometry. Strict GUI Clippy passed, including
`test-support` and `perf-probe`; a subsequent bounded fold-hit change passed the
20-test fold/sticky group. The final integration gate remains separate.

The combined run caught a failed-SVG regression: source preparation treated every
image metadata value as a rendered image. The eligibility check now distinguishes a
rendered/unshowable raster from an SVG that falls back to source. The existing image
workflow test went red and then passed with the other 31 focused checks.

An initial whole-prefix emoji oracle accumulated native `f32` rounding: 171896.47 pixels
for an unbounded prefix versus 171912.19 for bounded advances, against 171912.01 from
repeated glyph widths. The corrected check uses the painted literal's native position,
checks its advance against the independent glyph-width calculation, then verifies
exact original source selection/copy. An ad hoc command that omitted `test-support`
ran zero GUI tests and is excluded from passing evidence.

## Equivalent release comparison

Baseline source: `1d874aee50c620c10201d0695de0e97fe140a83a`.
Measured candidate: `5de351a9ec8c45ec530d48d5af7eb1c2954be938`.
The latter includes the shared source-read optimization and strict logical-row bounds.
Later test-only coverage and the focused SVG/fold-hit fixes are recorded separately;
these measurements precede final integration with analysis 10.

Both executables used `cargo build --release --locked -p reader-gui --features perf-probe`
without release-profile overrides. Dependency artifacts were reflink-copied into the
GUI's private target; workspace package artifacts were removed before rebuilding,
avoiding reuse of a baseline workspace library. Instrumentation is excluded from
ordinary production builds.

The collector uses its own 1600×1000 Xvfb server and Mesa software rendering, isolated
settings/state/cache and disabled providers. Each of ten Rust files contains 2,000
one-line functions; this differs from the older README fixture. There are three
unmeasured warmups before eight measured launches per cache mode. A fresh cache is a
new application cache, not a cold OS/shader cache. The parent timer starts after
process creation and ends on the first-readable-frame marker, before text paint/GPU
presentation. These endpoints are unchanged between the two executables.

| Workload | Baseline | Candidate |
| --- | ---: | ---: |
| Sequential warm launch median, 8 samples | 141.08 ms | 142.17 ms |
| Sequential fresh-cache launch median, 8 samples | 142.40 ms | 144.23 ms |
| Alternating warm launch median, 8 pairs | 142.45 ms | 142.39 ms |
| Alternating internal readable marker median | 140.45 ms | 140.30 ms |
| Ten kept tabs, settled RSS | 172.50 MiB | 172.03 MiB |
| Ten kept tabs, sampled process high-water mark | 172.50 MiB | 172.03 MiB |
| 15-second unwrapped scroll, frame count | 314 | 314 |
| Unwrapped frame interval median / p95 | 48.00 / 48.05 ms | 48.00 / 48.04 ms |
| 15-second wrapped scroll, frame count | 314 | 314 |
| Wrapped frame interval median / p95 | 48.00 / 48.03 ms | 48.00 / 48.08 ms |

The first sequential startup pair was slightly slower; eight alternating pairs did
not reproduce that direction. No GUI performance tradeoff is accepted or requested
from these samples. Scrolling intervals describe this software-rendered environment:
all 313 intervals in each run exceed 25 ms. They are not 60 Hz or physical-display
latency claims. Memory sampling lasted about 60 seconds per executable; settled RSS
is the median of the final 60 half-second samples. Fixture hashes were checked again
after all workloads and were unchanged.

## Native reveal oracle

Owned X11 input enters Go To 1:14002, captures the native source row before further
caret movement, then selects and copies `END` through the actual X11 clipboard.
The 600×300 fixture has 100 spaces after the target, so an EOF clamp alone cannot
satisfy the check. Source bytes are asserted unchanged.

The original full-window OCR missed visible text beside emoji. Oracle version 4 uses
a fixed right source band, grayscale/inversion/scaling/padding and case-insensitive
OCR. The retained screenshots were inspected, and the identical version 4 driver was
replayed against both old and new executables: old reports `painted_END=false`, new
reports `painted_END=true`; both copy `END`. Failed oracle versions remain retained.
This is a correctness check, not a timing measurement. Final performance integration
with analysis 10 remains separate.

## Native diagnostic and retained artifacts

`READER_PROBE_NATIVE` enables the optional `READER_NATIVE_WIDTH_V1` diagnostic in a
probe build. Bounded samples on the owned native display measured Hangul choseong
filler plus U+200B at 11.96 pixels / 2 logical columns, soft hyphen plus U+200B at 0 / 0,
and the family emoji at 16.19 / 8. The same choseong sample measured 15.6 pixels in
TestPlatform. Thus the attempted positive-logical-width/zero-native-advance fixture
was not reproduced in the tested default font/backend; the test platform cannot
substitute for native glyph measurements. This is not a guarantee for arbitrary
fonts or scripts.

Raw samples, logs, fixture hashes, exact probe executables and source/executable
identities are under `target/perf/editor-implementation/08/`:

- `paired-baseline/`, `paired-candidate/`: complete startup/memory/scroll collectors.
- `startup-pairs/`: alternating launches and stderr for every trial.
- `native-diagnostic/`: exact measured candidate binary, identity and native widths.
- `native-goto-oracle-v4/`, `native-goto-old-oracle-v4/`: final native positive/negative replay.
- `native-goto-final/`: immutable final native probe executable.
- `root-gui/`: focused GUI red/green and lint evidence.
- `core-checks/`: shared projection/layout bounds and real Session continuation checks.

Native macOS, physical-display latency, GPU memory, arbitrary-font shaping and
long-session behavior remain unverified. The complete 08–10 gate and subsequent
editing/provider work are not claimed complete by this scoped report.
