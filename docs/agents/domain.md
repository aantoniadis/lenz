# Domain docs

Lenz uses a single shared domain context across its Rust workspace and GUI/TUI adapters.

## Before exploring or writing a spec or ticket

- Read root `GLOSSARY.md` when present and use its terms for domain concepts.
- Read relevant decisions under `docs/adr/` when present.
- If a future root `GLOSSARY-MAP.md` exists, follow it to relevant context glossaries and context-scoped ADRs as well.

If these documents are absent, proceed silently. Create them lazily through domain modeling when terms or architectural decisions are actually resolved.

## Consumer rules

Use the glossary’s vocabulary in issue titles, plans, hypotheses and tests. If an existing ADR conflicts with proposed work, identify the conflict explicitly before changing that decision. Keep resolved terms and decisions in their authoritative domain documents rather than duplicating them in agent instructions.
