# DuckDB query-local assistance prerequisite

Ticket 06 provides a usable synchronous assistance API in
[`duckdb_assistance`](../../crates/reader-core/src/duckdb_assistance.rs).
It is separate from the later editor/provider/UI integration in ticket 62.
PostgreSQL keeps its public API and native grammar authority; both routes now
reuse the private [`sql_assistance`](../../crates/reader-core/src/sql_assistance.rs)
scope resolver, guarded formatter and call-nesting traversal.

`complete(sql, byte)` performs no process or database I/O.
`complete_with_catalog(sql, byte, snapshot)` accepts an explicit immutable map of
SQL relation names to column names. It opens no source, refreshes no metadata and
cannot execute statements. Empty `ready.` and partial `ready.f` in a FROM clause
return `fixture` only when `ready.fixture` is present in that explicit snapshot.
Unknown offline schema names intentionally produce no invented relation names.

The [public-boundary tests](../../crates/reader-core/tests/duckdb_assistance.rs)
replay the original failed readiness documents with exact expected results:

| Query-local scope | Expected insertion |
| --- | --- |
| Derived alias `a.` | `alias_col` |
| CTE `c.` | `cte_col` |
| Nested wildcard-derived `q.` | `nested_col` |
| Quoted output through `q.` | `"Mixed Column"` |
| Explicit catalog `ready.` / `ready.f` | `fixture` |

Additional assertions cover CTE wildcard chains, explicit quoted CTE columns,
quoted aliases with spaces, inner scopes, bare column qualifiers, partial column
names, FROM-first queries, GROUP BY ALL, and EXCLUDE projection filtering.
Unsupported/malformed completion documents return errors, including invalid UTF-8
positions and resource limits (256 KiB, 4096 tokens, expression depth 64).

`DuckDb::new(explicit_executable)` enables native `diagnose`, `format`,
`function_help` and `signature_help`. Requests launch the selected installed CLI
with `-no-agent -init /dev/null -json :memory:`. The fixed bootstrap disables
extension autoinstall, autoload and external access before assistance. Native
requests have a three-second process limit, bounded captured output and a 32 KiB
NUL-free source limit. Failures do not return command lines/source in tool errors.
This is a native in-memory engine process, not an external catalog connection.

Diagnostics use DuckDB v2.0.0's `json_serialize_sql` with the entire original
source escaped into one SQL string literal. The parser checks all statements
before JSON serialization; its specific non-SELECT serialization rejection is
accepted as syntax-only success. A valid first non-SELECT followed by malformed
SQL still returns the later parser error. No binding or actual database validation
is claimed. Original UTF-8 byte offsets and lengths are checked before returning
ranges; multiline non-BMP input points exactly at the original offending token.
FROM-first, EXCLUDE, GROUP BY ALL, list literals and lambda syntax are tested
against the native parser, independently of sqlparser's DuckDbDialect.

Formatting uses sqlformat whitespace layout with DuckDB token equivalence guards,
protected dollar literals, and native syntax preservation for valid inputs.
Comments/literals remain present byte-for-byte; malformed lexical input is refused.
Incomplete parseable token streams can be formatted. Native builtin help comes
from a fixed `duckdb_functions()` query with a quoted name, at most eight
overloads and bounded documentation. Signature help handles incomplete nested
calls and commas inside list literals. Completion includes four curated builtin
names; help is not limited to those four.

The native boundary test observes exact CLI arguments and fixed assistance SQL
for DROP, UPDATE, COPY/export, ATTACH, a quote escape and a malformed second
statement. Fixture creation and read-only verification are explicitly separate.
The disposable DuckDB database retains identical bytes and rows; no export
sentinel appears. Linux strace observes five assistance processes with zero
`connect` calls and zero fixture/export opens. This proves these observed routes;
it is not a statement about a future catalog adapter.

Evidence is retained under `target/perf/editor-implementation/06/`: red/green logs,
`native-boundary.jsonl`, `native-fixture.json`, `syscalls/`, `syscall-summary.json`,
`measurements.json`, `native-resources.json`, `identity.json` and check logs.
The release API sample has 20 completions and eight syntax/help requests each.
Median completion was 0.013 ms, syntax 30.93 ms and help 46.02 ms. Fifteen separate
native parser process samples measured 25.49 ms median and 45.86 MiB median child
maximum RSS (Linux wait4). These short, contended-host prerequisite samples are
not production frontend, persistent-worker, sustained-memory or display claims.

Limits remain explicit: no UNION/recursive CTE or table-function output/type
inference; wildcard RENAME/REPLACE and general DuckDB case-insensitive quoted-name
resolution are not complete. General malformed-query recovery is unsupported
beyond replacing the current partial identifier. There is no external metadata
reader, editor application, async cancellation, dialect persistence or GUI/TUI
integration here. Native macOS, production performance and those ticket 62
behaviors remain unverified. No tool installer or new dependency was added.

Final verification on the merged prerequisite branch: all 539 ordinary
reader-core tests passed (including 16 PostgreSQL and five DuckDB assertions),
three explicit native DuckDB tests and the release measurement passed, strict
reader-core Clippy passed, and the Reader panic-site ratchet retained its existing
limits (including 108 string-slice sites). Documentation links and diff whitespace
checks passed. GUI/TUI input checks and the full workspace/local release gate were
not rerun for this unused prerequisite API; editor integration remains later work.
