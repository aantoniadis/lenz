# Reject disk observations based on a superseded Save baseline
Status: resolved
Type: task

An actual disk check begun against the pre-Save baseline can arrive after Save
confirmation and incorrectly mark that own write external, or restore an old
stamp. A deterministic real-writer regression fails against the original path.
This is independently confirmed; the earlier intermittent Vim parity failure's
direct cause remains unconfirmed.

Refresh replies now carry opaque confirmed-baseline identity. A current-request
reply whose baseline changed triggers a fresh check, preserving detection of an
actual external write after Save. Move replies remain separate. No unconditional
additional disk read is added to ordinary Save.

Four focused regressions, core strict Clippy, complete combined-tree CI and native
replays pass. Performance acceptance is tracked separately in ticket 04.

## Answer

All four deterministic regressions, strict core Clippy and final combined CI11
pass. GUI/TUI native and release editing oracles pass; final runtime files and
Code executable hashes match the validated d7 source exactly after reverting
the separate layout experiment. The independent original Vim failure's cause
remains unconfirmed. See the [audit report](../../../docs/research/code-branch-picker-audit.md).
