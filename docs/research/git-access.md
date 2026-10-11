# Git access layer: `gix` vs `git2` vs shelling out to `git`

Ticket: `.scratch/diff-viewer-mvp/issues/03-git-access-research.md`. Researched 2026-10-01.
Terms (Source, Side, Comparison, Change Set) are defined in `CONTEXT.md`.

## Recommendation

**Shell out to the `git` CLI for the MVP. Do not use `git2`.** Keep every git call in one module. If spawning processes later costs too much, or a pure-Rust single binary becomes a requirement, swap in **`gix`**.

Why:

1. **It matches git exactly, by definition.** The viewer has to show what `git diff` shows: textconv, smudge/clean filter drivers (for example LFS), `eol`/`autocrlf`, git's binary heuristic, and rename pairing. Only git itself guarantees all of that. `gix` documents deviations from git's rename heuristics ([crate-status L110](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L110), [L371, L387](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L371)). `libgit2` has no textconv support: the feature request has been open since 2013 ([libgit2#1328](https://github.com/libgit2/libgit2/issues/1328)).
2. **Big repos.** git can use `core.fsmonitor` and `core.untrackedCache` ([core.adoc L64, L125](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/core.adoc?plain=1#L64)). gitoxide lists these accelerators as not done ([crate-status L58](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L58)). libgit2's public `index.h`, `status.h` and `diff.h` never mention fsmonitor or the untracked cache (a grep of those headers at commit `0551dfd` finds nothing).
3. **Build story.** It needs no C toolchain and adds no crate churn. `gix` is pre-1.0 and ships a breaking minor release about once a month (see [API stability](#api-stability)).
4. **The runtime cost is acceptable for this product.** The viewer needs `git` on `PATH`. Anyone who runs `git difftool` already has it. Plain-file Comparisons need no git at all.

When to revisit: if launch-to-first-paint on a large Change Set is dominated by process spawns, even with one long-lived `git cat-file --batch`. In that case, move the **content reads** to `gix` first (`find_blob` plus `diff_resource_cache` for filters and textconv). Keep **listing** on `git` until gitoxide ships fsmonitor and untracked-cache support.

## Comparison table

| Need | `git` CLI | `gix` 0.88.0 | `git2` 0.21.0 (libgit2 1.9.x) |
|---|---|---|---|
| Content at a revision | `git cat-file --batch`, `<rev>:<path>` ([cat-file](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-cat-file.adoc?plain=1#L109)) | `Repository::find_blob` ([docs.rs](https://docs.rs/gix/latest/gix/struct.Repository.html#method.find_blob)) | `Repository::find_blob`, `Blob::content` ([docs.rs](https://docs.rs/git2/latest/git2/struct.Blob.html#method.content)) |
| Index content | `:<path>` (stage 0), same batch process ([cat-file L73-82](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-cat-file.adoc?plain=1#L73)) | `Repository::index()` then `find_blob` ([docs.rs](https://docs.rs/gix/latest/gix/struct.Repository.html#method.index)) | index entry, then `find_blob` |
| Working-tree content | read the file directly | read the file directly | read the file directly |
| Filters applied to blob content (eol, smudge, `ident`) | `cat-file --filters` ([L79](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-cat-file.adoc?plain=1#L79)) | yes: to-worktree/to-git conversions in the diff resource cache ([crate-status L379](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L379)); driver processes ([L799-800](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L799)) | C: `git_blob_filter` ([blob.h L227](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/blob.h#L227)), but **not bound** in `libgit2-sys` and not in `git2`; only `crlf` and `ident` are built in, and driver filters must be registered by the app ([sys/filter.h L379](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/sys/filter.h#L379)) |
| textconv | `cat-file --textconv` ([L73](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-cat-file.adoc?plain=1#L73)) | yes ([crate-status L107, L380](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L107)) | **no** ([#1328](https://github.com/libgit2/libgit2/issues/1328), [#6126](https://github.com/libgit2/libgit2/issues/6126)) |
| Binary detection | `--numstat` prints `-\t-` for binary files ([diff-options L231](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/diff-options.adoc?plain=1#L231)); the heuristic is "NUL in the first 8000 bytes" ([xdiff-interface.c L197](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/xdiff-interface.c#L197)) plus `diff`/`binary` attributes | first 8k header bytes plus attributes ([crate-status L382](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L382)) | `Blob::is_binary` ([docs.rs](https://docs.rs/git2/latest/git2/struct.Blob.html#method.is_binary)); `GIT_DIFF_FORCE_TEXT/BINARY` |
| rev vs rev | `git diff --raw -z -M A B` | `diff_tree_to_tree` ([docs.rs](https://docs.rs/gix/latest/gix/struct.Repository.html#method.diff_tree_to_tree)) | `diff_tree_to_tree` ([diff.h L884](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/diff.h#L884)) |
| `A...B` (merge base) | native: `= git diff $(merge-base A B) B` ([git-diff L102](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-diff.adoc?plain=1#L102)) | `merge_base`, then tree diff ([docs.rs](https://docs.rs/gix/latest/gix/struct.Repository.html#method.merge_base)) | `merge_base`, then tree diff ([docs.rs](https://docs.rs/git2/latest/git2/struct.Repository.html#method.merge_base)) |
| index vs HEAD (staged) | `git diff --cached --raw -z -M` | `tree_index_status` with rename tracking ([docs.rs](https://docs.rs/gix/latest/gix/struct.Repository.html#method.tree_index_status)) | `diff_tree_to_index` ([diff.h L911](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/diff.h#L911)) |
| worktree vs index | `git diff --raw -z -M` | `status()…into_index_worktree_iter` with renames ([crate-status L121-122](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L121)) | `diff_index_to_workdir` ([L939](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/diff.h#L939)) |
| worktree vs HEAD or any rev | `git diff --raw -z -M <rev>` | no single call; see [gix findings](#gix-gitoxide) | `diff_tree_to_workdir_with_index` ([L989](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/diff.h#L989)) |
| Untracked files | `git status --porcelain=v2 -z` ([L294](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-status.adoc?plain=1#L294)) | dirwalk inside status ([crate-status gix-status](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L775)) | `statuses()` |
| Rename detection | git's own heuristic, `diff.renames`/`-M` ([config/diff L157](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/diff.adoc?plain=1#L157)) | yes, but heuristics deviate from git ([L110](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L110)) | `git_diff_find_similar` ([L1045](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/diff.h#L1045)); status flags `RENAMES_HEAD_TO_INDEX/INDEX_TO_WORKDIR` ([status.h L154](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/status.h#L154)) |
| fsmonitor / untracked cache | yes ([core.adoc](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/core.adoc?plain=1#L64)) | no ([L58](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L58)) | no public API (headers grep) |
| C toolchain at build time | none | none: "No C toolchain is involved" ([gix/Cargo.toml L259](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/Cargo.toml#L259)) | yes: `cc`-built vendored libgit2 plus `libz-sys` ([libgit2-sys/Cargo.toml](https://github.com/rust-lang/git2-rs/blob/main/libgit2-sys/Cargo.toml)) |
| Runtime dependency | `git` on `PATH` | none | none if vendored (statically linked) |
| API stability | git plumbing output formats are stable | 0.x, about one breaking release per month | 0.x but slow-moving |

## Per-option findings

### `git` CLI

**Listing a Change Set.** `git diff --raw -z --no-abbrev -M` prints one record per changed path: old/new mode, old/new object id, status letter (with a score for `R`/`C`), and one or two paths ([diff-format.adoc L1-31](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/diff-format.adoc?plain=1#L1)). This is exactly the record `git difftool --dir-diff` itself parses ([builtin/difftool.c L445-466](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L445), invoked at [L816-817](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L816)), so one parser serves both paths.

| Source pairing | Command |
|---|---|
| rev vs rev | `git diff --raw -z --no-abbrev -M A B` |
| merge base `A...B` | `git diff --raw -z --no-abbrev -M A...B`, which equals `git diff $(git merge-base A B) B` ([git-diff L98-104](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-diff.adoc?plain=1#L98)) |
| index vs HEAD | `git diff --cached --raw -z --no-abbrev -M` ([L47-55](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-diff.adoc?plain=1#L47)) |
| worktree vs index | `git diff --raw -z --no-abbrev -M` ([L26-32](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-diff.adoc?plain=1#L26)) |
| worktree vs rev | `git diff --raw -z --no-abbrev -M <rev>` ([L61-71](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-diff.adoc?plain=1#L61)) |
| plain files/dirs | `git diff --no-index` ([L34-45](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-diff.adoc?plain=1#L34)). The viewer can also skip git here. |

- **Always pass `-M` explicitly.** `diff.renames` defaults to `true`, but only for porcelain `git diff`/`git log`, not for plumbing such as `diff-files` ([config/diff L157-164](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/diff.adoc?plain=1#L157)). `diff.renameLimit` defaults to 1000 ([L151-155](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/diff.adoc?plain=1#L151)).
- **Untracked files** are not in `git diff`. Use `git status --porcelain=v2 -z`, which also reports index and worktree renames ([git-status L152, L294](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-status.adoc?plain=1#L294)).
- **Binary flag**: `--raw` does not carry it. Either add `--numstat` (binary shows `-` and `-`, [diff-options L231-236](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/diff-options.adoc?plain=1#L231)) or apply git's heuristic yourself: binary if a NUL byte appears in the first 8000 bytes ([xdiff-interface.c L197-203](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/xdiff-interface.c#L197)). Attributes can override it: `-diff`/`binary` ([gitattributes "Marking files as binary" L1031](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/gitattributes.adoc?plain=1#L1031)).

**Reading content.** Keep one long-lived `git cat-file --batch` (or `--batch-command`) per repo and feed it object names on stdin ([cat-file L109, L161-170](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-cat-file.adoc?plain=1#L109)). That avoids a spawn per Side. `<rev>:<path>` gives revision content and `:<path>` gives index content. `--filters` returns the content as checkout would write it (smudge, eol), and `--textconv` returns the textconv output. Both require the `<tree-ish>:<path>` / `:<path>` form so git can look up attributes ([L73-82](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-cat-file.adoc?plain=1#L73)). Working-tree content is read straight from disk.

**Attributes and eol.** `text`, `eol`, `core.autocrlf`, `filter` and `diff`/textconv are all defined by git ([gitattributes L135, L174, L406, L728, L942](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/gitattributes.adoc?plain=1#L135)). With the CLI, the viewer only has to choose which form to display: raw blob, `--filters`, or `--textconv`.

**Performance.** git uses `core.fsmonitor` (the built-in daemon "can speed up Git commands that need to refresh the Git index (e.g. `git status`) in a working directory with many files") and `core.untrackedCache` ([core.adoc L64-72, L125-133](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/core.adoc?plain=1#L64)). The cost is one process spawn per listing plus one long-lived cat-file process. I took no measurements; that is follow-up work for the implementation ticket.

**Build.** Nothing is linked. The viewer must find `git` at runtime and fail with a clear message if it is missing (plain-file mode still works).

### `gix` (gitoxide)

Version: **gix 0.88.0, published 2026-09-25** ([crates.io](https://crates.io/crates/gix); [docs.rs](https://docs.rs/gix/0.88.0/gix/)).

Feature completeness for the viewer:

- **Tree vs tree** with rename and copy tracking (identity and similarity), textconv, `diff` attributes, and correct binary handling: all checked in [crate-status L103-120](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L103). API: `Repository::diff_tree_to_tree` (added in 0.67 "for greater similarity to `git2`", [CHANGELOG L2067](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/CHANGELOG.md?plain=1#L2067)).
- **Tree or index vs working tree** with rename tracking and recursive submodule status: checked ([L121-123](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L121)). `gix-status` also checks index-vs-worktree and index-vs-index with rename tracking and untracked files ([L775-782](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L775)).
- **Tree vs index (staged).** crate-status still shows this unchecked ([L125-127](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L125)), but that line is **stale**:
  - The file itself says "Generated by Codex … Reconcile on demand" ([L38](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L38)).
  - `Repository::tree_index_status()` shipped in 0.70.0 ([CHANGELOG L1697](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/CHANGELOG.md?plain=1#L1697)).
  - `is_dirty()` uses it ([status/mod.rs L156](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/src/status/mod.rs#L156)).
  - The status platform exposes `tree_index_track_renames` ([status/platform.rs L123](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/src/status/platform.rs#L123)).
- **Worktree vs an arbitrary rev** has no single call. Two compositions work:
  - Set `status().head_tree(rev_tree)` ([platform.rs L116](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/src/status/platform.rs#L116)) and merge the tree→index and index→worktree streams yourself.
  - Or pass an in-memory index built with `index_from_tree` to `status().index(..)` ([L81](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/src/status/platform.rs#L81); [docs.rs](https://docs.rs/gix/latest/gix/struct.Repository.html#method.index_from_tree)). *Inference:* an index built from a tree carries no stat data, so every tracked file would be re-hashed. That is slow on big repos.
- **Filters**:
  - The diff resource cache offers to-worktree, to-git and to-worktree-if-needed conversions, textconv, a big-file threshold and 8k-header binary detection ([L378-384](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L378)).
  - Worktree conversions cover `eol`, `working-tree-encoding`, `text` and `ident`, plus filter processes and single-invocation clean/smudge drivers ([L792-800](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L792)).
  - The unchecked `gix-filter` "base" items (L679-681) are for *writing* filter programs, not running them.
- **Merge base**: `merge_base`, `merge_bases_many` and octopus are available ([docs.rs](https://docs.rs/gix/latest/gix/struct.Repository.html#method.merge_base); [crate-status L812-816](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L812)). Rev-spec ranges parse ([L816](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L816)).
- **Known deviations**:
  - Rename tracking uses "the first-found candidate that matches the similarity percentage", where git keeps up to four candidates, "this can lead to different sources being found" ([L110-111](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L110)).
  - "heuristics to find best candidate", "find by basename" and "diff-heuristics match Git perfectly" are all unchecked ([L371-375, L387](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L371)).
  - For a viewer whose Change Set should match `git diff`, this means rename pairings may occasionally differ.

**Performance.**
- gix runs the index-modification check and the directory walk in parallel, which it calls "ultimately … much faster" than git's sequential order ([status/mod.rs L84-89](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/src/status/mod.rs#L84)).
- Big-repo accelerators (commit-graph, bitmaps, split-index, sparse-index, fsmonitor, untracked-cache) are unchecked as a workflow ([L58](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/crate-status.md?plain=1#L58)).
- Split-index `link` extensions are read but dissolved on write ([SHORTCOMINGS.md](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/SHORTCOMINGS.md)).

**Build.** Pure Rust: "No C toolchain is involved" ([gix/Cargo.toml L259](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/Cargo.toml#L259)), and it "always uses zlib-rs" ([L262](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/Cargo.toml#L262)). The default features pull in a lot ([L32](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/Cargo.toml#L32)). The manifest recommends picking only the components you need to cut compile time; for the viewer that is `status`, `blob-diff`, `revision` and `attributes`.

<a id="api-stability"></a>**API stability.**
- The README lists `gix` under "Initial Development" ("usable … possibly incomplete functionality"); only `gix-lock` and `gix-tempfile` are "Production Grade" ([README L72-100](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/README.md?plain=1#L72)).
- Release cadence:
  - The changelog headings run 0.80 (2026-02), 0.81 (03), 0.82/0.83 (04), 0.84 (05), 0.85 (06) ([CHANGELOG L253-784](https://github.com/GitoxideLabs/gitoxide/blob/f819565c2c4c56619c4888acef6cf3b8144cbccb/gix/CHANGELOG.md?plain=1#L253)), and crates.io is at 0.88.0 (Sep 2026).
  - Under 0.x semver, each minor release is breaking.

### `git2` (libgit2 bindings)

Versions: **git2 0.21.0** (2026-05-18) and **libgit2-sys 0.18.8+1.9.7** (2026-08-21) ([crates.io/git2](https://crates.io/crates/git2), [crates.io/libgit2-sys](https://crates.io/crates/libgit2-sys)).

- **Diff surface is complete**: tree↔tree, tree↔index, index↔workdir, tree↔workdir-with-index (which emulates `git diff <rev>`), and index↔index ([diff.h L884-1008](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/diff.h#L884)). Renames come from `git_diff_find_similar` ([L1045](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/diff.h#L1045)) and status rename flags ([status.h L154-161](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/status.h#L154)). All of these are bound in Rust ([Repository](https://docs.rs/git2/latest/git2/struct.Repository.html#method.diff_tree_to_workdir_with_index), [Diff::find_similar](https://docs.rs/git2/latest/git2/struct.Diff.html#method.find_similar)).
- **textconv: not supported.** [libgit2#1328 "Textconv support"](https://github.com/libgit2/libgit2/issues/1328) has been open since 2013-02-09. [#6126](https://github.com/libgit2/libgit2/issues/6126) reports that textconv'd files show as "Binary files differ".
- **Filters**:
  - Only `crlf` and `ident` are preregistered ([sys/filter.h L379-381](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/sys/filter.h#L379)).
  - A `filter=<driver>` such as LFS needs an app-registered filter, and the registry is not thread-safe (L383-385).
  - `git_blob_filter` exists in C ([blob.h L227](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/include/git2/blob.h#L227)) but is not declared in `libgit2-sys/lib.rs` and has no safe wrapper. For content, `git2::Blob` offers only `content`, `size` and `is_binary` ([docs.rs](https://docs.rs/git2/latest/git2/struct.Blob.html)). Showing a revision Side in worktree form (eol, smudge) would mean writing that conversion yourself.
- **Threading**: "libgit2 objects cannot be safely accessed by multiple threads simultaneously"; only `git_odb` is internally locked ([docs/threading.md](https://github.com/libgit2/libgit2/blob/0551dfd4ad989b6a3d5683c0d4cf326c6efef929/docs/threading.md)).
- **Big repos**: no fsmonitor or untracked-cache API in `index.h`, `status.h` or `diff.h` (grep at `0551dfd`).
- **Build**:
  - libgit2 1.9.6+ is vendored and **statically linked** unless `LIBGIT2_NO_VENDOR=1` is set or a compatible system libgit2 is found ([git2-rs README L30-41](https://github.com/rust-lang/git2-rs/blob/main/README.md?plain=1#L30); probe range `1.9.6..1.10.0`, [build.rs L12](https://github.com/rust-lang/git2-rs/blob/main/libgit2-sys/build.rs#L12)).
  - The vendored build compiles C with the `cc` crate, not CMake ([build.rs L131](https://github.com/rust-lang/git2-rs/blob/main/libgit2-sys/build.rs#L131)), and links zlib via `libz-sys` ([libgit2-sys/Cargo.toml L27-31](https://github.com/rust-lang/git2-rs/blob/main/libgit2-sys/Cargo.toml#L27)). It links the `Security` framework on Apple targets ([build.rs L323](https://github.com/rust-lang/git2-rs/blob/main/libgit2-sys/build.rs#L323)).
  - Default features are empty, so a local-only viewer avoids OpenSSL/libssh2 ([git2 Cargo.toml L55-60](https://github.com/rust-lang/git2-rs/blob/main/Cargo.toml#L55)). `ssh` on macOS needs OpenSSL ([README L61-63](https://github.com/rust-lang/git2-rs/blob/main/README.md?plain=1#L61)).
  - Summary: a C compiler is needed on both CI targets, and the resulting binary has no runtime git dependency.

**Verdict on git2**: it brings a C toolchain and still can't do textconv or driver filters, so it is strictly worse than the CLI on correctness and worse than `gix` on build.

## `git difftool` integration

### How git hands files over

**File mode (default, one launch per changed path).**
- `git difftool` runs `git diff` with `GIT_EXTERNAL_DIFF=git-difftool--helper` and `GIT_PAGER=` ([builtin/difftool.c L701-714](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L701)).
- `git diff` calls the helper with 7 args: `path old-file old-hex old-mode new-file new-hex new-mode` ([git.adoc L648-668](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git.adoc?plain=1#L648)).
- The helper loops `launch_merge_tool "$1" "$2" "$5"`, `shift 7` ([git-difftool--helper.sh L105-122](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-difftool--helper.sh#L105)) and sets:
  - `MERGED="$1"`: the path as it appears in the working tree, repo-relative ([L34](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-difftool--helper.sh#L34))
  - `LOCAL="$2"`: the pre-image
  - `REMOTE="$3"` (the post-image, i.e. arg 5)
  - `BASE="$1"`: same as `MERGED` ([L37](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-difftool--helper.sh#L37); [git-difftool docs L66-75](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-difftool.adoc?plain=1#L66); [git-scm.com/docs/git-difftool](https://git-scm.com/docs/git-difftool))

**What `$LOCAL` / `$REMOTE` actually are** ([diff.c `prepare_temp_file`](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L4677)):
- **A temp file** `$TMPDIR/git-blob-XXXXXX/<basename>`, created by `mks_tempfile_dt("git-blob-XXXXXX", base)` ([L4659](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L4659)). Its content has already passed through `convert_to_working_tree`, so eol and smudge are applied ([L4662](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L4662)). The basename is kept, so the extension works for syntax detection.
- **The real working-tree file**, when git can borrow it ([L4715](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L4715)). This is a **repo-relative path**, resolved against the command's cwd, which is the top of the working tree (*inference*: git `lstat()`s the same relative path at L4697).
- **`/dev/null`** for the missing side of an add or delete ([L4687](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L4687)).
- Temp files "are removed when `GIT_EXTERNAL_DIFF` exits" ([git.adoc L664-668](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git.adoc?plain=1#L664)).

**Directory mode (`-d` / `--dir-diff`, one launch for the whole Change Set).**
- Builds `$TMPDIR/git-difftool.XXXXXX/left/` and `…/right/` ([difftool.c L404-419](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L404)) from `git diff --raw --no-abbrev -z` ([L816-817](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L816)).
- **Only changed paths are materialized.** For renames, `left/` gets the old path and `right/` gets the new path ([L455-466](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L455)), so rename pairing is lost and the viewer must re-detect it.
- `left/` (and non-worktree `right/` files) are checked out through `checkout_path`, so filters are applied ([L497-502](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L497)).
- When the right-hand content equals the working-tree file, `right/<path>` is a **symlink to the working-tree file** by default. `--no-symlinks` makes it a copy, and copies are the default on Windows ([L518-560](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L518); [docs L80-88](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-difftool.adoc?plain=1#L80)).
- After the tool exits, modified regular-file copies are copied back into the working tree, and git warns if both sides changed ([L612-668](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L612)).
- Repo symlinks are written as regular files containing the link target ([L289-330](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L289)). Submodules are written as files containing `Subproject commit <sha>[-dirty]` ([L472-483](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L472)).
- The tool runs as `difftool--helper <left> <right>` with `GIT_DIFFTOOL_DIRDIFF=true`. With `-x`, it runs `<extcmd> <left> <right>` directly ([L599-607](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L599)). *Inference:* `cmd` here has no `use_shell`, so in dir mode `-x` should name a bare executable with no arguments.
- The helper sets only `LOCAL=$1` and `REMOTE=$2`, so **`$MERGED` is empty in dir mode** ([helper L84-87](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-difftool--helper.sh#L84)).
- The temp dir is deleted with `remove_dir_recursively` once the tool returns ([L672](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L672)).
- `--dir-diff` and `--no-index` cannot be combined ([L774](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L774)).

**How the command string is run.**
- `difftool.<tool>.cmd` falls back to `mergetool.<tool>.cmd` ([git-mergetool--lib.sh L283-290](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-mergetool--lib.sh#L283)). It is run as `( eval $merge_tool_cmd )` in a subshell ([L171-177](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-mergetool--lib.sh#L171)).
- So `$LOCAL`, `$REMOTE`, `$MERGED` and `$BASE` are **shell variables expanded in the command string, not exported env**. The cmd must quote them.
- `-x/--extcmd` in file mode runs `eval $GIT_DIFFTOOL_EXTCMD "$LOCAL" "$REMOTE"` and does `export BASE` ([helper L60-62](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-difftool--helper.sh#L60); [docs](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-difftool.adoc?plain=1#L90)).
- Env that *is* exported:
  - `GIT_DIFF_PATH_COUNTER` and `GIT_DIFF_PATH_TOTAL` ([diff.c L4792-4794](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L4792); [git.adoc L686-690](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git.adoc?plain=1#L686))
  - `GIT_PREFIX`, the user's original subdirectory, defaulting to `.` ([mergetool--lib L313-316](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-mergetool--lib.sh#L313))
  - `GIT_DIR` and `GIT_WORK_TREE`, absolute ([difftool.c L771-772](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/builtin/difftool.c#L771))
- **Exit codes**: 126, 127 and ≥128 abort the whole loop. Any other non-zero only matters with `--trust-exit-code` / `difftool.trustExitCode` ([helper L88-122](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/git-difftool--helper.sh#L88); [docs L109-116](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-difftool.adoc?plain=1#L109)).
- **Prompting** per file is the default; `-y` or `difftool.prompt=false` disables it. `-d` never prompts ([docs L22-31](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/git-difftool.adoc?plain=1#L22); [config/difftool.adoc](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/difftool.adoc)).
- **Tool selection**: `-t`, else `diff.tool`. `-g` selects `diff.guitool`, and `difftool.guiDefault=auto` picks it when `DISPLAY` is set ([config/difftool.adoc](https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/difftool.adoc)).

### What the viewer's CLI must accept

1. **`viewer <left> <right>`**: two positional paths that are **both files or both directories**. Directories mean a Change Set: pair files by relative path and run rename detection for unpaired ones.
2. **`/dev/null` or a missing counterpart** means an empty Side (added or deleted file).
3. **An optional display name**, e.g. `--name "$MERGED"`, used for the title and syntax detection. Temp files are named `git-blob-XXXXXX/<basename>`, so without it only the basename is known. It **must tolerate an empty value**, because `$MERGED` is empty in dir-diff mode.
4. **Block until the window closes.** File-mode temp files are deleted when the command exits, and the dir-diff tree is deleted right after the tool returns. A single-instance app that hands off to a running process and exits at once will show vanished files. If the app is single-instance, the CLI must wait for "this Comparison closed" before exiting.
5. **Exit 0 on normal close.** Never exit 126, 127 or ≥128 except on a real failure: those abort the whole difftool loop.
6. **Resolve relative paths against the cwd**: `$REMOTE` may be a repo-relative working-tree path. Read `GIT_PREFIX` to show paths relative to where the user ran the command. Optionally show "3/12" from `GIT_DIFF_PATH_COUNTER/TOTAL`.
7. **Follow symlinks in `right/`.** Edits saved through them land in the working tree. That is the point of difftool's symlink mode, so the viewer must not replace the symlink with a regular file when saving.
8. **Don't rely on env for `LOCAL`/`REMOTE`**: take them from argv.

User config we can document:

```ini
[diff]
    tool = viewer
[difftool "viewer"]
    cmd = viewer --name \"$MERGED\" \"$LOCAL\" \"$REMOTE\"
[difftool]
    prompt = false
```

Usage: `git difftool` (per file) or `git difftool -d A...B` (one window per Change Set). `git difftool -x viewer` also works in file mode (`viewer LOCAL REMOTE`) and dir mode (`viewer LEFTDIR RIGHTDIR`).

## Not verified / open

- No benchmarks were run: spawn latency vs `gix` in-process reads on a large repo is untested.
- The minimum git version the CLI commands need was not pinned. `cat-file --batch-command` is the newest feature used; a fallback is plain `--batch`.
- How git behaves on macOS when only the Xcode CLT `git` shim is present was not researched.
