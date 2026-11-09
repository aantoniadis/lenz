# GPUI Linux transport patch

This directory contains `crates/gpui_linux` from
[Zed v1.26.2](https://github.com/zed-industries/zed/tree/c4273271092b0287f7d181f616bd5171e0a2f70c/crates/gpui_linux),
commit `c4273271092b0287f7d181f616bd5171e0a2f70c`, under its Apache-2.0 license.
`LICENSE-APACHE` is retained here. Other GPUI crates remain pinned to that same tag.

The source change is confined to the X11 compose policy and its client dispatch: when Escape cancels an
XKB compose sequence, discard preedit, reset the compose state, notify the input
handler of cancellation, and consume that Escape. Previously the printable-key
fallback committed a spacing accent before the application received Escape.
Unmatched printable keys keep their existing fallback. A test exercises a real
XKB compose table, Escape cancellation, the next composition, and printable
fallback; production private-X11 replays cover the actual window/input path.

`Cargo.toml` resolves the upstream workspace-inherited values explicitly so this
one crate can be patched without vendoring Zed's workspace. Internal dependencies
point to the original tag; dependency versions, features and defaults are retained.
The crate is excluded from Lenz's workspace members. On Linux/FreeBSD, `lenz-gpui`
tests include the exact shipping `x11/compose.rs` source with a test-only XKB
dependency, so its regression runs in the ordinary workspace test gate.

When updating GPUI, compare this crate against the new tag and remove the local
patch once upstream cancellation has equivalent behavior. Do not modify a Cargo
cache checkout: release builds use this tracked copy through the workspace patch.
