# 26: Offer IntelliJ and VS Code keymaps with overrides

**What to build:** Select familiar presets and override shortcuts while keeping actions accessible in both frontends.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Default to IntelliJ and persist VS Code or individual overrides with explicit conflict handling.
- [x] Resolve Reload, Type Hierarchy and language-status conflicts; labels and dispatch use centrally defined semantic actions.
- [x] Test modal/editor/menu focus, terminal printable keys and unsupported key delivery with equivalent menu access.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 55, 56, 57, 58, 59, 66.

## Answer

Merged in 6ac75cf (branch work/editor-26-keymaps, 68e97d4; ratchet fix 5a47cf4). `crates/code-core/src/keymap.rs` holds the IntelliJ (default) and VS Code preset tables, per-command overrides from settings (`keymap`, `keybindings` slug→chord/`none`), and one resolution feeding both menu labels and key dispatch through `Session::key_command`/`key_hint` — `CommandId::shortcut()` was replaced by that seam. `Keymap::bind` refuses a chord another command owns; preset switches report displaced preset bindings. Conflicts resolved: Reload = Ctrl+R/F5, Type Hierarchy = Ctrl+H + Alt+H, F2/Shift+F2 separated, duplicate Alt+B/Ctrl+Q bindings removed. GUI 182 + 3 keymap tests, TUI 128 + 5 input/1 render, parity replay passing, clippy clean. Limits: commands without a current chord can only be bound from the settings file, editing keys stay non-rebindable, terminal delivery is a static guess — see docs/editor-follow-ups.md. Performance deferred; native macOS and Wayland overrides unverified.
