# 40: Display completion documentation and signature help

**What to build:** Inspect suggestion documentation and current-call parameters while typing or browsing completion.

**Blocked by:** 38: Show automatic and manual completion safely.

**Status:** resolved

- [x] Show documentation and signature/active-parameter details with usable controls in both adapters.
- [x] Honor negotiated encodings and request/version/context identity, with bounded async work and no stale popup resurrection.
- [x] Exercise overloaded/nested calls, Unicode, missing help and narrow modal layouts with real protocol payloads.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 63.

## Answer

Merged in ee498b1 (branch work/editor-40-docs-signatures). Parameter information: `textDocument/signatureHelp` on its own lane, asked by the new Parameter Info command (Ctrl+Shift+Space in both presets, F10/Find Action everywhere) and by typing a server trigger character however the typing is applied (paired delimiters and steps over a closing pair included), asked again while it shows, and shown only for the exact tab, version, caret and server session (`State::signature_help`); Escape closes it and a late answer never reopens it. Overloads report their count, labels are found by name or by offsets in the negotiated encoding (offsets inside a character are not placed), and parameter and signature documentation are shown. Suggestion documentation: the browsed suggestion's documentation is shown beside the list in both adapters, asked once per suggestion through `completionItem/resolve` on its own lane when the list lacks it. Client capabilities now declare `signatureHelp` and wider `resolveSupport`.
Evidence: core semantics tests (fake server) `parameter_info_follows_typing_through_overloads_and_nested_calls_and_closes_with_the_call`, `parameter_info_says_when_there_is_none_and_a_dismissed_late_answer_stays_dismissed`, `a_server_without_parameter_information_says_so`, `parameter_info_opens_when_typing_inserts_a_paired_parenthesis`, `browsing_suggestions_shows_their_documentation_asked_once_each`, and unit `parameter_labels_are_found_by_name_or_by_offsets_in_the_negotiated_encoding`; semantics filter 24 passed (two Ruby LSP tests fail environmentally: `ruby-lsp` is not installed here), core lib filter 87 passed. Parity replay `completion_typing_acceptance_and_dismissal_match_in_both_adapters` extended: `pick(` opens the same signature in both adapters, both draw it, a comma moves both to the second overload, Escape closes both (TUI/parity filter 23 passed), GUI filter 29 passed. Also fixes ticket 39's stale decline-notice expectation (`edits_are_declined_and_initialization_is_not_readiness`). Clippy `-D warnings` clean; panic-site ratchets at baseline. Performance deferred; limits in docs/editor-follow-ups.md (ticket 40 section). Real providers, native macOS, Wayland and physical terminals unverified.
