# 62: Deliver DuckDB offline assistance

**What to build:** Write DuckDB queries with complete query-local assistance and DuckDB-aware syntax/formatting.

**Blocked by:** 06: Resolve DuckDB query-local completion gaps; 60: Choose and persist a visible SQL dialect; 39: Apply completion imports and snippet edits atomically; 40: Display completion documentation and signature help; 41: Show diagnostics and apply single-file quick fixes; 42: Format and organize imports under project settings.

**Status:** resolved

- [x] Integrate the verified resolver for previously missing alias/CTE/nested/quoted/qualified names and actual function help.
- [x] Exercise EXCLUDE, GROUP BY ALL, malformed/incomplete text and exact original-source diagnostics/edits through both adapters.
- [x] Observe zero metadata connections or edited-statement/EXPLAIN execution; switching dialect rejects obsolete replies.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 93, 94, 95, 99, 101.

## Comments

Read `docs/research/duckdb-query-local-assistance.md` and reuse the shared private SQL scope helper. Ticket06 proves finite expected-name/native-boundary fixtures, not complete product assistance. Extend UNION/recursive CTE/table-function output inference, wildcard RENAME/REPLACE, general case-insensitive quoted-name resolution and malformed-query recovery where required. Completion currently has four curated builtin names; native help can return bounded real overloads. The native serializer only serializes SELECT, so preserve syntax-only success for its exact non-SELECT limitation and actual parser failures for malformed/later statements. Add async cancellation, dialect/catalog identity, persistence and usable current-version edits/diagnostic presentation in both adapters. Re-measure release API and actual product latency/memory; earlier shared-target API timings are explicitly excluded. No metadata reader or tool installer exists yet.

Product integration must also move edited SQL out of process arguments: the prerequisite native CLI currently passes its quoted script through `-c`, so current source is visible in process command lines. Use a bounded stdin transport with the same timeout/output/cancellation guarantees and retain the fixed-operation/no-execution boundary tests before exposing this route to editor drafts.

When moving the CLI script to stdin, include multiline edited literals containing line-leading DuckDB dot commands (for example `.shell` and `.read`) in the observed no-execution boundary tests; the CLI transport must keep those bytes inside quoted SQL data rather than interpreting meta commands.


## Answer

Merged at `cbedfdb` (implementation `7dd78e7`). DuckDB editor requests use the shared SQL scheduler and exact context fences. Query-local completion works without the native tool; a chosen installed DuckDB 2.0.0 CLI supplies syntax, bounded native overload help and guarded formatting. Both adapters replay EXCLUDE/GROUP BY ALL, current-draft completion, exact Unicode/CRLF diagnostics, Parameter Info, formatting and Undo with source files unchanged.

The native route no longer puts document text in argv. Fixed CLI arguments and a single-line, hex-encoded stdin data expression prevent edited `.shell`/`.read` lines from reaching the CLI grammar. The transport caps output, time and engine resources and cancels the owned process group. Six actual native boundary traces observed zero connect calls, no private database/export opens, unchanged database bytes/rows and no sentinel, including multiline dot-command and quote-escape documents. The real parser's exact non-SELECT serialization limit remains syntax-only success; malformed later statements still report errors.

Shared scope inference adds case-insensitive quoted-name lookup for DuckDB, wildcard RENAME/REPLACE outputs and set/known function outputs. sqlparser's inherited DuckDB dialect has only its missing native-supported RENAME flag enabled. No database metadata is acquired automatically. See [DuckDB offline editing](../../../docs/code/providers.md#duckdb-offline-editing) for explicit tool setup and remaining inference/lexical limits.

Verification: 11 SQL core regressions, 6 pure DuckDB tests, 4 real native tests, native editor test, 21 PostgreSQL regressions, real GUI/TUI replay, transport output/cancellation checks, strict touched-crate clippy and both panic-site ratchets pass. The argv boundary test failed against the old transport before implementation. Native macOS/Wayland/physical terminals and the end-of-epic performance/stress campaign remain unverified/deferred; startup/normalization costs are in editor follow-ups.
