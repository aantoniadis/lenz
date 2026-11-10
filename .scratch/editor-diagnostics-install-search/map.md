# Editor feedback, server setup and Find

## Notes

Base: merged PR #51, `6f7a186`. Evidence: `perf-results/editor-diagnostics-install-search/`.

## Decisions so far

- [01: Syntax feedback](issues/01-tab-syntax-feedback.md): current peer payloads
  restore split colors; ordinary/pending dirty tab navigation remains covered.
- [02: Server setup](issues/02-language-server-install.md): direct Install, Node
  runtime before server, retry/cancel and curated Ruby/Go recipes with guards.
- [03: Find](issues/03-search-performance.md): bounded prepared literal scanner,
  zero-hit repaint, independent release comparisons and corrected GUI collector.

## Fog

Live Ruby registry discovery and a complete Go dependency download are blocked
here. Native macOS/Windows/Wayland, physical display/GPU latency, prolonged external
providers, indefinite leak freedom and fuzzy/project-search speed remain unverified.
