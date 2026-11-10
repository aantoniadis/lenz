# Reject disk observations based on a superseded Save baseline
Status: claimed
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

Four focused regressions and core strict Clippy pass. Complete combined-tree CI,
native replays and equivalent normal-release performance remain pending.
