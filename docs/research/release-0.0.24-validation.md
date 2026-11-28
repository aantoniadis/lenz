# Lenz 0.0.24 beta validation — 2026-11-28

Release source: `95599e6308380457ba1cde884b88f1058a4180c8`, annotated tag
`v0.0.24`, after PR #75 (Lenz Code project tabs and Side by Side, Format Selection with
an XML formatter, the welcome screen and Open Recent Project, line-wise Cut/Copy/Paste,
YAML through ERB, macOS Cmd/Opt shortcut labels).
[Lenz 0.0.24 beta](https://github.com/aantoniadis/lenz/releases/tag/v0.0.24)
was published at 08:46:31 UTC on 2026-11-28.
Raw logs and samples are retained in `perf-results/release-0.0.24/`.

## Local gates

The pre-push hook ran `scripts/ci-local.sh` on pushing `main`: 57 Cargo test-result groups
with zero failures, 87 Python checks, strict Clippy, panic budgets, standalone TUI release
builds and the production size check (stripped GUI 80,061,616 bytes, under the
85,000,000-byte budget); the supervisor returns 0 in 314 seconds. `scripts/perf-check.sh`
passes, exit 0. All builds use repository Cargo profiles and no compiler or profile
overrides.

## Lenz Code against 0.0.23

PR #75 changes the single-project path of both Lenz Code adapters (the desktop window
root, a projects check per terminal draw), and its own measurement ticket was not run.
Release `perf-probe` builds of `v0.0.23` and of the release source were compared on one
Linux machine, alternating builds, with the same copied corpus.

| Workload | 0.0.23 | 0.0.24 |
| --- | --- | --- |
| Desktop warm launch to readable, median of 100 (two passes) | 135.9, 136.1 ms | 136.1, 136.1 ms |
| Desktop fresh launch to readable, median of 100 | 136.0, 136.2 ms | 136.1, 136.1 ms |
| Desktop ten tabs, settled RSS | 199.8, 199.0 MiB | 204.8, 204.8 MiB |
| Desktop ten tabs, peak RSS | 221.6, 221.5 MiB | 226.5, 226.0 MiB |
| Terminal first content (four passes) | 10.98, 11.83, 10.97, 10.94 ms | 10.16, 10.33, 10.47, 10.52 ms |
| Terminal input first byte, median of 30 | 0.77, 0.81, 1.20, 0.78 ms | 0.77, 1.29, 0.79, 1.06 ms |
| Terminal 10 s stress input first byte, median | 0.93, 0.99, 0.86, 0.86 ms | 0.91, 1.37, 0.84, 0.85 ms |

The desktop window with ten tabs settles about 5 MiB higher, reproducibly. Launch time is
unchanged. Terminal figures overlap between builds; the slower input passes occur in both.
Terminal settled RSS (43–53 MiB) varies by run in both builds.

`scripts/code-tui-bench.py` waited for the footer's `View 2/2`, which the project strip
replaced; it now waits for the empty view's start screen (commit `1d57b86`; its Python
tests pass). Both builds were measured with the updated collector.

Not measured: Side by Side with two and four projects (frame interval, RSS per project, the
terminal's off-screen copy at large sizes), repeated opening and closing of project tabs,
and desktop interaction latency.

## Native packages and publication

Native builds: [46125458409](https://github.com/aantoniadis/lenz/actions/runs/46125458409).
Verification/publisher: [46125471112](https://github.com/aantoniadis/lenz/actions/runs/46125471112),
the 0.0.23 publisher with the version and source commit changed. The publisher waits for
the `automation/validated-v0.0.24` ref at the release source, requires all three native
jobs and their package smoke steps, verifies all six downloads against release digests,
checksum sidecars and build artifacts, checks embedded source IDs, and runs packaged Linux
GUI/TUI launch and editing checks before publishing the beta.

| Archive target | SHA-256 |
| --- | --- |
| Apple Silicon | `37a08fda188bf71abc3b6e4dd06b3f93bdcaf817a18c09fe941ff7419f28b1a7` |
| Linux ARM64 | `ccd5472b26ea70d64b4e2445343393cf7603b2ebe03a477871882623bb0c870a` |
| Linux x86_64 | `40d9e25122ef2d999f33530e269b56de32ca10e03abd16d88d8773a3f3e483d1` |

Homebrew and repository Arch metadata use the published sidecars, which match the
downloaded archives. A fresh Homebrew installation on every supported platform is not
claimed.

## Limits

Native AppKit/macOS interaction, including the new Cmd and Opt shortcut labels,
physical-display timing, trackpads, fresh Homebrew installs on each platform and
prolonged live-provider sessions remain unverified. Native CLI/signature checks do not
establish interactive macOS acceptance. macOS packages are ad-hoc signed. AUR repository
metadata is updated separately; an AUR submission is not claimed.
