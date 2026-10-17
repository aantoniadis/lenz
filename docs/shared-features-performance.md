# Shared Reader and Diff features: verification

Linux release comparison on 2026-10-17, against `9dedeb8` in the launcher experiment.
Reader and Diff retain independent sessions. Shared font preferences are read at
startup; theme selections made in one process are used on another process's next
launch. Existing open processes do not synchronize their selections or documents.

## Correctness

`scripts/ci-local.sh` passed: workspace clippy with warnings denied, both panic-site
budgets, workspace tests, isolated memory tests, 253 Diff GUI adapter tests, Reader
GUI tests, 50 Python collector tests (one skipped), and the release size gate
(stripped Diff GUI: 67,948,216 bytes). The targeted Reader context-menu regression
also passed after testing repeated small pixel-wheel events.

Shared tests cover query validation, exact versus truncated 10,000-match results,
cancellation, anchor/next/previous navigation, counters and option metadata. Real
GUI and TUI adapter tests cover input, option clicks, modal routing, presentation
and narrow geometry. Reader retains its cancellable search workers; Diff retains
its existing scheduling. Sharing matching policy does not make those schedulers
identical.

Private Xvfb checks exercised both production GUI find controls and preserved
fixture bytes. Reader ignored the shared font size before the change (13 and 26
both produced a 10px sample glyph). Afterward, the same sample measured 10px at
13, 20px at 26, and 23px at 32. At 32, a real 800×600 Reader window displayed the
last context commands after End; the menu now clips and scrolls, preserving
keyboard and trackpad access. Screenshots were inspected as well as checked with
OCR. These checks used DejaVu Sans Mono and both light and dark appearances.

## Performance

All probes used disposable plain-text fixtures (3,000 lines), isolated XDG
configuration, Mesa lavapipe on owned 1600×1000 Xvfb displays, and an owned 120×36
PTY. GUI timing ends when the parent observes the X11 window title, **not first
paint or completed document readiness**. TUI timing ends at first comparison
content bytes. Input timing ends at terminal feedback, not physical display.

Original same-worktree baseline binaries were preserved before edits. A second
baseline was rebuilt from an archive of the same revision with the same package
build commands. Measurements use immutable binary copies and recorded SHA-256
identities. Eight initial alternating pairs and two sets of 16 alternating pairs
(two warmup pairs per set) found no consistent timing regression: every per-run
paired-median bootstrap interval included zero. The final 16-pair comparison with
the original preserved binaries measured:

| Metric | Before median | After median |
| --- | ---: | ---: |
| Reader window observed | 35.8 ms | 35.1 ms |
| Diff GUI window observed | 36.3 ms | 37.3 ms |
| TUI first content | 6.0 ms | 6.4 ms |
| TUI find feedback | 0.363 ms | 0.363 ms |

Paired median anonymous-memory changes in that comparison were −0.041 MiB for
Reader, +0.004 MiB for Diff GUI and +0.004 MiB for TUI. Total RSS was sensitive to
file-backed page residency: the archived-baseline repeat suggested +0.87 MiB for
Diff and +0.16 MiB for TUI, while the original-baseline repeat measured −0.15 MiB
and −1.54 MiB. These are not established memory optimizations or heap regressions.

Matched 30-second repeated find/open/close runs completed 978–981 cycles per
application/version without changing fixture bytes. Reader's RSS rose
168.1→169.5 MiB before and 167.8→169.1 MiB after; Diff rose 205.2→206.5 MiB before
and 206.3→207.5 MiB after. These short runs do not prove absence of leaks.

## Evidence and reproduction

Raw source snapshots/hashes, executable hashes, configuration/method identity,
per-process timing and memory samples, screenshots, red/green regression logs and
probe scripts remain in `target/shared-features-evidence/` (local ignored output).
The successful gate is `ci-verified.log`; primary sample sets are
`performance-samples.json`, `memory-repeat-samples.json` and
`original-baseline-samples.json`; uncertainty is in
`paired-timing-uncertainty.json`. `sources.json` identifies the inspected files.

The first attempted archived-baseline comparison reused a Cargo target directory
and left stale baseline executables in the after slot. The production font check
caught this. Those samples were rejected and moved under `invalid-cache-run/`;
workspace release artifacts were rebuilt before the recorded runs. Baseline and
after executable copies must remain distinct when replaying the local probes.

Cold OS caches, hardware GPU/display latency, native macOS, long-duration stress
and real language-server integration remain unverified by this change. The
existing performance table uses different endpoints and fixtures and is unchanged.
