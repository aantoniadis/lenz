# 43: Explain active tools, environments and missing capabilities

**What to build:** See which language environment/tool is active and why particular assistance is unavailable.

**Blocked by:** 25: Respect project editing and formatting preferences.

**Status:** resolved

- [x] Expose configured executable/environment/project, readiness, missing capabilities and clear setup guidance through both adapters.
- [x] Discover installed/project-provided tools lazily without automatic installation or unrelated project changes.
- [x] Keep editing available while tools fail/index and keep source text/credentials out of logs or shareable configuration.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 42, 73, 74.

## Answer

Merged in bec4a06 (branch work/editor-43-tool-details). Server Details (both adapters, one core text) now adds to the existing status, workspace, executable, version and setup guidance: the Java or Node.js runtime in use (chosen or the first on PATH), the names of environment variables the provider settings add with their values withheld, and the assistance the running server declared versus what it lacks (missing commands stay disabled with that reason; before start it says capabilities are not known yet), with an explicit statement that typing, undo, save and text search never wait for the server. Discovery stays lazy and never installs anything.
Evidence: core semantics tests `server_details_name_the_offered_and_missing_assistance_and_never_show_env_values_or_source` and `editing_continues_while_the_language_server_cannot_start` (typing and Save work while the server fails to start, exact file bytes); parity replay `server_details_explain_missing_assistance_alike_and_hide_environment_values` (same rows in both dialogs, env names without values, no source text, Format Document's disabled reason alike, Escape dismissal, file unchanged). Core lib filter 10 passed, semantics filter 4 passed, TUI/parity filter 12 passed, reader GUI filter 14 passed. Clippy clean; panic-site ratchets at baseline. Gaps (arguments shown as configured, runtime row only for Java/Pyright, dynamic registration, per-language environments) in docs/editor-follow-ups.md (ticket 43 section). Real providers, native macOS, Wayland and physical terminals unverified.
