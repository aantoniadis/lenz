# 25: Respect project editing and formatting preferences

**What to build:** Choose project-specific indentation/formatting settings and resolve EditorConfig without touching unrelated files.

**Blocked by:** 23: Verify and enable the foundation on validated targets.

**Status:** resolved

- [x] Expose applicable settings and precedence through both adapters, persisting project-local preferences without source text or credentials in shareable diagnostics.
- [x] Resolve project indentation and configured tools without installing/upgrading tools or executing environment-file values.
- [x] Keep explicit formatting the default and store the opt-in format-on-save choice for the formatting slice.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 42, 71, 72.

## Answer

Merged in f9c812a (branch work/editor-25-preferences, 2332a52). Focused core/GUI/TUI checks + clippy passed; performance deferred to end of epic per user. Follow-ups in docs/editor-follow-ups.md.
