# Real-world comparison corpus

Pairs of one source file from two released versions of the same crate, used by `tests/corpus.rs` to snapshot
Change Blocks and Inline Changes (snapshots in `tests/snapshots/`). Each directory holds `left` (older) and
`right` (newer), copied unmodified from the crates as published on crates.io.

Licences: tokio is MIT (`licenses/tokio-LICENSE`); regex-automata, regex-syntax and syn are MIT OR Apache-2.0
(`licenses/<crate>-LICENSE-MIT`, `licenses/<crate>-LICENSE-APACHE`). Copyright remains with their authors.

| Directory | left | right | File |
|---|---|---|---|
| `tokio--src--fs--rename.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/fs/rename.rs` |
| `tokio--src--fs--try_exists.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/fs/try_exists.rs` |
| `tokio--src--fs--read.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/fs/read.rs` |
| `tokio--src--runtime--config.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/runtime/config.rs` |
| `tokio--src--runtime--metrics--worker.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/runtime/metrics/worker.rs` |
| `tokio--src--util--rand.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/util/rand.rs` |
| `tokio--src--util--sharded_list.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/util/sharded_list.rs` |
| `tokio--src--io--util--empty.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/io/util/empty.rs` |
| `tokio--src--runtime--time_alt--timer.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/runtime/time_alt/timer.rs` |
| `tokio--src--time--timeout.rs` | tokio 1.52.1 | tokio 1.53.1 | `src/time/timeout.rs` |
| `tokio--tests--tcp_echo.rs` | tokio 1.52.1 | tokio 1.53.1 | `tests/tcp_echo.rs` |
| `tokio--tests--time_rt.rs` | tokio 1.52.1 | tokio 1.53.1 | `tests/time_rt.rs` |
| `regex-automata--src--hybrid--error.rs` | regex-automata 0.4.5 | regex-automata 0.4.18 | `src/hybrid/error.rs` |
| `regex-automata--src--util--utf8.rs` | regex-automata 0.4.5 | regex-automata 0.4.18 | `src/util/utf8.rs` |
| `regex-automata--src--util--sparse_set.rs` | regex-automata 0.4.5 | regex-automata 0.4.18 | `src/util/sparse_set.rs` |
| `regex-automata--src--meta--limited.rs` | regex-automata 0.4.5 | regex-automata 0.4.18 | `src/meta/limited.rs` |
| `regex-automata--src--util--prefilter--memchr.rs` | regex-automata 0.4.5 | regex-automata 0.4.18 | `src/util/prefilter/memchr.rs` |
| `regex-automata--tests--dfa--regression.rs` | regex-automata 0.4.5 | regex-automata 0.4.18 | `tests/dfa/regression.rs` |
| `syn--src--lifetime.rs` | syn 2.0.48 | syn 2.0.117 | `src/lifetime.rs` |
| `syn--src--tt.rs` | syn 2.0.48 | syn 2.0.117 | `src/tt.rs` |
| `regex-syntax--src--unicode_tables--perl_decimal.rs` | regex-syntax 0.8.2 | regex-syntax 0.8.11 | `src/unicode_tables/perl_decimal.rs` |
