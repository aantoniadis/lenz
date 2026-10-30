# PostgreSQL offline assistance prerequisite

2026-10-23. This selects a working in-process prerequisite for editor ticket 05, not completed SQL product integration. The implementation is [the shared provider](../../crates/code-core/src/postgres_assistance.rs); [its public-boundary tests](../../crates/code-core/tests/postgres_assistance.rs) exercise real parsers and formatting, without mocks or a language-server installation.

The provider combines three locked dependencies:

- `pg_parse 0.15.0`, whose bundled `pg_query.h` identifies **PostgreSQL 18.4**. `diagnose` calls its native `normalize` API, discards the normalized string, and uses the structured parser error. This invokes PostgreSQL's raw parser; it does not execute or plan statements. The string is never used as formatted output. This route avoids the AST decode error cleanup problem in this version's `parse` wrapper. A NUL precheck prevents its `CString::new(...).unwrap()` panic. Positive, one-based Unicode error cursors become original UTF-8 byte offsets, including EOF; unavailable cursors stay absent.
- `sqlparser 0.63.0`, with `PostgreSqlDialect` and AST visitors, supplies the independently tested SELECT scope resolver. It is **not** the syntax authority: it accepts `FROM` as an expression in a fixture where PostgreSQL errors at `FROM`. The native-parser regression pins the actual token, including multibyte source before the error.
- `sqlformat 0.5.0`, in `PostgreSql` mode, supplies whitespace formatting. Its dollar-string handling changed literal contents in the first fixture. The adapter now masks and restores each original dollar-string span, rejects significant-token or comment changes, and rejects output that makes native-valid input invalid. The last guard also protects PostgreSQL's newline-sensitive adjacent string literals. No AST `Display` output becomes a formatted document. Rejected output leaves the original draft with the caller.

Primary sources: [pg_parse structured errors](https://docs.rs/pg_parse/0.15.0/pg_parse/enum.Error.html), [native normalization implementation](https://docs.rs/pg_parse/0.15.0/src/pg_parse/query.rs.html), [sqlparser scope limitations](https://github.com/apache/datafusion-sqlparser-rs), [sqlformat options](https://docs.rs/sqlformat/0.5.0/sqlformat/struct.FormatOptions.html), and [PostgreSQL string lexical rules](https://www.postgresql.org/docs/18/sql-syntax-lexical.html#SQL-SYNTAX-STRINGS). The exact downloaded crate sources, rather than upstream main, were inspected before selection.

## Public contract and observed behavior

`complete(source, byte_offset)` returns SQL identifier insertion candidates. `complete_with_catalog` additionally takes an explicit immutable map of SQL-quoted, qualified relation names to columns. It never discovers a database or invents unknown schema columns. `diagnose` returns either success or the first native syntax diagnostic with an optional original byte position. `format` returns safe formatted text or an error. `function_help(name)` returns curated offline builtin documentation; `signature_help(source, byte_offset)` tracks the currently open call and zero-based argument index through incomplete nested calls. Neither operation determines SQL types or performs database validation.

Exact assertions cover:

- A derived-table alias with two projected columns; chained CTE projections and `SELECT *`; explicit CTE column names over `VALUES`; nested SELECT scope; quoted aliases containing spaces, quoted column names, and PostgreSQL unquoted case folding.
- Fully schema-qualified explicit snapshot metadata, aliased and partially typed columns, predicate completion, and hiding the original relation name after aliasing. Unknown schema columns remain absent offline.
- Bare-dot and partial-identifier holes in otherwise parseable SELECTs; an exact incomplete-format output; empty/comment-only documents; `DISTINCT ON`, JSON operators, arrays/casts, `ILIKE` and dollar strings.
- Original UTF-8 positions and EOF for native syntax errors; NUL, invalid cursor offsets, deep parentheses, and a multibyte separator that originally caused a slicing panic. Completion limits source size to 256 KiB, 4,096 significant tokens and 64 expression levels; deeper/oversized input returns an error rather than traversing unbounded ASTs.
- Curated `abs`, `date_trunc` and `jsonb_build_object` help; nested incomplete calls, `pg_catalog` qualification and active argument indices. These signatures are summaries of documented builtins, not exhaustive overload discovery. Sources: [numeric functions](https://www.postgresql.org/docs/18/functions-math.html), [date/time functions](https://www.postgresql.org/docs/18/functions-datetime.html), [JSON functions](https://www.postgresql.org/docs/18/functions-json.html).

The provider has text/snapshot inputs and text/value outputs; no connection string, executor, process-launch or database-client input exists. The release probe passed actual `DROP`, `UPDATE`, escaped-quote `COPY ... TO PROGRAM` and export documents through the native parser and formatter. Owned fixture bytes stayed unchanged and export/program sentinels stayed absent. `strace -f` observed **zero network socket/connection/message syscalls** and only the initial probe `execve`; there was no provider child process. This observes the real offline route. The fixture file is a filesystem sentinel, not a claim to have tested rows in a connected PostgreSQL database. Previous readiness metadata tests remain separate evidence for the optional catalog-acquisition boundary.

## Evidence and reproduction

Raw local evidence is under `target/perf/editor-implementation/05/` in the main checkout's shared target directory. `identity.json` records compiler/platform, base revision, exact source hashes, executable SHA-256 and build command; `sources/` and `postgres-assistance-probe` preserve the measured implementation. `*-red.log` retain meaningful failing behavior during development. `boundary.strace`, `boundary-summary.json` and `boundary.log` preserve observed syscalls, destructive source payloads and real results. `measurements.json` retains 100 raw timings per operation, source/response examples, RSS samples and actual sustained duration; `stress.log` retains deterministic assertion results.

Integration replaced seven guarded string slices with checked access to meet the existing panic-site budget. Focused checks are retained under `target/perf/editor-implementation/merge-05/`; the timings below describe the preserved pre-integration executable, not a new measurement of that guard-only change.

The final post-merge release process completed **1,353,858 deterministic workload cycles in 60.000 seconds**. Across 100 measured requests per operation:

| Operation | Median / p95 |
| --- | ---: |
| Query-local completion | 0.009684 / 0.015459 ms |
| Native syntax error with original position | 0.000611 / 0.001142 ms |
| Guarded formatting | 0.029451 / 0.046598 ms |
| Nested signature help | 0.000822 / 0.001242 ms |

Sampled stress RSS stayed between **8.41 and 8.52 MiB**; the final-quarter median was 8.51 MiB. The sampler retains one status record per second, so its allocations are included. A separately retained first run (`first/`) produced the same deterministic outcomes, zero network/provider-child syscalls, and a similarly bounded short-run RSS range. Large supplied catalogs and long-running product workloads were not measured.

Build with `CARGO_BUILD_JOBS=4` and an isolated `CARGO_TARGET_DIR` for each worktree. Concurrent worktrees must not share Cargo build outputs; source/artifact collisions invalidate those measurements:

```sh
cargo test --locked -p code-core --lib --test postgres_assistance
cargo clippy --locked -p code-core --all-targets -- -D warnings
cargo test --release --locked -p code-core --test postgres_assistance --no-run
```

Run the produced release test executable directly under `strace -f -qq -e trace=network,process,openat -o boundary.strace` with `--nocapture --test-threads=1`; this keeps Cargo/tool-download traffic outside the observed provider boundary. For the 60-second success/error/format/completion workload, set `POSTGRES_EVIDENCE_DIR` to an owned directory and run that executable with `--ignored --exact postgres_assistance_measurements --nocapture --test-threads=1`.

Measurements are provider-process wall time and Linux RSS, not frontend display latency, GPU memory, leak proof or a baseline-preservation claim. There was no previous implemented provider to compare; the module has no GUI/TUI caller. The shared development machine was not CPU-isolated. Native macOS, production startup/interaction behavior and GUI/TUI parity remain unverified by this prerequisite.

## Required handoff to ticket 61

The accepted SQL requirements remain required. This route proves each prerequisite category with finite fixtures, not semantic completeness. Extend and test the following before advertising complete product support:

- Arbitrary incomplete-statement recovery, editing inside unfinished quoted identifiers, UNION output inference, recursive CTE inference, qualified wildcard outputs, function-return columns/types, and complete PostgreSQL name visibility rules (including non-LATERAL derived tables, CTE visibility and ambiguous unqualified columns).
- A broader offline builtin catalog, overload/extension metadata, parameter validation, arrays/grouping in signature argument tracking and function completion under `pg_catalog`. Current help is a curated three-function set.
- Safe successful formatting for currently rejected constructs, including newline-concatenated literals; preserve source on every rejection. Native-validity/token guards are protections, not proof of every PostgreSQL whitespace semantic.
- A shared request identity/cancellation envelope and bounded worker scheduling; catalog freshness and explicit metadata acquisition; dialect switching and stale-result rejection; usable controls, presentation and edit application in both adapters. The current synchronous API is not called on an input/render path.
- Native-target build verification: `pg_parse` needs make, a C compiler and libclang/bindgen at build time. No user tool installation or application auto-install was added.

Reuse the source-to-scope/provider contract for the future DuckDB adapter where dialect semantics actually agree; do not copy a competing resolver into a frontend. Native PostgreSQL diagnostics and the guarded PostgreSQL formatting route remain dialect-specific.
