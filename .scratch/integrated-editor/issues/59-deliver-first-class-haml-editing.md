# 59: Deliver first-class Haml editing

**What to build:** Edit Haml templates with correct indentation, Ruby/Rails context and Haml-appropriate markup edits.

**Blocked by:** 57: Route embedded-language edits through source maps; 50: Deliver Rails-aware application assistance; 51: Deliver HTML and embedded-language assistance.

**Status:** resolved

- [x] Recognize real Haml syntax and compound extensions automatically, including nested indentation and interpolation.
- [x] Verify helper/member/signature/diagnostic behavior and Haml-valid tag/attribute replacements and formatting, not HTML snippets pasted into Haml.
- [x] Replay Unicode/CRLF/multiline/stale-response cases in both adapters with exact source ranges and coherent undo.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 79, 80.

## Comments

Ticket04 provides a verified bounded route, not full template support. Carry its research-note limits into implementation: Haml dynamic attributes/control blocks/filters/comments/escaped lines and nested/incomplete interpolation remain unmapped; ERB escaped closing delimiters/trim modes, mixed markup/Ruby routing, full snippet navigation and Rails context remain unverified. Signature prototype covers parenthesized calls and tested argument shapes only; other call forms, splats and keyword semantics need correct behavior. Preserve or extend verified source/URI/version safety when porting to production and prove actual GUI/TUI input/undo/save/cancellation; do not resolve these product tickets by re-running only the prototype fixtures.


## Answer

Merged as `2f455541a58d11b00823b111359ec5d17a214c6a` (implementation `a8db5ea`).
Haml tag/attribute completion and spaces-only structural Enter are shared Session commands,
with exact GUI/TUI input, rendering and Undo replays. Ruby projections now cover nested
indentation-controlled blocks, dynamic hashes/variable attributes, nested interpolation and
comma/pipe continuations. A per-URI Rails view context admits application/controller helper
members without booting Rails. Generated context is unmapped and omitted from file symbols;
edits changing block shape or template boundaries are refused whole.

Configured haml-lint safe autocorrections run on stdin, with the project's bundle when it
locks haml_lint, current-version admission and one Undo transaction. Original files and the
Git index are not formatter targets. CRLF and missing final newlines survive the verified
fixtures. A File.write sentinel never executes. Both adapters replay real completion,
parameter information and project-configured formatting, then restore exact source with Undo.

Verification: new projection tests fail against the old mapper; 11 Haml core tests,
7 formatting regressions, 4 template semantic regressions, Haml 7.5.1 compiler/parser checks
on four fixtures, 2 real Ruby LSP tests, the real haml-lint safety test, and both local and
real GUI/TUI Haml replays pass. Touched-crate clippy with warnings denied and both Reader
panic-site ratchets pass. See [capabilities and limits](../../../docs/code/providers.md#haml-editing).

Limits remain explicit: filters, object references, multiline attribute lists, tab-indented
source and regex/percent literals in mapped interpolation/attributes are not mapped; Haml
syntax highlighting/live grammar problems and dynamic/namespaced Rails context are not
claimed. Native macOS/Wayland/physical terminals are unverified. Per the handoff decision,
performance/stress and the full epic gate remain deferred, with costs recorded in
[follow-ups](../../../docs/editor-follow-ups.md#ticket-59-haml-authoring-costs-and-remaining-contexts).
Ticket 58 remains claimed by its other implementer.
