# 04: Resolve template projection and encoding blockers

**What to build:** Establish a reproducible, correct Ruby/markup assistance route for real ERB and Haml syntax before production integration.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Use real provider payloads to demonstrate correct completion, signatures and diagnostic ranges for Unicode, CRLF, interpolation, multiline Ruby and indentation.
- [x] Demonstrate automatic region recognition and markup replacement syntax; explicit fixture regions or candidate labels alone cannot satisfy acceptance.
- [x] Resolve the known native ERB range and encoding-dependent signature defects through a verified alternate route or fix; retain failures and keep unresolved capability gaps open.

**Spec coverage:** user stories 79, 80.

## Answer

Merged test-only route by `1e996cdf`, with dependency-portability correction `45d3ce2`. Real Ruby LSP/HTML providers repeatedly demonstrate six automatic ERB/Haml Ruby projections, four actually applied/compiled markup replacements and fourteen exact signature boundary cases. The route uses the actual ERB scanner and an owned Haml7.5.1 parser/compiler, strict original-source maps and a private version-scoped UTF8 signature-target fix; installed provider files remain unchanged.

Normal mapping contracts remain runnable without optional Ruby/gems (3 pure contracts pass; 4 recognition checks explicitly skip). Required real-provider acceptance fails missing prerequisites, and passed with the owned environment. Strict Clippy and diff checks pass. Source, repro, timings, tool identities, retained failed hypotheses and bounds are in the template-resolution research note and integration evidence `target/perf/editor-implementation/04/` plus `merge-04/`.

This resolves the bounded feasibility prerequisite only. Production source routing, all Haml contexts, Rails helpers, all Ruby signature forms and actual adapter behavior remain tickets57–59; no completed template feature is claimed.
