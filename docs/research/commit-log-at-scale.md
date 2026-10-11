# Commit log and branch graph at scale

Research for `.scratch/lenz-0.2/issues/02-commit-log-at-scale-research.md`: how Lenz can read the commit log and draw an IntelliJ-style branch graph fast on huge repositories, still shelling out to `git` (≥ 2.30).

## Answer in short

- **Stream one `git log --topo-order` process; never page.** With a commit-graph file, the first screen of the Linux kernel (1.48M commits) arrives in **5 ms**. The full 1.48M-row table with author and subject takes **~9 s**, and rows fill in as they arrive.
  - `--skip=N -n 100` is O(N): 0.8 s at a million rows even with a commit-graph. So keep the whole table in memory instead of paging.
- **The commit-graph file is what makes it fast.**
  - Without one, `--topo-order` and `--date-order` show nothing for **8.4 s**: git walks the whole history before emitting its first row.
  - With one, the first row arrives in 5 ms.
  - Writing it takes 12 s (85 MB). Adding Bloom filters takes 74 s (105 MB).
  - A fresh clone has none. Any `git gc` or auto-maintenance writes one; we saw a lazy fetch in this very clone write one within seconds.
  - Whether Lenz may write one is a product decision (see "Decisions for the spec").
- **Walk branches and remotes, not tags.** Pass `HEAD --branches --remotes` and decorate tags from `git for-each-ref` (1 ms for 951 refs).
  - `--all` / `--tags` include tags back to v2.6.11. That pins the generation-number walk to the root and costs **1.16 s** before the first row even with a commit-graph. Only tags from v6.1 onwards cost 244 ms. IntelliJ leaves out `--tags` from its first block for the same reason.
- **Lane layout is negligible.** A streaming "active lanes" pass over all 1.48M commits takes **36 ms** in total.
  - `--topo-order` gives 2.6× fewer lanes than `--date-order`: max 214 vs 577 active lanes, average 91 vs 236.
  - Store one column and a CSR parent list per row. Work out each visible row's edges on demand, and cut edges of 30 or more rows into arrow stubs as IntelliJ does. After cutting, the edges crossing a row are p99 4, avg 0.6. The maximum is 534, at the largest octopus merges (`2cde51fbd0f3` has 66 parents).
- **Memory:** about 140 B per commit for a full table (ids, parents, column, author id, time, subject), so **~205 MB estimated / 245–265 MB RSS** for Linux. The graph alone is 72 B per commit. Never store a lane snapshot per row: that would be 134M–350M entries, 0.5–1.4 GB.
- **Filters:**
  - **`--author` and `--grep`** walk and inflate every commit, so they take about 9 s with or without a commit-graph. First matches arrive in 25–190 ms.
  - **Path filters** are fast thanks to history simplification, and Bloom filters make them a further 1.8–4.8× faster (`kernel/sched/core.c` 0.90 → 0.53 → 0.20 s). Under a path filter, pass **`--parents`**, or `%P` names parents that aren't in the output.
  - **`--follow`** takes 6–12 s, gets no Bloom benefit, and works for a single file only. In a blobless clone it also fetches blobs over the network: 115 MB here. Keep it for single-file history.
- **gix is not materially faster for this.**
  - Decoding every commit costs the same: gix 9.05 s vs `git log` 8.7–9.0 s.
  - Its topological walk is slower: 1.6 s to the first row and 2.8 s total, vs 1.15 s and 1.56 s for `git rev-list`. Without a commit-graph it is 16 s vs 7 s.
  - It has no path-limited log, no `--follow`, no Bloom filter reading, and no commit-graph writing, and it is pre-1.0 with API churn.
  - The one real speed-up is parallel decoding, and the CLI can do that too: `rev-list --parents` plus 16 parallel `git log --no-walk --stdin` processes give the full table in **2.7 s** instead of 11.4 s.

## Setup

- **Repository:** `git clone --filter=blob:none https://github.com/torvalds/linux`, HEAD `ce1e0223d8ad` (2026-10-01).
  - 1,484,278 commits reachable from HEAD and from `--all`; 112,200 merges; 101 commits with ≥ 10 parents.
  - 951 refs (948 tags). 1.96 GiB pack.
  - Blobless is enough for log work: commits and trees are local, only blobs are lazy.
- **Machine:** git 2.55.0, Linux 7.2, NVMe, warm page cache.
- **Method:**
  - Each case runs 3 times; the table shows the median.
  - "First 100" is the time until 100 rows have been read from git's stdout, roughly a screen plus margin. "Full" is the time until git exits.
  - Output format: `--format=%H %P%x1f%an%x1f%ae%x1f%ct%x1f%s`.
  - "git RSS" is the peak resident memory of the git process. It includes mmapped pack pages, so it is not heap.
- **Commit-graph states:**
  - "none" is the fresh clone. `maintenance.auto` and `gc.auto` were later turned off, after a `--follow` run's lazy blob fetch triggered auto-maintenance, which wrote a commit-graph chain mid-benchmark. Every case it could have touched was rerun without it.
  - "CG" is after `git commit-graph write --reachable`.
  - "Bloom" is after `… --reachable --changed-paths`.
- **Probes:** these were throwaway and are not in the repo:
  - `bench.py`: timing harness.
  - `layout`: a Rust lane-layout and memory probe fed by real `git log` output.
  - `gixwalk`: a gix 0.88 rev-walk.
  - `par.py`: parallel metadata decode.

## Measurements

### Whole-history log (1,484,278 rows)

| command | none: first 100 / full | CG: first 100 / full |
|---|---:|---:|
| `rev-list --count HEAD` | — / 7.57 s | — / 0.92 s |
| `log HEAD` (default order, reverse-chronological) | 3 ms / 11.3 s | 3 ms / 8.66 s |
| `log --all` (default order) | 16 ms / 9.09 s | 21 ms / 8.68 s |
| `log HEAD --topo-order` | 8456 ms / 11.2 s | **5 ms** / 9.00 s |
| `log HEAD --date-order` | (≈ topo) | 5 ms / 9.52 s |
| `log HEAD --branches --remotes --topo-order` | (≈ topo) | **5 ms** / 9.70 s |
| recommended loader: `log -z --topo-order --parents --format=… HEAD --branches --remotes` | — | **5 ms** / 8.99 s |
| `log --topo-order HEAD --tags='v6.[1-9]*'` | — | 244 ms / 8.93 s |
| `log --all --topo-order` (948 tags as tips) | 8434 ms / 11.2 s | 1164 ms / 8.94 s |
| `log --all --date-order` | 8515 ms / 11.2 s | 1147 ms / 8.96 s |
| `log --all --topo-order` + `%D` decorations | 8768 ms / 11.9 s | 1161 ms / 8.94 s |
| `rev-list --all --topo-order --parents` (graph only, no decode) | 7114 ms / 7.75 s | 1152 ms / **1.56 s** |
| `log --graph --all --format='%h %s'` (git draws the graph) | 9020 ms / 18.2 s | 1172 ms / 14.6 s |
| `log --topo-order --skip=1000000 -n 100` | 8.91 s | 0.82 s |
| `log --skip=1000000 -n 100` (default order) | 5.42 s | 0.61 s |
| `for-each-ref` (951 refs) | 1 ms | 1 ms |

What the table shows:

- **Ordering latency is all-or-nothing.**
  - Without generation numbers, `--topo-order` and `--date-order` (and `--graph`, which implies topo) set `revs->limited`, so git walks everything before printing.
  - With a commit-graph, git uses the incremental topo walk (`init_topo_walk`, git ≥ 2.20), which only explores down to the lowest generation among the tips. Old tags as tips drag that down to the root, which explains the 1.16 s for `--all`.
- **Full-table time is commit decoding, not walking.** `rev-list --parents` reads parents and dates straight from the commit-graph in 1.56 s. Anything that prints author or subject has to inflate all 1.48M commit objects, which takes ~9 s single-threaded, commit-graph or not.
- **Paging with `--skip` re-walks from the top every time**, so the cost grows with the offset.

### Filters

| filter | rows | none: first 100 / full | CG: first 100 / full | Bloom: first 100 / full |
|---|---:|---:|---:|---:|
| `--all --author=torvalds@…` | 43,297 | 25 ms / 9.35 s | 30 ms / 9.05 s | (n/a) |
| `--all --grep=io_uring` | 5,041 | 193 ms / 8.70 s | 182 ms / 8.36 s | (n/a) |
| `--all -i --grep=regression` | 15,169 | 58 ms / 8.81 s | 61 ms / 8.46 s | (n/a) |
| `HEAD -- kernel/sched/` (directory) | 5,763 | 16 ms / 1.06 s | 13 ms / 0.65 s | 11 ms / 0.32 s |
| `HEAD -- MAINTAINERS` (hot file) | 21,600 | 17 ms / 4.03 s | 12 ms / 2.01 s | 8 ms / 1.14 s |
| `HEAD -- kernel/sched/core.c` | 1,959 | 45 ms / 0.90 s | 31 ms / 0.53 s | 15 ms / 0.20 s |
| same, `--topo-order` | 1,959 | 885 ms / 0.95 s | 31 ms / 0.55 s | 15 ms / 0.20 s |
| `HEAD -- drivers/tty/n_null.c` (rare file) | 10 | 938 ms (all 10) / 0.94 s | 0.63 s | **0.13 s** |
| `HEAD --follow -- kernel/sched/core.c` | 2,919 | 694 ms / 12.0 s | 320 ms / 6.23 s | 316 ms / 6.19 s |
| `HEAD --author=torvalds@… -- kernel/` | 6,212 | 48 ms / 3.57 s | 46 ms / 3.58 s | 45 ms / 3.04 s |

What the table shows:

- **Author and message filters** are a full walk plus a decode, about 9 s however the repository is set up. Matches stream from the start, though, so the first screen of results is quick. Once the full table is loaded, Lenz can also filter author and subject itself, in memory, without git. Only full-message search needs `git log --grep`.
- **Path filters without `--follow`** use git's default history simplification: at a merge, git follows only a TREESAME parent, so whole side branches are skipped. That is why they beat a plain log. Bloom filters skip most tree diffs, about 98% "definitely not changed" according to GitHub's write-up, which gives a further 1.8–4.8× over a plain commit-graph.
- **The graph under a path filter needs `--parents`.** That flag turns on parent rewriting, so `%P` names the nearest ancestors that are in the output. Without it, `%P` names the real parents, which mostly aren't in the output. Verified on `n_null.c`.
- **`--follow`** turns off pruning (`revs->prune = 0`), so it walks the entire history and runs rename detection wherever the path appears. It never uses Bloom filters and only accepts a single file. In a partial clone the rename detection needs blob contents: the first `--follow` run here fetched a 115 MB promisor pack.
- **Costly format tokens to avoid:** `%G?`/`%GS` run gpg on each signed commit, and `%N` loads notes. `%d`/`%D` is cheap (+0.7 s over 1.48M rows, refs are loaded once), but taking refs from `for-each-ref` is cheaper still and also covers tags that aren't walk tips.

### gix 0.88 vs the git CLI (all refs as tips, 945 tips after peeling)

| walk | none: first 100 / full | CG: first 100 / full | git CLI equivalent (CG) |
|---|---:|---:|---:|
| `rev_walk` by commit time, ids only | 21 ms / 7.8 s | 21 ms / **0.89 s** | `rev-list --count --all` 0.92 s |
| same + decode author and subject | 21 ms / 8.9 s | 21 ms / 9.05 s | `log --all` 8.68 s |
| `traverse::commit::topo` TopoOrder, ids only | 16.2 s / 25.1 s | 1.6 s / 2.8 s | `rev-list --all --topo-order --parents` 1.15 s / 1.56 s |
| same + decode | 16.2 s / 26.3 s | 1.6 s / 10.8 s | `log --all --topo-order` 1.16 s / 8.94 s |
| `topo` DateOrder | 16.2 s / 25.4 s | 1.6 s / 2.8 s | `log --all --date-order` 1.15 s / 8.96 s |

Assessment of gix for this job:

- **Raw walking is a draw.** gix's topological sort is slower than git's: 1.4–2× with a commit-graph, 2.3× without.
  - Its commit-graph reader uses generation v1 (topological level). It doesn't read GDAT (corrected commit dates) or the Bloom chunks, and it can't write a commit-graph.
  - It has no path-limited log; gitoxide's own `log` bails on file lookups. It has no `--follow` outside gix-blame.
  - The API is still churning: 0.88's new `Exn` error type doesn't implement `std::error::Error`, so `?` into `Box<dyn Error>` breaks.
- **The one real advantage would be in-process parallel commit decoding.** The CLI gets the same result with several processes:

| `rev-list --topo-order --parents HEAD` (1.7 s), then decode the metadata with N parallel `git log --no-walk=unsorted --stdin` | total |
|---|---:|
| N = 1 | 11.43 s |
| N = 4 | 4.63 s |
| N = 8 | 3.30 s |
| N = 16 | **2.67 s** |

### Lane layout and memory (Rust probe over the real Linux stream)

The probe uses the "active lanes" algorithm, the one in git's graph.c, git-graph and lazygit:

1. Each row takes the lane reserved for its commit, or else the lowest free lane.
2. The first parent inherits that lane, unless the parent already has a lane, in which case the edge merges into it.
3. Every other parent that has no lane yet gets a new one.
4. The lowest free lane comes from a min-heap; a lane array indexed by node id gives O(1) lookup. The pass streams: row r needs only rows ≤ r.

The pass relies on every parent coming after all of its children. `--topo-order` and `--date-order` guarantee that; git's default commit-date order does not when clocks are skewed. That is also why option (c) under "Decisions for the spec" may reorder rows.

| | `--topo-order` | `--date-order` |
|---|---:|---:|
| lane assignment, all 1.48M rows | 36 ms | 38 ms |
| max active lanes / average per row | 214 / 90.6 | 577 / 236.1 |
| Σ active lanes over all rows (= size of per-row snapshots) | 134M | 350M |
| edges (1,604,512) spanning ≥ 30 rows / ≥ 1000 rows | 110,126 / 59,604 | 350,146 / 122,684 |
| edges crossing a row, uncut: max | 825 | 766 |
| edges crossing a row, cut at 30 rows (IntelliJ): max / avg / p99 | 534 / 0.6 / 4 | 535 / 1.2 / 5 |
| table (hash→index map, 20-byte ids, CSR parents, column, author id, ctime, subject arena) | 205 MB est., 138 B/commit | same |
| process peak RSS | 244–264 MB | 246–260 MB |
| graph only (no author or subject) | 107 MB est., 72 B/commit; 128–154 MB RSS | — |

- **Subjects** are the biggest part of the table: 80.6 MB, 54 B each.
- **The hash→index map** is the biggest optional part, at about 50 MB. It is needed only to resolve parents while loading; afterwards a sorted id array with binary search would do.
- **Authors** intern to 34,038 distinct names.
- **The cut maximum (534)** sits at the largest octopus merges; `2cde51fbd0f3` has 66 parents. Everywhere else, a cut graph is a handful of lanes wide.

## How other tools do it

- **IntelliJ `vcs-log`** (`platform/vcs-log/graph`):
  - Loading:
    - First block: `git log HEAD --branches --remotes --max-count=2000`, with no tags because tags "introduce great slowdown"; `vcs.log.recent.commits.count=1000` × 2.
    - Then all hashes, with `--date-order` including tags.
    - Format `%H %P %ct` (+ `%an %ae %d`) with control-character separators.
    - A refresh joins recent commits onto the existing graph (`VcsLogJoiner`).
  - In-memory graph:
    - `PermanentLinearGraphImpl`: rows in topological order, node index = row.
    - One bit per "simple" node (single parent on the next row); every other edge goes in `int[] longEdges` with CSR offsets.
    - Hashes become ints through an on-disk enumerator, so SHAs are never in memory.
    - `GraphLayoutBuilder` gives every node a `layoutIndex` by DFS from heads sorted by ref priority. Columns within a row are ordered by layoutIndex.
  - Rendering:
    - Print elements are generated per visible row (`PrintElementGeneratorImpl`, SLRU cache 100 rows).
    - `EdgesInRowGenerator` rebuilds the edges crossing a row by walking at most 1000 rows from a 40-row block boundary.
    - Long edges: `LONG_EDGE_SIZE = 30`, `LONG_EDGE_PART_SIZE = 1`, drawn as arrows; `VERY_LONG_EDGE_SIZE = 1000` / part 250 when "show long edges" is on.
  - Optional "IntelliSort" (Bek) re-sort.
  - Filters use a persistent index of paths, users and message trigrams, which is disabled by default in the current registry.
  - Its path filter adds `--full-history --simplify-merges`, which can never stream (`revs->limited`).
- **`git log --graph`** (graph.c):
  - Streaming, one `columns[]` array, a linear column search: O(lanes × parents) per commit.
  - `--graph` implies `--topo-order`.
  - Git 2.55 added `--graph-lane-limit=<n>`.
  - Measured above: 14.6 s for all of Linux with a commit-graph, vs 9 s for the same log without `--graph`.
- **gitk:** plain `git log --parents --pretty=raw` without `--topo-order`, reordered in Tcl. Layout is lazy for the visible rows (`drawcommits` → `layoutrows`/`optimize_rows`), with lines cut by `uparrowlen`/`downarrowlen = 5`, `mingaplen = 100`, `maxwidth = 16`.
- **git-graph** (Rust, now `git-bahn/git-graph` + `gleisbau`):
  - Full git2 revwalk (`TOPOLOGICAL | TIME`), a `HashMap<Oid,usize>`, and branch tracing along first parents. Nothing is lazy.
  - Issue #83: it runs out of memory on medium and large repos, and the maintainer suggests `-n`. Not a model to copy.
- **Sublime Merge:** "a custom high-performance Git reading library"; no public details of the graph.
- **lazygit:** `git log --topo-order -300` with `%H%x00%at%x00%aN%x00%ae%x00%P%x00%m%x00%D%x00%s`. Lane "pipes" are computed for every loaded commit and cached; only the visible range is rendered.
- **tig:** `--topo-order --parents --pretty=raw`, with an incremental graph (`graph-v2.c`).
- **vscode-git-graph:** `--max-count=300` then +100 at a time, with the graph recomputed over everything loaded.
- **gitui:** gix `rev_walk` by commit time with `use_commit_graph(false)`; it draws no graph.
- **Fork** (developer email quoted in git-graph #83, second-hand): a full traversal makes lazy loading impossible, and more than 10K tags are expensive.

## Recommendation

1. **Loader.**
   - Run one streaming process: `git log -z --topo-order --parents --format=%H %P%x1f%an%x1f%ae%x1f%ct%x1f%s HEAD --branches --remotes` (`-z` separates records with NUL).
   - Under a path filter, keep `--parents` so `%P` is rewritten.
   - Parse rows on a background thread into a compact table: a `u32` row id per commit, CSR parents, a `u16` column, an interned author, a timestamp, and a subject arena.
   - Lay out lanes as rows arrive.
   - Hand the UI growing row counts so the gpui list scrolls immediately and the scrollbar grows as rows load.
   - Refs come from `git for-each-ref --format='%(objectname) %(*objectname) %(refname)'` (`%(*objectname)` is the commit an annotated tag peels to). They are decorations, not walk tips, so tags cost nothing.
   - Tags that aren't on any branch are rare. If they matter, run a second `git log --tags --not --branches --remotes HEAD` and append its rows.
   - No `--skip`/`-n` paging.
   - Upgrade path, if 9 s to a fully searchable table is too slow: `rev-list --parents` for the graph (1.6 s) plus N parallel `git log --no-walk=unsorted --stdin` for the metadata (2.7 s total with N = 16).
2. **Graph.**
   - Use the streaming active-lanes assignment in topo order: 36 ms for Linux.
   - Store only each row's column and its parents.
   - Work out each visible row's print elements on demand:
     - For every edge crossing the row, rebuild the crossing-edge set from the nearest checkpoint. IntelliJ walks at most 1000 rows from a 40-row block. A per-K-row checkpoint of the lane array is the simpler alternative.
     - Cut edges of 30 or more rows into arrow stubs at both ends, which keeps the graph a few lanes wide.
   - Never materialise per-row lane snapshots for the whole history.
   - Colour by lane, or, as IntelliJ does, by the head the commit belongs to.
3. **Filters.**
   - Branch: rerun the loader with the selected refs.
   - Author and subject: filter the loaded table in memory. Free.
   - Full-message search: `git log --grep -i --fixed-strings`, streamed, about 9 s on Linux to finish.
   - Path: `git log --parents … -- <path>`, streamed, without `--follow`: 0.2–2 s on Linux, faster with Bloom filters.
   - `--follow`: only for one file's history (file history view). It can fetch blobs in partial clones.
   - Combine filters in one git call when a path is involved; otherwise filter in memory.
4. **gix:** don't adopt it for the log. The CLI is as fast or faster, has every filter, and keeps the existing shell-out design.

## Decisions for the spec

- **Missing commit-graph.** On a kernel-sized repository without a commit-graph, the first sorted row takes ~8.5 s. Options:
  - (a) Accept it, with a progress state. Typical repositories are far smaller: the cost scales with commit count, roughly 6 µs per commit.
  - (b) Write `.git/objects/info/commit-graph` (12 s for Linux, 85 MB; 74 s / 105 MB with Bloom filters). This is a cache that git itself writes during `gc` and maintenance, but it is still a write into `.git`, and map.md says Lenz is read-only.
  - (c) Show default-order rows at once (3 ms) while the topo-sorted load runs, then swap. That is more code, and the first view may reorder.
  - We can't decide this here; it is a product call. A hint ("run `git commit-graph write --reachable --changed-paths` for a faster log") is a cheap middle ground.
- **Which refs the log walks by default** (HEAD + branches + remotes, tags as decorations only) and whether tags that aren't on any branch appear.
- **Whether path filters follow renames** (only possible for a single file, 6–12 s on Linux).

## Sources

- IntelliJ: [GitLogProvider.kt](https://github.com/JetBrains/intellij-community/blob/master/plugins/git4idea/backend/src/log/GitLogProvider.kt), [VcsLogRefresherImpl.kt](https://github.com/JetBrains/intellij-community/blob/master/platform/vcs-log/impl/src/com/intellij/vcs/log/data/VcsLogRefresherImpl.kt), [PermanentLinearGraphImpl.java](https://github.com/JetBrains/intellij-community/tree/master/platform/vcs-log/graph/src/com/intellij/vcs/log/graph/impl/permanent), [GraphLayoutBuilder.kt](https://github.com/JetBrains/intellij-community/tree/master/platform/vcs-log/graph/src/com/intellij/vcs/log/graph/impl/permanent), [PrintElementGeneratorImpl.kt / EdgesInRowGenerator.java](https://github.com/JetBrains/intellij-community/tree/master/platform/vcs-log/graph/src/com/intellij/vcs/log/graph/impl/print), [GitLogParser.java](https://github.com/JetBrains/intellij-community/blob/master/plugins/git4idea/backend/src/history/GitLogParser.java), [registry.properties](https://github.com/JetBrains/intellij-community/blob/master/platform/util/resources/misc/registry.properties).
- git: [revision.c](https://github.com/git/git/blob/master/revision.c) (`limited`, `init_topo_walk`, Bloom use in `rev_compare_tree`, `follow_renames` disabling prune), [graph.c](https://github.com/git/git/blob/master/graph.c), [rev-list-options](https://github.com/git/git/blob/master/Documentation/rev-list-options.adoc) (`--graph` implies `--topo-order`), [git-commit-graph docs](https://git-scm.com/docs/git-commit-graph), [gitk](https://github.com/git/git/blob/master/gitk-git/gitk).
- Commit-graph write-ups:
  - Microsoft, [Supercharging the Git Commit Graph](https://devblogs.microsoft.com/devops/supercharging-the-git-commit-graph/), parts [II](https://devblogs.microsoft.com/devops/supercharging-the-git-commit-graph-ii-file-format/) and [III](https://devblogs.microsoft.com/devops/supercharging-the-git-commit-graph-iii-generations/), and [Updates to the commit-graph feature](https://devblogs.microsoft.com/devops/updates-to-the-git-commit-graph-feature/) (`log --graph -10` on Linux: 6.1 s → 0.009 s).
  - GitHub, [Git's database internals II: commit history queries](https://github.blog/open-source/git/gits-database-internals-ii-commit-history-queries/) (Linux v5.19: `rev-list --topo-order -n 100` 6.88 s → 0.02 s).
  - GitHub, [III: file history queries](https://github.blog/open-source/git/gits-database-internals-iii-file-history-queries/) (Bloom: `git log -- <file>` 1.03 → 0.18 s).
- gitoxide: [gix-traverse topo](https://github.com/GitoxideLabs/gitoxide/tree/main/gix-traverse/src/commit/topo), [gix-commitgraph](https://github.com/GitoxideLabs/gitoxide/tree/main/gix-commitgraph), [crate-status.md](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md), [STABILITY.md](https://github.com/GitoxideLabs/gitoxide/blob/main/STABILITY.md), [discussion #76](https://github.com/GitoxideLabs/gitoxide/discussions/76) (2021 Linux walk: gix 6.4 s vs libgit2 14.1 s).
- Others:
  - [git-graph](https://github.com/git-bahn/git-graph) and [issue #83](https://github.com/mlange-42/git-graph/issues/83).
  - [lazygit graph.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/gui/presentation/graph/graph.go) and [commit_loader.go](https://github.com/jesseduffield/lazygit/blob/master/pkg/commands/git_commands/commit_loader.go).
  - [tig graph-v2.c](https://github.com/jonas/tig/blob/master/src/graph-v2.c).
  - [vscode-git-graph dataSource.ts](https://github.com/mhutchie/vscode-git-graph/blob/develop/src/dataSource.ts).
  - [gitui logwalker.rs](https://github.com/gitui-org/gitui/blob/master/asyncgit/src/sync/logwalker.rs).
  - [Sublime Merge announcement](https://www.sublimetext.com/blog/articles/sublime-merge).
- Unverified: Sublime Merge's and GitKraken's graph internals; IntelliJ's default sort mode in the UI; any published gix-vs-git-CLI benchmark other than the ones measured here.
