# 58: Deliver first-class ERB editing

**What to build:** Edit ERB templates with Ruby/Rails and markup assistance at correct source locations.

**Blocked by:** 57: Route embedded-language edits through source maps; 50: Deliver Rails-aware application assistance; 51: Deliver HTML and embedded-language assistance.

**Status:** ready-for-agent

- [ ] Recognize ERB templates including compound HTML extensions and route interpolation/multiline Ruby/attributes correctly.
- [ ] Verify actual helper/member completion, signatures, diagnostics, markup edits and configured formatting with Unicode/CRLF fixtures.
- [ ] Both adapters offer useful current-source assistance; the unsafe native ERB range behavior cannot be accepted as a passing route.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 79, 80.

## Comments

Ticket04 provides a verified bounded route, not full template support. Carry its research-note limits into implementation: Haml dynamic attributes/control blocks/filters/comments/escaped lines and nested/incomplete interpolation remain unmapped; ERB escaped closing delimiters/trim modes, mixed markup/Ruby routing, full snippet navigation and Rails context remain unverified. Signature prototype covers parenthesized calls and tested argument shapes only; other call forms, splats and keyword semantics need correct behavior. Preserve or extend verified source/URI/version safety when porting to production and prove actual GUI/TUI input/undo/save/cancellation; do not resolve these product tickets by re-running only the prototype fixtures.
