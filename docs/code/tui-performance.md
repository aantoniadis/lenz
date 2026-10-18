# Reader TUI workspace integration — 2026-10-18

The terminal reader now uses this workspace's `reader-core` and `lenz-content`.
The command-family trial exposes it through `lenz --tui [file-or-folder]` and
`lenz-code-tui`. Release packaging remains a separate migration.

## Correctness and integration

`scripts/ci-local.sh` passed: strict Clippy, separate Diff/Reader/TUI panic-site
budgets, workspace/core tests, serial content memory checks, actual Diff and
Reader GUI adapter tests, terminal event/render tests, the combined GUI/TUI
semantic replay, Python collectors, independent terminal release builds and
the existing stripped Diff executable size check.

The reader TUI suite has 26 passing tests with `gui-test`, including real
constructors, empty documents, dark/light and narrow rendering, modal capture,
UTF-8 selection/copy, Markdown/images, stale results, cancellation and independent
views. The shared replay checks selection, find, splits, reload and modal outcomes
through both adapters' inputs, together with unchanged source bytes. This covers
those scenarios, rather than claiming every possible interaction is equivalent.

The staging script built all four applications. An owned PTY verified that the
actual `lenz --tui` launcher replaces itself with the reader TUI, shows the source,
opens/dismisses Help, preserves source bytes and restores termios and the alternate
screen on exit. Both terminal help routes select their respective applications.
The standalone reader TUI's normal dependency tree contains no GPUI or GPU crates.

One initial **pre-change** reader test run timed out in
`a_typescript_7_project_is_served_by_typescript_go_from_its_own_tsc` while waiting
for its fake provider. The subsequent combined reader suite and full local gate
passed. Its intermittent cause was not diagnosed by this port.

## Method and retained evidence

Local raw evidence is under `target/reader-tui-port/`: full gate/build logs,
`launcher-smoke.json`, raw terminal output, process/timing samples, fixture hashes,
binary hashes and source identities. The integration starts from `3cca626` plus
the pre-existing worktree changes saved in `preexisting.patch`; those changes were
preserved. No commit or installed configuration was changed.

GUI comparisons use immutable release `perf-probe` copies built before and after
the port. `paired-gui-fresh.py` alternates their order, uses a fresh owned Xvfb
display for each launch at 1600×1000 with Mesa lavapipe, isolated configuration,
disabled providers, warmed OS/shader caches and one shared reader cache. Each case
has one warmup pair and five measured pairs. Startup ends at the content-frame
marker, before physical presentation. The small fixture is a three-line Rust
program; the large fixture is 100,000 plain-text lines.

`gui-scroll.py` adds one warmup pair and three measured pairs of scrolling the
large fixture. Each run records 63 intervals spanning about 3.024 seconds. The
software renderer's 48 ms intervals are observed values, not an assumed refresh
rate. Process CPU is cumulative from launch through the last sample. RSS is
sampled from Linux `/proc`, excludes GPU allocations and child processes, and is
not proof of leak freedom.

| GUI measurement, median | Before | After |
|---|---:|---:|
| Small source, content marker | 151.77 ms | 147.44 ms |
| Large text, content marker | 148.29 ms | 148.63 ms |
| Small source, RSS at marker | 157.68 MiB | 156.21 MiB |
| Large text, RSS at marker | 155.63 MiB | 157.54 MiB |
| Scroll interval | 48.001 ms | 47.999 ms |
| CPU through scroll sample | 1.87 s | 1.79 s |
| Scroll process high-water RSS | 173.21 MiB | 173.38 MiB |

These repeated samples show no consistent GUI timing or memory regression for
the measured workloads. The final RSS sample varied with renderer teardown;
it is not presented as settled idle memory.

The terminal comparison uses the existing standalone reader TUI as its baseline,
with its own source identity retained in `standalone-source-identity.json`.
That baseline includes the prior reader TUI work and its older shared-content
dependency; it is not a clean checkout of this monorepo's base commit. The
candidate uses the staged `lenz-code-tui` alias and local shared-content crates.
Both run the same disposable 100,000-line fixture in an owned 120×36 PTY, isolated
HOME/XDG/provider state and Darcula appearance. Timing ends at PTY bytes received
by the collector, not a terminal emulator's displayed pixels.

The collector checks palette filtering/disabled reasons, modal isolation, OSC 52
copy, goto/find, independent views, intentional external modification/reload,
idle output/CPU, sizes down to 1×1, sustained page navigation, exact file bytes and
normal terminal cleanup. Its directory-named-`*.rs` identity failure was reproduced,
fixed by hashing only files and covered with a red/green Python regression test;
the failed attempt and logs are retained. Metric semantics remain schema 1.

`tui-startup.json` contains one warmup pair and seven alternating measured pairs.
The full collector separately records 100 arrow-key samples and one 60-second
page-navigation run per executable. Both stress runs completed 740 cycles in
60.04 seconds. They emitted zero bytes and accumulated zero measured CPU ticks
during their approximately 0.50-second idle samples.

| Terminal measurement | Standalone baseline | Workspace candidate |
|---|---:|---:|
| First frame, median of seven launches | 2.87 ms | 3.20 ms |
| First source content, median of seven launches | 6.55 ms | 6.76 ms |
| Arrow-key feedback, median / p95 | 0.485 / 0.643 ms | 0.490 / 0.738 ms |
| Sustained page feedback, median / p95 | 0.500 / 0.721 ms | 0.499 / 0.739 ms |
| Largest resize feedback sample | 0.50 ms | 0.49 ms |
| Process high-water RSS | 41.34 MiB | 38.21 MiB |
| RSS after stress and 0.25-second quiet interval | 37.46 MiB | 35.89 MiB |

Both runs passed all correctness assertions and restored termios. RSS was
unchanged between the final resize and the end of stress in each executable.
This short-run plateau is not a long-session leak test. The candidate's early
frame was about 0.32 ms later; source-content sample ranges overlap. Because the
standalone baseline uses an older shared-content version, these numbers describe
the integrated product comparison and do not isolate the cost of moving crates.

Reproduction from the workspace root:

```sh
scripts/try-launcher.sh
python3 -B scripts/code-tui-bench.py \
  --binary target/launcher-trial/lenz-code-tui \
  --out target/perf/code-tui-check \
  --lines 100000 --rounds 100 --stress-seconds 60
```

The normal GUI release executable was rebuilt after preserving the probe copies;
benchmark instrumentation is not left in the staged GUI application. After the
collector fix, the complete Python collector suite passed again (63 tests,
one platform-dependent skip). Documentation links and `git diff --check` passed.

## Limits

Native macOS, hardware GPU rendering, physical terminal appearance/display
latency, installed language-server combinations, cold OS caches, the full Diff
performance matrix and long sessions remain unverified here. The terminal guide
documents clipboard, shortcut, image and pane-layout limits. No release/install
migration was attempted. Historical reader measurements remain in
[the earlier performance record](performance.md); these integration measurements
use different fixtures and should not replace or be directly compared with them.


## 2026-10-18 menu parity and LSP recovery

The terminal reader now uses the GUI's shared navigation controls beside its tabs and the same contextual
commands on right-click. Left-click selects source text. Both readers expose persistent LSP status and a
scrollable recovery dialog with shared availability, full errors, provider setup and Restart/Enable/root actions.
Missing, disabled, failed, starting, indexing, unsupported and large-file states remain distinct; setup and
activation stay user-initiated. Recovery rows retain command identity while background status changes.

The full local gate passes, including workspace/core tests, 253 comparison GUI tests, 100 reader GUI tests and
32 reader TUI/combined parity tests. Tests exercise actual input, split/context behavior, preview context,
modal isolation, tiny resizes, source bytes, server crash/restart, missing setup and disabled/re-enabled providers.
The exact staged `lenz --tui` launch passes an isolated PTY smoke test with colored output, F2, Restart and
terminal cleanup. Private Xvfb screenshots cover dark/light menus and narrow recovery dialogs.

Raw samples, PTY output, executable/source identities and scripts are retained locally in
`target/perf/reader-menu-lsp/`. `baseline-identity.json` corrects the old binary's source attribution; the generic
collector records the current checkout at collection time. The retained pre-session GUI perf-probe corresponds
to the baseline production GUI: its recorded production SHA-256 matches `gui-production-before`. The measured
candidate source archive reproduces its recorded source digest. Final copy cleanup removes repeated setup
instructions and fixes the dialog footer; it does not change the measured startup or scroll paths.

| Terminal measurement | Before | After |
|---|---:|---:|
| Tab feedback, median of ten alternating launches | 2.88 ms | 3.08 ms |
| First source content, same launches | 6.44 ms | 6.60 ms |
| Arrow feedback, median of 50 inputs | 0.69 ms | 0.46 ms |
| Sustained page feedback, median | 0.51 ms | 0.50 ms |
| Stress duration / cycles | 15.03 s / 185 | 15.01 s / 185 |
| Settled / peak RSS | 36.07 / 41.30 MiB | 35.66 / 39.12 MiB |

Both terminal runs emitted zero idle bytes and zero sampled idle CPU ticks. An initial startup comparison used
the old `Lenz Reader` marker: it moves from the first row to the footer (116 versus 4,945 bytes into the
retained first transcript). Its arrival is later and is spatially confounded. The controlled repeat adds an
explicit tab-label feedback endpoint and retains first-content timing; it does not redefine the collector's
schema-1 metric. Initial startup pilot metrics are retained, but its raw transcripts were superseded by the
controlled repeat. Terminal measurements end at collector bytes, including decoding, rather than display.

| GUI measurement | Before | After |
|---|---:|---:|
| Small source content, median of nine alternating repeat pairs | 144.92 ms | 145.23 ms |
| Large source content, median of five pairs | 143.13 ms | 144.37 ms |
| Small source RSS at marker, repeat | 157.97 MiB | 157.41 MiB |
| Scroll interval, three measured trials | 48.00 ms | 48.00 ms |
| CPU through scrolling | 1.71 s | 1.72 s |
| Process high-water RSS through scrolling | 172.12 MiB | 172.55 MiB |

GUI probes use fresh private 1600×1000 Xvfb displays, Mesa lavapipe, isolated settings, a shared warmed reader
cache and alternating executable order, with warmups excluded. Each scroll trial spans about 3.024 seconds.
The first five-pair small-file median was 134.44 versus 146.46 ms; the nine-pair repeat did not reproduce that
difference. The checked workloads show comparable content latency, input responsiveness and memory.
These are short checks, not a long-session or leak guarantee. Native macOS, physical display latency, hardware
GPU behavior and installed-provider combinations beyond existing provider tests remain unverified.

## 2026-10-18 in-app Java runtime repair

Both readers now expose **F2 → Choose Java Runtime…**, sharing discovery, executable validation, settings
persistence and restart in the core. Validation runs on the existing workers only after selection, with a
three-second timeout. Installed runtime discovery runs only while requested. No server or runtime is installed.

The full local gate passed, including core/adapter tests and a GUI/TUI replay that produces identical provider
settings. Focused tests cover bad paths, failed/hung executables, malformed settings, write failure, preservation
of unrelated settings and file permissions, cancellation, disabled providers and narrow/1×1 layouts. GUI and
terminal dark/light chooser captures were taken on an owned Xvfb display. Native macOS and Windows remain
unverified.

The actual staged `lenz --tui` launcher reproduced JDT LS exiting under Java 21, then repaired it through the
chooser by selecting the installed Java 26 executable while ambient `JAVA_HOME` remained Java 21. Saving took
about 21 ms and the restarted server reached ready in about 2.09 s. Its copied report contained the persisted
Java 26 argument; it can subsequently show indexing as background project loading proceeds. Source bytes
were unchanged, and termios/alternate-screen state was restored. This exercises a disposable plain Java source
folder, not Maven/Gradle import or the user's repository. Runtime validation checks successful execution of
`-version`; compatibility with the installed JDT LS version is confirmed by its subsequent status.

Raw samples, collector scripts, source patch/new module, executable identities, screenshots and full gate/build
logs are retained locally under `target/perf/java-runtime-ui/`. Production executables were compared directly:
10 alternating TUI pairs and one GUI warmup pair followed by seven alternating GUI pairs opening the same disposable 100,000-line text fixture,
isolated XDG settings, warmed OS caches, and a fresh private display for each GUI launch. GUI uses lavapipe.
The GUI endpoint detects the first non-uniform client image; it is neither completed file loading nor physical
presentation. Dialog feedback detects changed pixels and includes capture overhead. TUI endpoints are PTY
receipt of source text and arrow feedback. These timings are distinct from the earlier instrumented GUI markers.

| Measurement, median | Before | After |
| --- | ---: | ---: |
| TUI source feedback | 7.079 ms | 7.009 ms |
| TUI arrow feedback | 0.472 ms | 0.471 ms |
| TUI RSS after initial arrows | 13.41 MiB | 13.40 MiB |
| GUI first visible client paint | 177.66 ms | 172.14 ms |
| GUI dialog feedback | 21.91 ms | 25.75 ms |
| GUI RSS after initial dialog cycles | 174.59 MiB | 175.43 MiB |

Fifteen-second F2/Escape stress completed 182 cycles for both TUIs (0.16 s cumulative process CPU each);
post-stress RSS was 13.61 → 13.25 MiB. The final GUI run completed 178 → 200 cycles in 15.06 → 15.01 s;
cumulative process CPU was 10.54 → 12.08 s (about 0.059 → 0.060 s per cycle). Post-stress RSS was
174.66 → 176.79 MiB and was flat from the first stress sample. A prior stress pair completed 206 → 205 cycles,
so the feedback/throughput timing shifts did not repeat consistently.

The small GUI RSS increase repeated. Four additional alternating memory pairs measured anonymous memory
51.98 → 51.93 MiB before opening a menu and 52.77 → 52.75 MiB after three open/close cycles. Post-menu file-backed
RSS was 97.09 → 97.81 MiB and shared RSS 24.62 → 24.86 MiB. This isolates the observed increase mainly to
mapped file/renderer pages, without proving exact RSS preservation. The same repeat measured dialog feedback
24.99 → 24.47 ms, rather than reproducing the earlier increase. RSS excludes GPU allocations and server
descendants; short plateaus do not establish leak freedom.

The first GUI collector attempt incorrectly expected F2 to dismiss a modal and failed its feedback assertion;
the retained corrected collector uses Escape. An initial comparison included cold shader-cache samples; it is
retained separately and the final comparison includes a warmup pair. Long sessions and native hardware
presentation are unverified.

## 2026-10-18 per-directory workspaces

The shared core checkpoints a folder's workspace into its existing places store on folder switches and normal
close/quit. It restores up to 200 file tabs, their order and retention, selection/caret and scroll positions,
source/preview choices, split panes, soft wrap, panel sizes and explorer state. Only shown tabs load immediately.
The existing debounced saver handles writes and a flush fence orders a quick folder reopening after its save;
no worker is added. Optional workspace metadata is boxed so it does not enlarge normal completion payloads.

The full local gate passed. Core tests cover lazy loading, two directory switches, missing files, UTF-8 position
clamping, late restore rejection after explicit file navigation and closing before a restore arrives. Existing
places tests cover old files, unreadable/foreign-version state and independent windows merging bookmarks. A real
input TUI → GUI → TUI replay verifies shared persisted state and source bytes. It reproduced stale GUI close state
when relying on entity destruction; explicit window-close and app-quit checkpoints fixed that failure. The quit
callback waits for the background save. An older tree test now expects its saved folders to reopen expanded.

An owned PTY exercised the staged `lenz --tui` launcher: two tabs, a retained tab, active caret/scroll, hidden
explorer, normal exit and reopening. The active source returned in 5.05 ms; a 50-tab fixture returned in 4.78 ms.
The 50-tab fixture completed 120 actual folder reopens in 15.12 s. RSS grew from 9.73 to 10.35 MiB, peaked at
10.36 MiB, and ranged from 10.34 to 10.35 MiB over the last 20 cycles; cumulative process CPU was 0.59 s.
Source bytes and terminal modes were preserved. A separate real GUI on private Xvfb closed via WM_DELETE_WINDOW,
reopened the directory and retained identical file, caret and scroll metadata. Before/after captures are retained.

Production comparisons reused the previous Java-runtime repair's protocol and the immediately preceding
production binaries. Ten alternating TUI pairs and one GUI warmup followed by seven alternating GUI pairs opened
an identical disposable 100,000-line text fixture with isolated XDG state. Each GUI launch owned a fresh Xvfb and
used lavapipe with warmed shader caches. These file-only comparisons check normal reading overhead; the folder
restore timing above measures the new feature separately, since the baseline has no automatic restore.

| Measurement, median | Before | After |
| --- | ---: | ---: |
| TUI source feedback | 7.048 ms | 7.158 ms |
| TUI arrow feedback | 0.479 ms | 0.471 ms |
| TUI RSS after initial arrows | 13.32 MiB | 13.47 MiB |
| GUI first visible client paint | 171.62 ms | 171.51 ms |
| GUI dialog feedback | 24.33 ms | 23.77 ms |
| GUI RSS after initial dialog cycles | 174.82 MiB | 175.62 MiB |

Fifteen-second F2/Escape stress completed 182 cycles in each TUI, with post-stress RSS 13.48 → 13.59 MiB and
cumulative process CPU 0.16 → 0.15 s. GUI completed 196 → 192 cycles in 15.01 → 15.08 s, with CPU
12.21 → 11.82 s. GUI stress RSS was flat from the first stress sample (174.80 → 177.04 MiB between binaries).
Four additional GUI pairs measured paint 172.47 → 171.65 ms and feedback 26.48 → 26.00 ms. Their post-menu
anonymous memory was 52.76 → 53.96 MiB, file-backed RSS 97.31 → 97.69 MiB, and shared RSS 24.86 MiB in both.
The small GUI memory increase repeated; these measurements do not establish exact memory preservation.

Raw samples, source/executable identities, full gate/build logs, collector scripts and screenshots are retained
under `target/perf/workspace-restore/`. The initial PTY stress script opened a file prompt instead of reopening a
folder; its samples are kept separately and excluded above. The corrected collector selects Open Folder and
asserts that the prompt closes and restored content returns. The first GUI collector injected a semicolon for a
colon and tried Ctrl-Q (Quick Documentation); the corrected collector uses a valid line and the native window
close protocol. These harness failures did not change source files.

TUI timing ends at PTY receipt. GUI timing detects the first non-uniform client image or changed dialog pixels,
including capture overhead; neither is physical display latency or completed document loading. RSS excludes GPU
allocations and descendants. Short plateaus are not leak proof. Forced-termination recovery, long sessions,
native macOS/Windows and hardware-rendered display latency remain unverified. Search results and navigation
history are intentionally not restored; forced termination can lose state since the last checkpoint.

## 2026-10-18 Reader GUI app menu and Commands control

Reader now exposes its existing catalog through a top-left ☰ app menu, with File, View, Navigate, Find,
Selection and Language Servers groups, and a bottom-right Commands button following Diff's controls.
Groups, labels and command availability stay in the shared core; the terminal palette shows the same groups
and filters on their names. Menus show setting values and disabled reasons, support pointer/keyboard selection,
a back row and wheel scrolling, and consume input before documents. Below 640 px the footer button drops its
shortcut hint; below 240 px it yields to LSP status. F10 remains available.

Evidence is retained under `target/perf/reader-gui-menus/`: `before.json` pins the preceding workspace build's
source archive and copied binaries; `menu-change.patch` isolates this change from the preceding dirty tree;
`identity.json`, `source.patch` and `source.tar.gz` identify the final source and production executables.
`red.log` reproduces the missing visible menu before implementation. `ci-local.log` passes the full local gate,
including 253 Diff GUI checks, 105 Reader GUI checks and 35 terminal/parity checks; existing ignored tests remain
ignored. `final-clippy.log` additionally checks the final menu-wheel change with Reader's test-support feature.
`gui-build.log` builds the normal GUI without test/probe features. The existing launcher aliases resolve to the
rebuilt normal GUI and standalone terminal releases.

Actual GPUI input covers opening groups by pointer and keyboard, running existing actions, visible toggle and
disabled states, All menus / All commands, footer Commands versus left-hand LSP status, wheel scrolling and
End/Up in tiny windows, dismissal without selecting source, and unchanged file bytes. The combined adapter
replay now reaches split/move/unsplit/wrap/reload through the GUI app menu and terminal command palette and
compares the resulting semantic state and source bytes. Production screenshots in `shots/` cover Darcula,
GitHub Light and a 280×180 window. They use an owned private Xvfb display with lavapipe and disposable sources;
no automated input reaches the user's desktop or repositories.

The paired production probes reuse the preceding collectors and configuration: 100,000-line plaintext,
isolated XDG state, alternating old/new binaries on this Linux machine. GUI measurements use one warmup pair
and seven measured pairs, a warmed shared shader cache and a private 1920×1200 display. First-feedback GUI
images are the first non-uniform client capture, not completed file loading or physical display latency.
F10-to-changed-image measurements include screenshot overhead. Terminal timings end at PTY byte receipt.

| Median | Before | After |
|---|---:|---:|
| GUI first non-uniform image | 172.60 ms | 173.50 ms |
| GUI Commands open/close image response | 24.79 ms | 26.06 ms |
| GUI RSS after six dialog transitions | 177.66 MiB | 178.51 MiB |
| Terminal first source content, ten pairs | 7.11 ms | 6.93 ms |
| Terminal arrow feedback | 0.459 ms | 0.462 ms |
| Terminal RSS | 13.49 MiB | 13.52 MiB |

A second GUI run (one warmup plus four measured alternating pairs) measured first image 166.96→172.54 ms,
Commands response 24.21→23.03 ms and RSS 175.93→176.62 MiB. The response difference reversed; startup samples
overlap, so these trials do not establish a response-time regression. The small RSS increase repeats: the
second run's post-dialog median RssFile was 97.32→97.91 MiB, RssAnon 53.62→53.56 MiB and RssShmem
25.02→25.04 MiB. This is higher resident file-backed memory, not a growing anonymous allocation; exact RSS
preservation is not claimed. Attribution to a particular mapped file or binary layout is unverified.

Each paired GUI stress run lasted about 15 seconds, with 156→168 full Commands open/close cycles. Their
last/first sampled RSS was 178.17→178.15 MiB before and 178.60→179.90 MiB after; the latter stayed below
179.92 MiB. A separate 15.24-second app-menu/View/back stress ran 19 cycles with RSS flat at 181.59 MiB.
These short runs establish bounded behavior for the sampled interval, not leak proof or GPU-memory accounting.
`paired-gui.json`, `gui-memory-diagnostic.json`, `capture-menus.json`, `paired-tui.json` and terminal raw streams
retain all samples, hashes and actual durations. Native macOS/Windows, physical terminal/display latency and
long-duration menu stress remain unverified.

## 2026-10-18 Reader Search Everywhere

Reader follows the name-navigation flow in [IntelliJ's official guide](https://www.jetbrains.com/help/idea/searching-everywhere.html):
a shared All / Files / Symbols / Actions popup, fuzzy matching, secondary paths/details, direct scope shortcuts,
and double bare Shift for All. Ctrl-K is the portable All shortcut; Ctrl-P / Ctrl-Shift-N select Files,
Ctrl-Alt-Shift-N Symbols, and Ctrl-Shift-A / Ctrl-Shift-P Actions. Ctrl-Shift-N replaces the old New Window
shortcut; that action remains in the catalog. F10 retains the plain command menu. Tab cycles scopes while
retaining the query. Both adapters show action values, shortcuts and disabled reasons. Narrow layouts retain
query tails and scroll bounded visible rows. Only acceptance opens a document or runs an action.

Ranking runs on existing core workers over shared inventory/declaration snapshots and the command catalog.
It does not load candidate documents or start language servers. Symbols are syntactic candidates from the
existing bounded declaration index; its incomplete/error coverage is visible. The best 100 rows are retained.
New requests cancel obsolete work; request identities, displayed revisions and current command availability
are checked again before acceptance. Declaration invalidation and final inventory rescans refresh open symbol
searches, including deletion of the sole indexed source. A focused review found this missing deletion case;
`deletion-red.log` reproduces it and `deletion-green.log` verifies the shared fix.

Evidence lives in `target/perf/search-everywhere/`. `before.json`, `before.patch`, copied baseline binaries and
the preceding `reader-gui-menus/source.tar.gz` preserve the comparison baseline. Final identities and source
are retained in `identity.json`, `source.patch` and `source.tar.gz`. Collector scripts, complete raw samples,
build/gate logs, terminal streams and private-display screenshots accompany them. No production dependency,
thread or language-server request was added. Actual GUI/TUI input replay compares files, symbols, action
settings, category transitions, cancellation and source bytes; focused checks cover stale completions/clicks,
disabled actions, empty roots, long Unicode input, repeated keys, narrow layouts and focus interruptions.

Production checks use disposable fixtures and isolated XDG state. GUI captures use owned Xvfb displays with
lavapipe in Darcula and GitHub Light, including a 280×180 window. Physical XTest Shift press/release events open
All, while shifted typing interrupts the sequence. The first OCR check falsely reported failure despite the
visible popup; cropping/upscaling its header fixed recognition without changing application behavior. Temporary
instrumentation was removed from source and production releases never contained it. The TUI probe sends kitty
modifier reports through an owned PTY, opens files at a line, navigates symbols without LSPs, executes an action,
resizes to 20×6 and 1×1, preserves source bytes and verifies restored terminal modes.

The production comparisons open the same disposable 100,000-line plaintext fixture with isolated state,
alternating baseline/current binaries on this Linux machine. GUI uses one warmup pair and seven measured pairs,
a shared warmed shader cache and a private 1920×1200 display. Its first-feedback endpoint is the first
non-uniform client image; F10 response is input-to-changed client image including capture overhead. Terminal
measurements end at PTY output receipt. Neither endpoint measures physical display latency or completed loading.

| Median | Before | After |
|---|---:|---:|
| GUI first non-uniform image | 180.75 ms | 175.32 ms |
| GUI F10 open/close image response | 25.38 ms | 24.93 ms |
| GUI RSS after six dialog transitions | 175.83 MiB | 175.95 MiB |

Fifteen-second GUI F10 stress completed 193→194 full open/close cycles in 15.03→15.08 seconds, with cumulative
CPU 13.96→14.07 seconds. RSS was flat over each sampled interval (177.89 MiB before, 175.74 MiB after). RSS
varies across launches, so these samples do not establish exact memory preservation; they show no growing
allocation during this stress interval.

Ten alternating terminal pairs measured first source content 7.51→7.18 ms, arrow feedback 0.487→0.479 ms and
RSS 13.42→13.52 MiB. Each F2/Escape stress completed 182 cycles in 15.05 seconds, with cumulative CPU 0.17 s
in both. The final RSS samples were 13.56 MiB before and 13.33 MiB after; neither grew continually. These
startup/input samples and the GUI samples show no reproducible slowdown in the tested normal-reading workload.

Separate new-popup workloads use 1,200 small Rust sources with LSP discovery disabled. GUI ran 48 cycles of
All/open, three different queries and dismissal in 15.03 seconds; first changed-query captures had median
15.97 ms including screenshot overhead. RSS was 184.79→185.54 MiB; the last twenty samples ranged
185.15–185.54 MiB, and process CPU advanced 23.68 s across renderer threads. TUI completed 73 cycles in
15.11 seconds; median query-feedback receipt was 41.25 ms including the collector's deliberate quiet wait.
RSS was 13.11→13.25 MiB and CPU advanced 0.36 s. These endpoints measure initial feedback, not completed
ranking. Both collectors preserve all samples and source-byte assertions. The short memory ranges are not
leak proof; RSS excludes GPU allocations and child processes.

The release-only ranking probe uses 100,000 file names and 10,000 declarations, one warmup and eleven retained
samples with deterministic counts/caps. Median existing file ranking is 4.02 ms; unified Files ranking plus
result-row construction is 4.38 ms; All with declarations is 4.57 ms. These are different output workloads,
not an adapter-latency comparison: the new path also builds labels, paths, icons' metadata and validated targets.
Mid-ranking cancellation returns in 0.67 ms. `core-perf-final.log` and `core-perf.json` retain raw samples;
the production comparisons above measure the normal-reader overhead separately.

Bare terminal Shift needs modifier press/release reports; legacy terminal/multiplexer/SSH paths may require
Ctrl-K instead. Native macOS/Windows, hardware display/terminal latency, long sessions and exhaustive language
coverage remain unverified. Symbol results use syntax declarations, not compiler-complete semantic resolution;
existing LSP symbol actions remain separate.

## 2026-10-18 LSP version diagnostics and runtime repair

The reported Pyright failure was reproduced with the running Reader's environment: `pyright-langserver`
resolved to a mise shim, the selected PATH used Node 26.10.0, and Pyright existed only under Node 26.8.1.
Mise's actual cause was “No version is set for shim”; Reader had retained only its generic final verbose hint.
This demonstrates runtime selection failure, not a proven incompatible Pyright/Node version pair.

Reader now retains the first and last halves of an 8 KiB stderr budget, including unterminated lines, with a
visible omission count. Conversion keeps valid display text and exposes hidden controls. EOF waiting is bounded;
no stderr thread is joined indefinitely. Failure reports name the resolved launch executable, the selected
Node/Java path and probed version when available, and readable Pyright `engines.node`. Runtime/version probes
honor provider environment and workspace, have bounded output and a three-second timeout, and run off the UI
thread. Existing provider version flags use the same bounded helper. F2/copy details show the detected server
version independently of Lenz's tested version; an untested version is not labelled incompatible.

Both adapters reuse the runtime dialog for Java and Pyright. Choosing existing Node records only
`providers.pyright.node`; launch prepends its bin directory and prefers its adjacent server when using the
default executable. Custom executable/arguments/environment and other providers remain intact; reset removes
only this override. Java still changes only its launcher argument. Invalid/malformed/concurrently changed
settings are preserved, save/reset retains permissions and disabled providers remain disabled. Discovery
scans existing installed locations on request; it never installs tools, changes global mise settings or adds
an idle service.

Evidence under `target/perf/lsp-runtime-recovery/` includes baseline source/production hashes and copied binaries,
red/green tests, full local gate, final core/clippy/build checks, private production captures, raw PTY streams,
configuration and per-sample timing/memory results. The shared input replay now repairs both Java and Python
through actual GUI/TUI menus and pasted paths, compares resulting settings and state, and preserves source
bytes. Native captures cover Darcula/GitHub Light, 280×180 and 1×1 resizing. A real production TUI copied the
mise cause, Node 26.10.0 and version context, selected the existing Node 26.8.1, saved/restarted and reported
Pyright 1.1.414 ready. The same app flow applied the override to the user's provider settings while preserving
Java settings; it did not drive the user's desktop. An initialize/shutdown handshake against the reported
`dbquacks` root advertised definition support and left Git status unchanged; full project analysis was not run.

The initial PTY collector supplied TERM twice; it failed before launching and was corrected. A first chooser-stress probe sent F2 immediately after Escape; waiting for dismissal and the next status frame corrected its input sequencing. Concurrent unit
probes exposed a fixture rewrite/execute race (`ETXTBSY`); test scripts now publish a fresh inode atomically.
No production retry was added. The final core suite passes 395 tests with 46 ignored. Source identities and
executable hashes are retained with the measurement artifacts; existing ignored external-tool tests remain
unverified rather than counted as passes.

Production comparisons use the preceding Search Everywhere release binaries, identical disposable 100,000-line
plaintext, isolated state and alternating order on this Linux machine. GUI runs use private 1920×1200 Xvfb,
lavapipe, warmed shared shaders, one warmup pair and seven measured pairs. GUI startup ends at the collector's
first non-uniform client image; F10 feedback ends at changed client pixels including screenshot overhead.
Terminal timings end at PTY receipt. These are first feedback, not completed loading or physical display latency.

| Median | Before | After |
|---|---:|---:|
| GUI first non-uniform image, seven pairs | 176.05 ms | 181.93 ms |
| GUI F10 feedback | 23.42 ms | 24.98 ms |
| GUI RSS after six menu transitions | 177.61 MiB | 178.50 MiB |
| Terminal first source, ten pairs | 7.87 ms | 7.48 ms |
| Terminal arrow feedback | 0.496 ms | 0.468 ms |
| Terminal RSS | 13.49 MiB | 13.60 MiB |

Four further GUI pairs measured first image 172.86→177.93 ms and F10 feedback 26.30→23.94 ms. Twenty additional
startup-only pairs measured 174.64→173.47 ms; the initial approximately five-millisecond startup difference did
not reproduce. Those paired startup differences have median +0.79 ms with mixed signs. All raw samples,
including initial high outliers, remain in the separate JSON records. Response differences reverse across runs.
The twenty-pair median RssAnon was 54.07→53.99 MiB, RssFile 97.10→97.67 MiB and RssShmem 24.59 MiB in both.
The repeated small increase is resident file-backed memory; attribution to a particular mapping and exact RSS
preservation remain unverified, while growing anonymous allocation was not observed.

Fifteen-second GUI F10 stress completed 194→199 cycles in 15.08→15.06 seconds, with cumulative process CPU
14.16→14.29 seconds. After RSS was flat at 178.33 MiB; before ended at 178.89 MiB below a 179.07 MiB peak.
Terminal F2/Escape stress completed 182 cycles each in 15.04 seconds, with CPU 0.16→0.17 seconds and end RSS
13.41→13.75 MiB. New GUI runtime-chooser stress completed fifteen open/choose/dismiss cycles in 15.79 seconds,
with RSS 186.83→186.89 MiB and process CPU advancing 2.08 seconds. The existing Pyright session remained running. The terminal chooser completed 120 cycles in 15.10 seconds, with RSS 13.00→13.62 MiB (flat over the last twenty samples) and CPU advancing 0.35 seconds. Source bytes, settings and terminal modes were preserved.

RSS excludes GPU allocations and language-server descendants. Short memory plateaus are not leak proof.
Runtime probes may add up to three seconds to a failure diagnosis; they do not block frontend input. Only
Java and Pyright have in-app runtime pickers. Generic provider setup instructions and detected/tested versions
remain available for other failures. Full `dbquacks` analysis, exhaustive tool/version combinations, native
macOS/Windows and hardware display latency remain unverified.


## 2026-10-18 Reader popup scrolling, caret arrows and Recent Files

Popup wheel input now changes adapter-owned viewport offsets without changing the selected option or the
underlying document. Keyboard navigation selects and reveals rows; Enter accepts the selection even after it
scrolls out of view. Dismissal/reopening resets the viewport and narrow layouts clamp it. Source arrows now
move a collapsed caret through a shared command, revealing its vertical and horizontal position and preserving
UTF-8 boundaries, CRLF, folds and display columns. Shift selection and tree/result/preview controls retain their
existing behavior. Ctrl+E reuses the file-search presentation with file icons, secondary paths and fuzzy matching,
while retaining MRU order for an empty query and Delete-to-forget. The bounded recent list ranks synchronously
so Ctrl+E followed immediately by Enter remains usable; project name searches remain asynchronous.

Evidence in `target/perf/reader-interactions/` retains the preceding production binaries and source identity,
red/green core and adapter regressions, full `scripts/ci-local.sh` gate, final focused GUI checks and production
release builds, combined real-input GUI/TUI state/file replay, private production captures and raw PTY streams.
The gate passes workspace/core tests, actual Diff and Reader GUI adapter tests, Reader TUI and parity checks,
collector tests, panic-site ratchets and standalone release builds. Removing unwraps lowered the Reader TUI
ratchet from 13 to 12. Existing ignored external-tool and platform tests remain unverified.

Review exposed unwrapped movement using source byte length for rows whose tabs or inlay labels expand the
shown text. A tab-indented regression reproduced byte 8 instead of byte 10 before the shared helper was fixed.
Review also caught a Places popup reusing a dismissed viewport; reopening now reveals its initial selection.
Initial native collector assertions misread whole-window OCR and background tab text, and expected a caret
footer while the clipboard notice was visible. Visual inspection and native Enter/clipboard assertions replaced brittle OCR predicates; popup-only text
assertions and explicit notice dismissal corrected the terminal checks. Failed evidence remains separate. A duplicate Delete hint was removed after visual review.

Production GUI checks use an owned Xvfb/lavapipe display and isolated settings. Dark/light captures cover the
full, fuzzy, scrolled and narrow Recent popup, all four caret arrows, and 1×1 resize/dismissal. Actual Enter and
clipboard reads verify the selected target and exact Unicode source selection. All twenty fixture hashes are
unchanged. Production PTY checks cover both themes, fourteen recent files plus a Unicode/tab/CRLF document,
40×10, 12×4 and 1×1 resizes, exact OSC 52 copy, saved collapsed caret, unchanged source hashes and terminal-mode
restoration. The retained final terminal evidence is `native-tui-final2/`, with persisted-state assertions in
`persisted-state-check.json`.

New-popup stress completed 34 GUI Ctrl+E/wheel/dismiss cycles in 15.35 seconds, with process RSS
182.73–182.88 MiB and cumulative CPU advancing 2.32 seconds between samples. Terminal stress completed
73 cycles in 15.02 seconds, with RSS 9.742–9.797 MiB. These correctness/resource checks overlapped some build/test
work; their durations are actual observed intervals, not controlled latency comparisons. Process RSS excludes
GPU allocations and language-server descendants; a short bounded range is not proof of leak absence.

Alternating before/after comparisons use the preceding LSP-repair production binaries, identical disposable
100,000-line plaintext, isolated state, the same machine and no concurrent builds or native stress. GUI runs use
an owned 1920×1200 Xvfb/lavapipe display, warmed shared shaders, one warmup pair and seven measured pairs.
First image means collector receipt of a non-uniform client image; F10 response means changed client pixels,
including capture overhead. Terminal runs use ten alternating pairs and measure PTY output receipt. These
boundaries measure first feedback, not completed loading or physical display latency.

| Median | Before | After |
|---|---:|---:|
| GUI first non-uniform image | 178.42 ms | 175.96 ms |
| GUI F10 feedback | 25.00 ms | 25.34 ms |
| GUI RSS after six menu transitions | 175.82 MiB | 175.91 MiB |
| Terminal first source | 7.64 ms | 7.76 ms |
| Terminal arrow feedback | 0.502 ms | 0.415 ms |
| Terminal RSS | 13.63 MiB | 13.68 MiB |

Arrow timings use identical Down inputs but intentionally different outcomes: the previous release scrolls,
while this release moves the caret. They establish response time for those respective behaviors, not equivalent
scrolling cost. Startup, F10 and RSS samples remain close to the preceding release; exact timing/allocation
preservation is not established. Raw per-sample data and binary hashes are retained in `paired-gui.json` and
`paired-tui.json`.

Fifteen-second paired GUI F10 stress completed 185→192 cycles in 15.03→15.01 seconds, with cumulative CPU
advancing 12.83→13.25 seconds between samples. Sampled RSS was flat at 178.93→176.43 MiB. Terminal F2/Escape
stress completed 182 cycles each over 15.05 seconds, with cumulative CPU 0.16→0.17 seconds, and after RSS
13.82–13.84 MiB. All fixture bytes and terminal modes were preserved. Native macOS/Windows, hardware latency,
GPU-memory attribution and long-session leak behavior remain unverified.

A subsequent push coincided with a host Rust/LLVM upgrade: the missing LLVM 22 library interrupted the final
release build, then Rust 1.99.1 restored compiler availability. That retry exposed an intermittent runtime-test
fixture `ETXTBSY`: atomic renaming alone cannot prevent another forked child from retaining a writable script
descriptor. A controlled fork/descriptor probe reproduces the failure and verifies an isolated writer process;
the Node and Java test helpers now publish scripts from that process. The core suite passes 318 tests with 16 ignored.
The probe and failed push logs remain in `target/perf/reader-interactions/`; this change affects test setup only.
