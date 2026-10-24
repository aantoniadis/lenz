# 57: Route embedded-language edits through source maps

**What to build:** Apply Ruby and markup assistance to original template text through one validated embedded-language route.

**Blocked by:** 04: Resolve template projection and encoding blockers; 39: Apply completion imports and snippet edits atomically; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings; 40: Display completion documentation and signature help.

**Status:** ready-for-agent

- [ ] Use the proven automatic region mapping, negotiated positions and original-source identities for completion/diagnostic/format requests.
- [ ] Reject scaffolding, foreign-file, stale or incompatible multi-cursor edits as whole transactions.
- [ ] Demonstrate a real template fixture through both adapters’ existing language-assistance controls, retaining surrounding bytes and one-action undo.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 79, 80.

## Comments

Ticket04 provides a verified bounded route, not full template support. Carry its research-note limits into implementation: Haml dynamic attributes/control blocks/filters/comments/escaped lines and nested/incomplete interpolation remain unmapped; ERB escaped closing delimiters/trim modes, mixed markup/Ruby routing, full snippet navigation and Rails context remain unverified. Signature prototype covers parenthesized calls and tested argument shapes only; other call forms, splats and keyword semantics need correct behavior. Preserve or extend verified source/URI/version safety when porting to production and prove actual GUI/TUI input/undo/save/cancellation; do not resolve these product tickets by re-running only the prototype fixtures.
