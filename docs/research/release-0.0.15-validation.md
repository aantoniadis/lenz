# Lenz 0.0.15 beta validation — 2026-11-10

Release source: `2e998fa2d0e24bc69e1a84177f1f23580340f9b8`, annotated tag
`v0.0.15`, after squash-merging PR #53 as `9f6955a`.
[Lenz 0.0.15 beta](https://github.com/aantoniadis/lenz/releases/tag/v0.0.15)
was published at 13:06:45 UTC on 2026-11-10.
Raw evidence is retained in `perf-results/release-0.0.15/`; the feature audit's
samples, identities and native screenshots remain in `perf-results/app-audit/`.

## Local gates

The exact release commit passes `scripts/ci-local.sh`: 2,980 workspace/serial
and 941 dedicated adapter results across overlapping invocations, zero failures;
86 Python checks with one platform skip. Strict Clippy, panic budgets, separate
standalone TUI release builds and the production GUI size check pass. Stripped
GUI size is 68,917,456 bytes. The supervisor returns 0 in 782.395 seconds,
collecting 819 orphans with no forced cleanup.

The normal-release `scripts/perf-check.sh` also passes, exit 0 in 143.290 seconds
including compilation. All builds use repository Cargo profiles and no compiler
or profile overrides. Shipping packages use the repository's `dist` profile and
independent frontend builds.

The first gate stops during compilation because disk space is exhausted. Idle
build caches are cleared while retaining evidence. A second attempt identifies
a scratch-picker test waiting for the selected tab path without waiting for its
document to load. The fixture now awaits the current source before typing;
all input/bytes assertions and the existing timeout remain. Its focused test
passes, followed by the complete final gate above. Neither attempt is omitted.

## Feature performance and coverage

The [Code audit](code-branch-picker-audit.md) records normal-release paired
editing and branch stress, native Linux light/dark/narrow GUI/TUI checks, four
X11 close-consent scenarios, exact-byte Save/Undo/Redo and the rejected layout
experiment. Release changes after that audit are version metadata and the test
readiness precondition, with no additional production algorithm change. The
full 133-case suite from 0.0.14 was not rerun; the fresh release component gate
and feature-specific campaigns are separate evidence.

Cached branch choices measure 20.635 → 1.407 ms on 1,000 generated refs, three
worktrees and 2,000 untracked files. The approximately 0.6 ms larger first-frame
cost and roughly 0.04 ms upward TUI key-feedback tendency were explicitly
accepted by the user. No universal editor speedup or exact preservation claim
is made. RSS modes, weaker diagnostic oracles and feedback versus presentation
boundaries remain explicit in the audit.

## Native packages and publication

Native builds: [42818185199](https://github.com/aantoniadis/lenz/actions/runs/42818185199).
Verification/publisher: [42821400246](https://github.com/aantoniadis/lenz/actions/runs/42821400246).
The publisher requires the exact successful local validation ref and release
source, all three successful native jobs, six checksum-matching release assets
and matching workflow artifacts. It then checks packaged source identities,
launcher routing and Linux GUI/TUI launches/editing before publishing a beta.

All three native packages pass their unpacked CLI/launcher checks; macOS also
passes strict bundle-signature verification. Publication succeeds after verifying
all six downloads against release digests, checksum sidecars and build artifacts,
checking embedded source IDs, and running packaged Linux GUI/TUI smoke checks.

| Archive target | SHA-256 |
| --- | --- |
| Apple Silicon | `8c6a385b598c233c4ff42f3a2cffd1cc2de381258a28b60383a094ab26358dca` |
| Linux ARM64 | `3071124fb2a02e1b6961067bf1794aeffb78f037378d97f7b474a6618dfe3ee0` |
| Linux x86_64 | `56687cd9b926d4d9c725fd1b81c99e285c836b8fc8b6b21e637b067e1ce01ee1` |

The first publisher attempt stops before publication because lookup by tag does
not return the draft. The corrected publisher uses authenticated release listing
to identify the draft, preserving all package/source/digest checks. The failed
attempt's log is retained; successful verification reuses the exact build assets.
Homebrew and repository Arch metadata use the verified sidecars. Ruby and shell
syntax checks pass. The Homebrew tap is updated in `b0c78bf` and its served formula matches exactly.
All six public downloads (152,692,231 bytes) are independently downloaded locally
and match recorded sizes and SHA-256 digests. A fresh Homebrew
installation on every supported platform is not claimed.

## Limits

Actual AppKit/macOS Command-Q and red-close delivery, physical displays,
trackpads, Wayland interaction, fresh Homebrew installs on each platform,
prolonged live-provider sessions and indefinite leak freedom remain unverified.
Native CLI/signature checks do not establish interactive macOS acceptance.
macOS packages are ad-hoc signed. AUR repository metadata is updated separately;
an AUR submission is not claimed.
