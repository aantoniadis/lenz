# Reader snapshot migration: measured costs and limits

Measured on Linux on 2026-10-23 for integrated-editor ticket 07. The current candidate is integrated for review; **the user accepted the documented measured costs on 2026-10-23**. This is a core read-path migration. It does not enable editing or establish GUI/TUI rendering migration, background-analysis admission, or native macOS acceptance.

## Source and method

The release baseline is `177c167`. The final measured snapshot representation is `e5d5546`, using shared original text and line-index ranges, approximately 4 KiB chunks, indexed UTF-16 measures and local owned indexes for edited chunks. A subsequent cold rename-boundary correction (`a78567a`, integrated at `39a830c`) fixes source metadata when reopening a renamed cached file. The measurements below identify the earlier executable explicitly; that correction has focused regression coverage and is included in the full integration gate.

Baseline and candidate were compiled in separate target directories with the same release settings and no release overrides. Retained identities include compiler/platform, source hashes, executable hashes and raw samples. Opening checks use 100 trials per fixture: 50,400 bytes / 2,001 lines and 945,000 bytes / 37,500 newlines (the existing `1mib` fixture label). The endpoint is core text availability, before syntax completion; it is not a displayed GUI frame or terminal output. Syntax readiness is reported separately.

The host also runs unrelated work. Quiet comparisons exclude our overlapping builds, tests and stress. They do not establish an exclusive-machine or physical-display result. Opposite run orders were retained.

## Final measured representation

| Core operation | Baseline median → candidate median | Baseline p95 → candidate p95 |
| --- | ---: | ---: |
| 50,400-byte first text, first clean pair | 0.095 → 0.103 ms | 0.174 → 0.171 ms |
| Same fixture, clean reverse pair | 0.097 → 0.103 ms | 0.154 → 0.207 ms |
| 945,000-byte first text, first clean pair | 0.888 → 0.912 ms | 1.395 → 1.122 ms |
| Same fixture, clean reverse pair | 0.888 → 0.976 ms | 1.269 → 1.641 ms |
| Cached tab switch, 2,000 interactions | 4.308 → 4.558 µs | 8.196 → 8.216 µs |
| Cached Back/Forward, 1,000 interactions | 5.300 → 5.480 µs | 10.129 → 10.730 µs |

The remaining opening median increase repeats: 6–8 µs for the small fixture and 24–88 µs for the larger one. Its size and tail behavior vary. Large-fixture syntax medians were 224.582 → 225.196 ms in the first pair and 225.206 → 225.274 ms in reverse. Do not describe these measurements as baseline preservation.

The cached endpoint includes dispatch and asynchronous settling, with visible file checks sharing the existing three-worker pool with background parsing. It does not measure a displayed frame. The cached-switch run also contains a 14.419 ms maximum versus 0.654 ms in the baseline. Ten alternating baseline/candidate pairs then reproduced tails in both binaries. Across 20,000 switches each, baseline/candidate had three/four samples above 10 ms (maxima 13.564/13.130 ms); across 10,000 Back/Forward operations each, two/three exceeded 10 ms (12.544/13.885 ms maxima). The tail is therefore not candidate-specific in these controls; its exact cause remains untraced. Aggregate switch median was 4.358 → 4.569 µs, p95 9.237 → 8.866 µs; Back/Forward median 5.230 → 5.391 µs, p95 12.113 → 10.630 µs. These results show slightly higher ordinary medians and lower p95, not a universal speedup. Raw pair order, samples and executable hashes are retained under `cached-repeat-pairs/`; percentiles use the existing benchmark convention `round(q * (n - 1))`.

## Sustained correctness and memory

The same owned 37-file workload performs navigation, selection, search, cache turnover and controlled external rewrites, asserting state and exact fixture bytes. Baseline completed 1,239,700 iterations / 24,794 rewrites in 180.20 seconds; the measured candidate completed 1,222,892 / 24,457 in 180.15 seconds. These single sustained runs are not a throughput-preservation proof.

After a 60-second warmup, ten 12-second windows sampled Linux process VmRSS. The median of the first three versus last three windows was 17.93 → 17.96 MiB in baseline and 17.02 → 17.23 MiB in candidate. Sampled process peaks were 43.03 / 33.42 MiB. This is short-run process memory, excluding GPU and external-server allocations; it is not leak proof. Retained shared backing allocations and history reclamation still require the later buffer/history and ownership-contraction checks.

## Experiments, including unsuccessful checks

- Initial chunk construction copied source bytes and newline indexes. Sharing the original text removed the duplicate text allocation. Reusing existing line offsets and the ASCII UTF-16 fast path reduced redundant scanning.
- Production dispatch-wide registry reconciliation was removed; current snapshots are admitted at load, refresh, virtual retrieval and rename boundaries. Test fixture construction uses explicit admission. Cache accounting runs at ownership transitions.
- An isolated intermediate representation still showed approximately 69–74 µs additional larger-fixture opening time. Its opening-suite RSS was also higher; attribution was not established.
- Preserving exact-size iteration and fusing prefix/storage accounting (`3458053`) was a small cleanup, but did not resolve opening latency: one pair measured 0.856 → 0.973 ms. Its separate 180.10-second stress run passed; it is an intermediate result.
- Sharing original newline-index ranges (`e5d5546`) removes roughly 300 KB of duplicated offsets and 231 per-chunk index allocations for the larger fixture. It reduced the measured overhead, while the final repeated increase above remains.
- Earlier shared-target Rust timing samples cannot establish artifact provenance after a real concurrent build collision. They are retained under `unverified-shared-target/` and excluded from acceptance. A later batch overlapped private validation; it is also explicitly excluded rather than averaged into quiet comparisons.

## Evidence and remaining gate

Raw evidence is retained locally under `target/perf/editor-implementation/07/`: `lineshare-configuration.json`, `lineshare-*-open*.{json,log}`, tab logs, `lineshare-candidate3m-stress.*`, `isolated-baseline3m-stress.*`, and `executables/manifest.json`. `compatibility-callers.json` identifies remaining legacy Document readers; tickets 08–11 own their migration. Private checks cover core/public APIs, Unicode/CRLF and chunk boundaries, held snapshots, cache eviction, independent sessions and rename/reactivation.

The full `scripts/ci-local.sh` gate passed (exit 0, 444.18 seconds) at `39a830c`: strict workspace Clippy, all panic ratchets, workspace/core tests, isolated content-memory checks, 261 Diff GUI / 144 Reader GUI / 91 Reader TUI checks, 66 Python collector tests, separate terminal release builds and the Diff release/size check. Stripped Diff size was 68,508,728 bytes, unchanged from the prior writer/snapshot integration gate. Evidence and unchanged source/config hashes are retained in `target/perf/editor-implementation/merge-07/`. Native macOS, physical display latency, standalone frontend startup, complete GUI/TUI editing parity and long-session memory remain unverified by this prerequisite. The user explicitly accepted the measured opening and ordinary cached-operation costs on 2026-10-23. This closes ticket 07's performance decision; subsequent migrations retain their own correctness, parity and performance gates.

The [benchmark guide](../benchmarking.md) defines the measurement requirements; the [optimization notebook](../optimization-notebook.md) records the engineering decisions.
