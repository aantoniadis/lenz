# 01: Expand the Reader text-snapshot boundary

**What to build:** Introduce the immutable indexed-text and shared source-metadata boundary alongside the existing Reader representation, keeping the current application working.

**Blocked by:** None (can start immediately).

**Status:** resolved

- [x] Existing Reader commands and both adapters remain green with the old representation available during migration.
- [x] Indexed UTF-8 chunks support bounded reads, newline and UTF-16 lookup, held snapshots and exact multi-range transactions across Unicode/CRLF boundaries.
- [x] A contiguous analysis Document can be constructed explicitly from preserved metadata without changing encoding; input/render access has no hidden full-document conversion.

**Spec coverage:** user stories 7, 114, 115, 118.

## Answer

Implemented by `bd113922`, integrated with the writer branch and merged by `7e5afe30`. Added immutable public TextSnapshot/identity/edit/window APIs and shared SourceMeta while retaining existing Reader representations. Document construction from decoded text preserves declared encoding/BOM/lossy/binary metadata; materialization is explicit rather than a lazy input/render getter.

Independent merged-tree verification: 6 public snapshot tests passed (one explicit measurement ignored normally), 400 core tests passed (19 ignored), targeted strict Clippy and diff checks passed. Focused release measurements and raw source/executable identity are retained under `target/perf/editor-implementation/01/`; merge logs are under `target/perf/editor-implementation/merge-01/`. Root index/chunk lookup costs and five short measured runs are not sustained adapter performance proof. Existing consumers remain available until the later migration/contract tickets.

Full integration gate passed on `177c167`: workspace/core, actual GUI/TUI adapters, panic ratchets, collector checks and separate release/size checks. This establishes the current read-only application integration; full editor performance remains the later contract/foundation acceptance work.
