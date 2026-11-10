# Publish Lenz 0.0.14 beta
Status: resolved

Publish the merged main tree after PR52 as the next beta, following the existing
three-platform tag packaging and verified-draft publication procedure. Validate
the version commit with normal Cargo profiles, local CI, performance gates and
the full suite. Verify platform archives, checksum sidecars, packaged CLI routing
and macOS bundle signatures, then publish exact assets and update Homebrew/AUR
metadata and the existing Homebrew tap. AUR registration remains unavailable.
Keep raw evidence under perf-results/release-0.0.14/. Report physical/native
manual coverage as unverified where unavailable; never mark it passed.

## Verified outcome

Published [0.0.14 beta](https://github.com/aantoniadis/lenz/releases/tag/v0.0.14)
from validated source `cc4411322c58756d5486e127ba9ef45af1772c16`. Fresh local CI,
normal-release gates, all 133 full-suite cases/394 metrics and sustained workloads
pass. All three native packages pass command checks; macOS strict signing and
packaged Linux GUI/TUI replays pass. All six public downloads match their final
build artifacts and checksum sidecars. Repository Homebrew/Arch metadata and the
existing Homebrew tap are updated. AUR submission and native physical interaction
remain unverified as recorded in the
[validation report](../../docs/research/release-0.0.14-validation.md).
