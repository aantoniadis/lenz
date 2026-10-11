# Large, binary and non-UTF-8 inputs

Research for ticket `14-large-binary-encoding-research`. Terms follow `/CONTEXT.md` (Comparison, Side, Change Block, Inline Change). This builds on [`intellij-diff-refinement.md`][prior] (branch `research/intellij-diff-refinement`), which already covers the IntelliJ comparison pipeline, the `MyersLCS` cost cap and the `MAX_BAD_LINES` rule. Those are referenced here, not repeated.

Sources read on 2026-10-01:

- IntelliJ Community at [`d846a4b`][ij] (2026-10-01).
- git at [`a018953`][git] (2026-09-27), plus local `git version 2.55.0` for the experiments.
- `gix-imara-diff` 0.3.0, `encoding_rs` 0.8.42 and `chardetng` 1.0.0, read from their published crate sources.

## Summary

### IntelliJ: thresholds and behaviour

| Situation | What IntelliJ does | Source |
|---|---|---|
| Line LCS too expensive | Myers stops at D > `max(20000 + 10·√(N+M), 20000)` and falls back to patience. If patience doesn't halve the problem, `DiffTooBigException` is thrown. The viewer then shows both Sides **without highlighting** under a banner: *"Unable to calculate diff. File is too big and there are too many changes."* | [Diff.kt][ij-diffkt], [SimpleDiffViewer][ij-sdv-catch], [DiffNotifications][ij-notif-toobig], [DiffBundle][ij-bundle-toobig] |
| Word refinement too expensive | That Change Block is shown line-only. After 3 such failures (`MAX_BAD_LINES`), every later block is left unrefined. No banner. | [ComparisonManagerImpl][ij-cmi-bad], [DiffConfig][ij-diffconfig] |
| Local file > content-load limit | **20 MB** by default (`idea.max.content.load.filesize`, in KB). Only a read-only preview of **2.5 MB** loads (`idea.max.content.load.large.preview.size`), with the banner *"File is too large. Only preview is loaded."* The "Contents are identical" notice is suppressed. A binary file over the limit gets no document at all. | [FileUtilRt][ij-fur-limits], [FileSizeLimit][ij-fsl], [FileDocumentManagerBase][ij-fdmb-load], [DiffUtil][ij-du-preview], [TextDiffViewerUtil][ij-tdvu-equal] |
| VCS revision > limit | `idea.max.vcs.loaded.size.kb`, default = the content-load limit (20 MB). Fails with *"Cannot show the contents of '…'. The file is bigger than …"*. | [VcsUtil][ij-vcsutil], [ContentRevisionCache][ij-crc], [VcsBundle][ij-vcsbundle] |
| Binary content | Decided by **file type** first (`fileType.isBinary()`). Content sniffing runs only for unknown or native types. There, binary = (high-bit bytes that aren't valid UTF-8) OR (any byte < 9). As a result, a BOM-less Latin-1 file of unknown type is shown as **binary**, where git shows it as text. Binary Comparisons go to the binary viewer, which compares bytes: *"Files contents are identical / different"*. Over the content-load limit with equal lengths: *"Files are too large to compare"*. | [DiffContentFactoryImpl][ij-dcf-binary], [CharsetToolkit][ij-ct-guess], [TwosideBinaryDiffViewer][ij-tbdv] |
| Huge single lines (minified JS) | No special case: no line-length limit in `diff-impl`, `util/diff` or the registry. Only the cost cap applies. | grep, see [Huge single lines](#huge-single-lines) |
| Encoding | Order: BOM (UTF-8/16/32), then the file type's own charset, then "valid UTF-8 → UTF-8", then the configured charset if it round-trips. Decoding errors show a red banner: *"Content was decoded with errors (using 'X' charset)"*. When the Sides' charset or BOM differ, each Side's title shows the charset (`UTF-8 BOM`). If the texts are equal, the notice reads *"Contents have differences only in encoding"*. | [LoadTextUtil][ij-ltu-guess], [DiffContentFactoryImpl][ij-dcf-charset], [TextContent.fromBytes][ij-dcf-textcontent], [DiffUtil][ij-du-title], [DiffNotifications][ij-notif-equal] |

### git: thresholds and behaviour

| Item | Behaviour | Source |
|---|---|---|
| Binary heuristic | `buffer_is_binary`: a NUL in the first **8000** bytes. Nothing else matters for diff: Latin-1 bytes pass through untouched. Verified: a NUL at byte 7991 → `Bin`, at byte 8001 → text. | [xdiff-interface.c][git-binary], [experiment](#git-experiments) |
| `diff` attribute | Set: always text, even with NUL. Unset (`-diff`): "Binary files differ". Unspecified: content check + size check. String: a named driver, which can carry `textconv` and `binary = true`. | [gitattributes][git-attr-diff] |
| `binary` attribute | Built-in macro: `-diff -merge -text`. | [gitattributes][git-attr-binary] |
| `textconv` | Runs a program to turn the blob into text for `diff`/`log` only. Needs a `diff=<driver>` attribute. Verified: `-c diff.u16.textconv=…` alone still prints "Binary files differ". | [gitattributes][git-attr-textconv] |
| `core.bigFileThreshold` | Default **512 MiB**. Larger files are treated as binary and not diffed. In our run it applied to commit-vs-commit but **not** to index-vs-worktree `--stat` (see experiments). | [core.adoc][git-bft], [diff.c][git-bft-code] |
| Hard cap | xdiff refuses inputs > 1023 MiB (`MAX_XDIFF_SIZE`). | [xdiff-interface.h][git-maxsize] |
| Non-UTF-8 | git never transcodes diff content. UTF-16 counts as binary because of its NULs. `working-tree-encoding=X` re-encodes to UTF-8 in the index (verified: the blob is stored UTF-8). `i18n.commitEncoding` only covers commit messages, so it doesn't matter here. | [gitattributes][git-attr-wte], [i18n.adoc][git-i18n] |
| `--minimal` | Truly minimal: `need_min` skips both the snake heuristic **and** the `mxcost` cap. | [xdiffi.c][git-needmin] |

### Recommended crates

| Need | Crate | Version / date | Licence | Notes |
|---|---|---|---|---|
| Decode + BOM sniff | **`encoding_rs`** | 0.8.42, 2026-09-24 | `(Apache-2.0 OR MIT) AND BSD-3-Clause` | WHATWG encodings. `Encoding::for_bom` (UTF-8/UTF-16 only). UTF-8 input is borrowed with no copy. **No UTF-32. No UTF-16 encoder** (decode-only is fine for us). 50 MB: UTF-8 in 2.0 ms, windows-1252 in 16 ms. |
| Guess legacy encodings | **`chardetng`** | 1.0.0, 2026-03-30 (previous release 0.1.17, 2021) | `Apache-2.0 OR MIT` | Firefox's detector, built on `encoding_rs`. Doesn't detect UTF-16 (that's the BOM layer's job) and only returns UTF-8 when allowed. **Feed only a prefix:** 64 KB takes 3 ms, 50 MB takes 2.6 s. |
| Binary sniffing | none: ~10 lines of our own | — | — | `content_inspector` 0.2.4 was last pushed in 2018. All it does is a 1024-byte NUL scan, BOM detection and `%PDF`/`\x89PNG` magic ([lib.rs][ci-src]), so it isn't worth a dependency. Use git's NUL-in-8000 rule. |

### Measured timings

`gix-imara-diff` 0.3.0, `Algorithm::Myers`. Each figure is the median of 3 runs and covers intern + diff + postprocess (or intern + diff for word diffs). File reading and decoding are not included. The bench source is in the [appendix](#appendix-bench-source).

Machine: Linux desktop (the bench is single-threaded), Linux 7.2.5, rustc 1.98.1, default `--release` profile (no LTO).

| Case | Size | Myers total (diff part) | Histogram | Notes |
|---|---|---|---|---|
| Baseline: 100k lines, 1 in 500 modified | 5.2 MB | **4.9 ms** (1.1) | 25 ms | |
| 100k lines, all unique, no line shared | 4.9 MB | **4.3 ms** (0.7) | 4.1 ms | One-sided tokens are discarded during preprocessing, so this is trivial. |
| 100k unique lines, right Side shuffled | 4.4 MB | **144 ms** (140) | 7.7 ms | Histogram is fast but cruder: 25 hunks against Myers' 229. |
| 100k lines from a 64-line pool, both Sides random | 3.3 MB | **240 ms** (236) | 240 ms | Histogram falls back to Myers. This is the true worst case. |
| 1M lines, 1 in 1000 modified | 43 MB | **100 ms** (8.8) | 1221 ms | Interning takes 72–103 ms of it. |
| ~50 MB, 600k lines, 1% of lines modified at random | 53 MB | **42 ms** (6.4) | 126 ms | Interning takes 34 ms. |
| ~50 MB, 20 blocks of 100 modified lines | 53 MB | **80 ms** (7.0) | 47 ms | Interning varies 45–76 ms between runs. |
| 2000 × 10k-char lines; 200-line block, 2% of words changed: line diff | 20 MB | **3.0 ms** | — | |
| …word diff of that 200-line block | 800k tokens per Side | **143 ms** (119) | 159 ms | 15.9k tokens changed |
| …word diff of a single 10k-char line pair | 4k tokens | **0.2 ms** | — | |
| One 5 MB minified line, 1% of words changed: word diff | 2.0M tokens per Side | **245 ms** (191) | 272 ms | Line diff of the same file: 0.5 ms. |
| 5k-line modified block, 10% of words changed: word diff | 169k tokens per Side | **37 ms** (32) | 39 ms | Realistic case. |
| 5k-line modified block, both Sides random over 40 words: word diff | 169k tokens per Side | **630 ms** (625) | 638 ms | Worst-case word diff. |

How the pathological shapes scale (Myers, median total). These come from a separate batch (`run2.sh`), so the 100k figures differ slightly from the table above:

| Lines per Side | 10k | 30k | 100k | 300k | 1M |
|---|---|---|---|---|---|
| Shuffled unique lines | 10 ms | 23 ms | 144 ms | 946 ms | 6.46 s |
| Random lines from a 64-line pool | 13 ms | 40 ms | 232 ms | 1.38 s | 8.96 s |

| Worst-case word block (lines / tokens, both Sides) | 500 / 34k | 1k / 68k | 2k / 135k | 3k / 203k | 5k / 338k | 10k / 677k | 20k / 1.35M |
|---|---|---|---|---|---|---|---|
| Myers word diff | 18 ms | 66 ms | 133 ms | 196 ms | 626 ms | 1.25 s | 4.86 s |

Encoding costs on a 56 MB file (median of 3):

| Step | Time |
|---|---|
| `std::str::from_utf8` | 5.8 ms |
| `encoding_rs` UTF-8 decode (valid input, borrowed) | 2.0 ms |
| `encoding_rs` windows-1252 decode | 16 ms |
| NUL check in first 8000 bytes | 0.002 ms |
| `chardetng`: 8 KB / 64 KB / whole 56 MB | 0.4 ms / 3.1 ms / 2631 ms |

### Key findings that change the plan

1. **`Algorithm::MyersMinimal` in `gix-imara-diff` is not minimal.** The search loop is `while ec <= self.max_cost`, with `max_cost = max(√(N+M+3), 256)`, and `need_min` only skips the snake heuristic ([myers.rs][imara-maxcost], [split][imara-split]). git's `xdl_split` instead loops forever and `continue`s past both the heuristic and the `mxcost` cap when `need_min` is set ([xdiffi.c][git-needmin]). The timings agree: `minimal` matched `myers` within noise on every case. One example: the worst-case 5k-line word block (D ≈ 173k) took 635 ms, which a true O(ND) algorithm can't do.
   - Consequence: imara never fails and never takes quadratic time, but past that cap its result is a heuristic. The "too big" decision is entirely ours to make.
2. **Diff time isn't the problem for large, mostly similar files; interning is.** For 1M lines or 50 MB, Myers takes 6–9 ms and interning 30–100 ms. A 20 MB IntelliJ-style limit is about memory and rendering, not Myers.
3. **Pathological shapes grow faster than linearly, and 100k lines is already at the bar.** On this machine, 100k shuffled or random-pool lines take 144–240 ms; 300k take 0.9–1.4 s; 1M take 6–9 s. A line-count threshold can't hold the bar, because a 100k-line file is fine or pathological depending only on its content. The line diff must run off the UI thread with a deadline. `gix-imara-diff` has no cancellation or deadline hook, so the options are:
   - discard the result on timeout (the thread keeps burning CPU until it finishes), or
   - fork the crate's `myers.rs` (Apache-2.0) and add a check to the `split` loop. That loop is also where an IntelliJ-style D cap would go.
4. **Histogram is not a safe fallback.** It is 20× faster on shuffled lines but gives a cruder result there, it is 12× *slower* on 1M sparse lines, and it falls back to Myers on the true worst case anyway. IntelliJ's fallback (patience, then give up) is closer to "detect and stop" than "try another algorithm".
5. **Word refinement needs a per-block cap.** IntelliJ caps on edit distance D. imara doesn't expose D, so the proxy is a token count. The measured crossover: about 135k tokens costs 133 ms in the worst case, 203k costs 196 ms, and 338k costs 626 ms.
   - The proxy is coarser than IntelliJ's cap. The 5 MB minified line has 4M tokens but only D ≈ 19k, so IntelliJ would refine it while a token cap would not.
   - Keep `MAX_BAD_LINES = 3`, and refine lazily for visible blocks as the [prior research][prior] suggests.

### Suggested thresholds (relative to < 300 ms open, 60 fps)

| Decision | Suggested rule | Why |
|---|---|---|
| Binary | 1. Honour gitattributes: `-diff`/`binary` → binary; `diff` set → text. 2. Else a BOM via `encoding_rs::Encoding::for_bom` → text in that encoding. 3. Else a NUL in the first 8000 bytes → binary. Show "Binary files differ / identical" using a byte comparison, as IntelliJ's binary viewer does. | Matches git for everything except UTF-16 with a BOM. That case matches IntelliJ instead, which shows it as text and is the better UX. |
| Per-Side size | Diff fully up to **20 MB** per Side, IntelliJ's default. Above that, show a banner with "Compare anyway". Treat as binary above `core.bigFileThreshold` (default 512 MiB) and above xdiff's 1023 MiB. | Measured: 50 MB takes 40–80 ms to intern and diff, so 20 MB is conservative for compute and leaves headroom for decoding, line indexing and rendering. |
| Line diff deadline | Run off the UI thread. Paint both Sides right away; show highlighting when it arrives. After roughly **2 s**, stop and show IntelliJ's text: "Unable to calculate diff. File is too big and there are too many changes." Start with discard-on-timeout; fork imara's `split` loop only if the wasted CPU matters. | The pathological 300k–1M cases take 1–9 s. Normal files of any size finish in < 100 ms. |
| Word refinement | Skip a block when tokens(left) + tokens(right) > **~100k**: about 100 ms worst case here, so roughly 300 ms on a laptop. Keep `MAX_BAD_LINES = 3`. Refine visible blocks first. | Measured crossover above. Realistic 5k-line blocks (37 ms) and single 10k-char lines (0.2 ms) are far below it. |
| Encoding | BOM → `encoding_rs` decode; else valid UTF-8 → zero-copy; else `chardetng` on the **first 64 KB** → `encoding_rs` decode. For working-tree Sides, honour `working-tree-encoding`. Show the charset in the title when the Sides differ, a red banner on decode errors, and "Contents have differences only in encoding" when the decoded texts are equal. | Detection costs ≤ 3 ms on the prefix against 2.6 s on the whole file. Without `working-tree-encoding`, index (UTF-8) vs working tree (UTF-16) would show the whole file as changed. |

## Detailed findings

### 1. IntelliJ

#### Cost caps and `DiffTooBigException`

- `DiffConfig`: `DELTA_THRESHOLD_SIZE = 20000`, `MAX_BAD_LINES = 3`, `USE_PATIENCE_ALG = false` ([DiffConfig.kt][ij-diffconfig]).
- `MyersLCS.executeWithThreshold()` caps D at `max(20000 + 10·√(count1+count2), 20000)` and throws `FilesTooBigForDiffException` past it ([MyersLCS.kt][ij-myers]).
- `Diff.buildChanges` catches that and retries with `PatienceIntLCS.execute(failOnSmallReduction = true)` ([Diff.kt][ij-diffkt]). Patience throws again when the unique-line recursion doesn't at least halve both Sides ([checkReduction][ij-patience-reduce]).
- `DiffIterableUtil.diff` maps that exception to `DiffTooBigException` ([DiffIterableUtil.kt][ij-diu]). `DiffTooBigException` extends `ProcessCanceledException` ([DiffTooBigException.kt][ij-dtbe]).
- Note: `CancellationChecker` is not threaded into `Diff.buildChanges` (`// TODO: use CancellationChecker inside`). IntelliJ's only guard against slow LCS is the D cap ([DiffIterableUtil.kt][ij-diu]).

#### How the viewer reacts

- **Line level:** `SimpleDiffViewer.performRediff` catches `DiffTooBigException` and calls `applyNotification(DiffNotifications.createDiffTooBig())` ([SimpleDiffViewer][ij-sdv-catch]). The notification text is `error.can.not.calculate.diff.file.too.big` = "Unable to calculate diff. File is too big and there are too many changes." ([DiffNotifications][ij-notif-toobig], [DiffBundle][ij-bundle-toobig]). Both editors are still shown, with no change highlighting. `UnifiedDiffViewer` catches the same exception and switches its panel to `setTooBigContent()` ([UnifiedDiffViewer][ij-udv-catch]). `SimpleThreesideDiffViewer` and `MergeThreesideViewer` also reference the exception; their handling wasn't traced.
- **Word level:** `ComparisonManagerImpl.createInnerFragments` catches `DiffTooBigException` per `LineFragment`. It keeps the fragment without inner fragments and increments `tooBigChunksCount`. Once `tooBigChunksCount >= MAX_BAD_LINES`, refinement isn't attempted at all ([ComparisonManagerImpl][ij-cmi-bad]). Pure insertions and deletions are never refined ([createInnerFragments][ij-cmi-inner]).
- **Identical:** `SimpleDiffViewer.apply` adds `createEqualContentsNotification` when there are no fragments and the texts are equal ([SimpleDiffViewer][ij-sdv-equal]). The message is chosen from charset and separator equality: "Contents are identical", "…differences only in line separators", "…only in encoding", or "…only in line separators and encoding" ([DiffNotifications][ij-notif-equal], [DiffBundle][ij-bundle-msgs]). The notice is suppressed when either Side is a partial preview ([TextDiffViewerUtil][ij-tdvu-equal]).

#### File-size limits

- `FileUtilRt.LARGE_FOR_CONTENT_LOADING = max(20 MB, idea.max.intellisense.filesize, idea.max.content.load.filesize)` ([FileUtilRt][ij-fur-limits]).
  - `idea.max.intellisense.filesize` defaults to 2500 KB; `idea.max.content.load.filesize` defaults to 20 MB.
  - Values are in KB, and a negative value means unlimited ([parseKilobyteProperty][ij-fur-props]).
- `LARGE_FILE_PREVIEW_SIZE = min(idea.max.content.load.large.preview.size (default 2500 KB), LARGE_FOR_CONTENT_LOADING)` ([FileUtilRt][ij-fur-limits]).
- `FileSizeLimit` is the non-deprecated façade. Per-extension limits can only *raise* the defaults. `getEncodingDetectionLimit` defaults to `Int.MAX_VALUE` ([FileSizeLimit.kt][ij-fsl]).
- `FileDocumentManagerBase.getDocument`:
  - A binary type over the limit gets no document.
  - A text file over the limit loads `getPreviewCharCount` characters (preview bytes ÷ average bytes per char of the charset).
  - That document is marked `BIG_FILE_PREVIEW` and made read-only ([FileDocumentManagerBase][ij-fdmb-load], [getPreviewCharCount][ij-fdmb-preview]).
- The diff title then shows "File is too large. Only preview is loaded." ([DiffUtil][ij-du-preview]).
- VCS revisions: `ContentRevisionCache.checkContentsSize` throws when the size exceeds `VcsUtil.getMaxVcsLoadedFileSize()`. That is `idea.max.vcs.loaded.size.kb`, defaulting to the content-load limit ([ContentRevisionCache][ij-crc], [VcsUtil][ij-vcsutil]). The message suggests raising the property ([VcsBundle][ij-vcsbundle]).
- Takeaway: IntelliJ never diffs a text Side above 20 MB. It diffs the 2.5 MB previews instead, read-only and with a banner.

#### Binary files

- `DiffContentFactoryImpl.isBinaryContent` decides binary in this order ([DiffContentFactoryImpl][ij-dcf-binary]):
  1. `UIBasedFileType` (SVG, UI forms) → binary.
  2. Text file type → text.
  3. Unknown or native type, or registry `diff.use.aggressive.text.file.detection` (default `false`, [registry.properties][ij-registry]) → binary iff `guessCharsetFromContent` returns null. That happens when there is no BOM and the content is neither 7-bit nor valid UTF-8.
  4. Any other binary type → binary.
- `CharsetToolkit.guessFromContent` returns `BINARY` for any byte < `BINARY_THRESHOLD = 9`. This is stricter than git, which only looks for NUL ([CharsetToolkit][ij-ct-guess]).
- Binary Comparisons open `BinaryDiffTool` → `TwosideBinaryDiffViewer` ([BinaryDiffTool][ij-bdt]):
  - Different non-zero lengths → "different", without reading.
  - Else, either Side `isTooLarge()` (content-load limit) → "Files are too large to compare".
  - Else a stream comparison → "Files contents are identical" or "Files contents are different" ([TwosideBinaryDiffViewer][ij-tbdv], [status][ij-tbdv-status]).

#### Huge single lines

There is no line-length special case:

- `grep -rn -i -E 'max_line_length|maxLineLength|line_length_limit|TOO_LONG|tooLong|LONG_LINE' platform/diff-impl/src platform/util/diff/src` returns nothing.
- The only `diff.*` registry keys are cosmetic, plus `diff.use.aggressive.text.file.detection` ([registry.properties][ij-registry]).

A minified file is one line, so the line diff is trivial (0.5 ms measured for 5 MB). The whole cost sits in word refinement, and that is bounded only by the D cap and the `MAX_BAD_LINES` fallback.

#### Encoding

- Local files, in `LoadTextUtil.guessFromBytes` ([LoadTextUtil][ij-ltu-guess]):
  1. A BOM (UTF-8, UTF-16LE/BE, UTF-32LE/BE) wins.
  2. Else `guessFromContent` up to `getEncodingDetectionLimit` (unlimited by default).
  3. `VALID_UTF8` → UTF-8, "ignore all directives".
  4. `INVALID_UTF8` → the configured charset, but only if decoding and re-encoding round-trips (`isEncodingSafe`).
  5. Otherwise the file type's or encoding manager's charset is used.
  - The whole step can be disabled with `-Didea.guess.utf.encoding=false`.
- Byte arrays (VCS revisions), in `DiffContentFactoryImpl.guessCharset` ([DiffContentFactoryImpl][ij-dcf-charset]):
  1. BOM.
  2. Caller default.
  3. The file type's charset.
  4. The highlight file's charset, or the global default. Then `takeCharsetOrGuessUTF`: keep that charset if it decodes cleanly, else sniff 7-bit/UTF-8, else keep it anyway.
- `TextContent.fromBytes` decodes. On malformed input it adds a red error notification, "Content was decoded with errors (using 'X' charset)" ([TextContent][ij-dcf-textcontent]).
- When charset or BOM differ between Sides, `DiffUtil` shows the charset in each title, with a " BOM" suffix; UTF-8 is coloured blue ([DiffUtil][ij-du-title], [areEqualCharsets][ij-tdvu-equal]).
- IntelliJ itself doesn't do statistical detection of legacy 8-bit encodings: Windows-1252 vs 1251 comes from configuration. `chardetng` would be an improvement over IntelliJ here, not parity.

### 2. git

- `buffer_is_binary`: `memchr(ptr, 0, min(size, 8000))` ([xdiff-interface.c][git-binary]). `diff_filespec_is_binary` checks the driver's `binary` setting first, then populates the filespec with `check_binary`, then applies the NUL rule ([diff.c][git-isbinary]).
- The checkout/`text=auto` conversion uses a different, stricter rule: lone CR, NUL, or `printable/128 < nonprintable` ([convert.c][git-convert]). It doesn't apply to diff.
- Attributes: `diff`, `-diff`, `diff=<driver>` ([gitattributes][git-attr-diff]); `binary` = `-diff -merge -text` ([gitattributes][git-attr-binary]); `textconv` ([gitattributes][git-attr-textconv]); `diff.<driver>.binary` ([gitattributes][git-attr-markbinary]). `-a/--text` forces text ([diff-options][git-text]).
- `core.bigFileThreshold`: default 512 MiB. Files above it are "treated as if they were labeled binary" and `git diff` will not compute diffs for them ([core.adoc][git-bft]). The code checks `size > threshold && is_binary == -1` inside `diff_populate_filespec` ([diff.c][git-bft-code]).
- `MAX_XDIFF_SIZE = 1023 MiB`: `xdi_diff` returns -1 above it ([xdiff-interface.h][git-maxsize], [xdiff-interface.c][git-maxsize-c]).
- `working-tree-encoding`: content is re-encoded to UTF-8 on add and back on checkout. UTF-16 without it counts as binary. JGit and libgit2 don't support it ([gitattributes][git-attr-wte]). Relevance for us: if we read working-tree files directly, as gitoxide does, we must apply this ourselves.
- `i18n.commitEncoding`: covers commit messages only ([i18n.adoc][git-i18n]), so it doesn't matter here.
- xdiff Myers: `mxcost = max(bogosqrt(ndiags), XDL_MAX_COST_MIN)`. The `--minimal` flag bypasses it ([xdiffi.c][git-needmin], [mxcost][git-mxcost]).

#### git experiments

Run with `$TMPDIR/gitdemo.sh` and `gitdemo2.sh` (git 2.55.0). Each file was committed, then changed:

```
$ git diff --stat
 latin1.txt          |   2 +-
 nul_after_8000.txt  |   2 +-          # NUL at byte 8001 → text
 nul_before_8000.txt | Bin 7997 -> 7997 bytes   # NUL at byte 7991 → binary
 utf16.txt           | Bin 26 -> 26 bytes
 wte.txt             |   2 +-          # working-tree-encoding=UTF-16 → text
$ git diff latin1.txt | grep -a '^[-+]caf' | od -c     # raw 0xE9 (351 octal) passes through
-  c a f 351   o n e \n + c a f 351   O N E \n
$ git -c diff.u16.textconv='iconv -f UTF-16 -t UTF-8' diff utf16.txt   # no diff= attribute
Binary files i/utf16.txt and w/utf16.txt differ
$ echo 'utf16.txt diff=u16' >> .gitattributes; git -c diff.u16.textconv='iconv -f UTF-16 -t UTF-8' diff utf16.txt
-world
+WORLD
$ git cat-file -p HEAD:wte.txt | od -c        # stored as UTF-8 in the object DB
h e l l o \n w o r l d \n
$ git -c core.bigFileThreshold=1k diff --stat nul_after_8000.txt            # index vs worktree
 nul_after_8000.txt | 2 +-                     # threshold NOT applied
$ git -c core.bigFileThreshold=1k diff --stat HEAD~1 HEAD -- nul_after_8000.txt   # commit vs commit
 nul_after_8000.txt | Bin 8007 -> 8007 bytes   # threshold applied
```

The likely mechanism for the `core.bigFileThreshold` discrepancy: `diff_filespec_is_binary` calls `diff_populate_filespec`, which holds the size check, only when `!one->data` ([diff.c][git-isbinary]). So the threshold is skipped when the data is already loaded. Which code path pre-loaded the worktree Side in the `--stat` run was **not traced**. Either way, don't rely on git's threshold behaving the same for every Source pair: apply our own size rule.

### 3. Rust crates

- **`encoding_rs` 0.8.42** (2026-09-24; three releases in September 2026; 551M downloads; by Henri Sivonen, the Firefox encoding owner) ([crates.io][cr-encrs]).
  - Licence: `(Apache-2.0 OR MIT) AND BSD-3-Clause`; the BSD part covers the WHATWG index data.
  - API: `Encoding::for_bom` ([lib.rs][encrs-forbom]); `decode`, which sniffs the BOM and replaces bad sequences; `decode_with_bom_removal`; `decode_without_bom_handling_and_without_replacement`, which returns `None` on error and suits a strict "is this valid X?" check; and a streaming `new_decoder` ([lib.rs][encrs-decode]).
  - Limits: no UTF-32 ([lib.rs][encrs-utf32]) and no UTF-16 encoder ([lib.rs][encrs-utf16]). We only decode, so the encoder gap doesn't matter. UTF-32 files need ~20 lines of our own, or can be treated as binary. IntelliJ does support UTF-32 BOMs ([CharsetToolkit][ij-ct-bom]).
- **`chardetng` 1.0.0** (2026-03-30, the first release since 0.1.17 in 2021; 14M downloads; `Apache-2.0 OR MIT`) ([crates.io][cr-chardet]).
  - It is Firefox's detector for unlabelled legacy content. It rules encodings out on any decode error or C1 control, and scores single-byte encodings by Wikipedia character-pair frequencies ([README][chardet-readme]).
  - Accuracy, from the author's write-up ([hsivonen.fi/chardetng][chardet-blog]):
    - "competitive with ced" (Chrome's detector) and 42% slower than ced;
    - at an 80% bar on title-length input, only Latvian (61%) and Lithuanian (48%) fall below;
    - GBK is 88% against ced's 95% on titles, weak below 6 hanzi;
    - legacy CJK reaches full-document accuracy with about 10 non-ASCII bytes.
  - It doesn't detect UTF-16, and returns UTF-8 only with `Utf8Detection::Allow`.
  - Measured: about 21 MB/s, so **always cap the input**. 64 KB took 3 ms and guessed windows-1252 correctly.
- **`content_inspector` 0.2.4**: last release 2018-11-04 and no pushes since; licensed MIT/Apache-2.0. Its heuristic is a BOM table, a NUL scan of the first **1024** bytes, and `%PDF`/`\x89PNG` magic ([lib.rs][ci-src], [crates.io][cr-ci]). Don't add it. git's 8000-byte rule is the de-facto standard, and it's one `memchr` call.
- `encoding_rs` 0.8.42 pulls in `simdutf8` (`cargo tree -i simdutf8`). That probably explains why its UTF-8 path beat `std::str::from_utf8` in our run (2.0 vs 5.8 ms on 56 MB). Validating through `encoding_rs` therefore needs no extra crate.

### 4. `gix-imara-diff` 0.3 internals relevant to caps

- Myers preprocessing discards tokens that occur only on one Side ([preprocess.rs][imara-pre]). That's why the "100k all-different lines" case takes 4 ms: everything is discarded and no search runs.
- `max_cost = max(√(N+M+3), 256)` and `HEUR_MIN_COST = 256` ([myers.rs][imara-maxcost]). The `split` loop is bounded by `ec <= max_cost` whatever `need_min` is ([myers.rs][imara-split]). At the cap it takes the furthest-reaching diagonal, as git does.
- Histogram falls back to Myers for highly repeated tokens (64+ occurrences) ([lib.rs][imara-alg]). That's why the 64-line-pool case is identical under both algorithms.
- There is no deadline, cancellation or progress hook anywhere in the crate (`grep -rn -i -c 'deadline\|cancel' src/` → 0 in every file). The `similar` crate does have a `deadline` ([prior research][prior]).

## Open questions

- Should the ~2 s line-diff deadline become a D cap in a forked `split` loop, which would give IntelliJ parity and deterministic output? Decide once lazy refinement is built and the CPU waste of discard-on-timeout can be measured.
- UTF-16 **without** a BOM: git, IntelliJ's `CharsetToolkit` and `chardetng` all treat it as binary or don't detect it. A cheap heuristic is many NULs at alternating byte positions in the first 8000 bytes. Is it worth adding?
- Should `textconv` drivers run? That means executing configured programs. It is probably opt-in, and only for git Sources.

## Appendix: bench source

`$TMPDIR/lbe-bench/` (outside the repo). `Cargo.toml` dependencies: `gix-imara-diff = "0.3"`, `encoding_rs = "0.8.42"`, `chardetng = "1.0.0"`. Run with `cargo build --release && bash run.sh && bash run2.sh`; `run.sh` runs each case 3× with Myers and 1× each with MyersMinimal and Histogram.

<details><summary>src/main.rs (abridged: case generators)</summary>

```rust
// words_line(rng, n): random words from a 40-word Rust-ish vocabulary until n chars.
// mutate_words(rng, line, pct): replace ~pct% of space-separated words with fresh identifiers.
// Line diff  = InternedInput::new(a, b) → Diff::compute(alg) → postprocess_lines.
// Word diff  = InternedInput::new(sources::words(a), sources::words(b)) → Diff::compute(alg) → postprocess_no_heuristic.
"base100k"         100k lines of ~40 chars + " // i"; every 500th line gets " changed" appended
"unique100k"       left "left line i …", right "right line i …" (no shared line)
"shuffled100k"     100k unique lines; right = Fisher–Yates shuffle of left
"smallvocab100k"   pool of 64 random ~30-char lines; each Side = 100k independent draws
"sparse1m"         1M lines of ~30 chars; every 1000th line modified
"sparse50mb"       600k lines of ~80 chars; each line modified (20% of words) with p = 1%
"blocks50mb"       same file; 20 blocks of 100 consecutive lines modified (20% of words)
"longlines"        2000 lines × 10k chars; lines 900..1100 get 2% of words changed;
                   then word diff of that block, and of line 1000 alone
"minified5mb"      one 5 MB line, ';'-separated words; 1% of words changed
"block5k-realistic" 5000 × 80-char lines; right = left with 10% of words changed
"block5k-worst"     5000 × 80-char lines per Side, generated independently
"encoding50mb"     600k lines incl. "café" → from_utf8, encoding_rs UTF-8 / windows-1252 decode,
                   NUL scan of 8000 bytes, chardetng on 8 KB / 64 KB / all
```

</details>

<!-- links -->
[prior]: ./intellij-diff-refinement.md
[ij]: https://github.com/JetBrains/intellij-community/tree/d846a4b059167acb3525a88d1ede9b8bd8dafa59
[git]: https://github.com/git/git/tree/a018953688f1b10bddf91bff8747068f5f4746a4
[ij-diffconfig]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/diff/src/com/intellij/util/diff/DiffConfig.kt#L7-L13
[ij-myers]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/diff/src/com/intellij/util/diff/MyersLCS.kt#L56-L86
[ij-diffkt]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/diff/src/com/intellij/util/diff/Diff.kt#L81-L98
[ij-patience-reduce]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/diff/src/com/intellij/util/diff/PatienceIntLCS.kt#L154-L158
[ij-diu]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/diff/src/com/intellij/diff/comparison/iterables/DiffIterableUtil.kt#L24-L37
[ij-dtbe]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/diff/src/com/intellij/diff/comparison/DiffTooBigException.kt
[ij-sdv-catch]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/simple/SimpleDiffViewer.java#L272-L287
[ij-udv-catch]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/fragmented/UnifiedDiffViewer.java#L392-L400
[ij-sdv-equal]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/simple/SimpleDiffViewer.java#L300-L329
[ij-notif-toobig]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/util/DiffNotifications.kt#L50-L53
[ij-notif-equal]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/util/DiffNotifications.kt#L29-L39
[ij-bundle-toobig]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-api/resources/messages/DiffBundle.properties#L298
[ij-bundle-msgs]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-api/resources/messages/DiffBundle.properties#L42-L48
[ij-cmi-bad]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L216-L241
[ij-cmi-inner]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L243-L278
[ij-fur-limits]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util-rt/src/com/intellij/openapi/util/io/FileUtilRt.java#L57-L77
[ij-fur-props]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util-rt/src/com/intellij/openapi/util/io/FileUtilRt.java#L1091-L1111
[ij-fsl]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/core-api/src/com/intellij/openapi/vfs/limits/FileSizeLimit.kt#L15-L122
[ij-fdmb-load]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/core-impl/src/com/intellij/openapi/fileEditor/impl/FileDocumentManagerBase.java#L73-L95
[ij-fdmb-preview]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/core-impl/src/com/intellij/openapi/fileEditor/impl/FileDocumentManagerBase.java#L253-L260
[ij-du-preview]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/util/DiffUtil.java#L810-L817
[ij-du-title]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/util/DiffUtil.java#L830-L845
[ij-tdvu-equal]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/util/base/TextDiffViewerUtil.java#L173-L191
[ij-vcsutil]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/vcs-api/src/com/intellij/vcsUtil/VcsUtil.java#L76-L96
[ij-crc]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/vcs-api/src/com/intellij/openapi/vcs/impl/ContentRevisionCache.java#L124-L137
[ij-vcsbundle]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/vcs-api/vcs-api-core/resources/messages/VcsBundle.properties#L1051
[ij-dcf-binary]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/DiffContentFactoryImpl.java#L441-L457
[ij-dcf-charset]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/DiffContentFactoryImpl.java#L460-L525
[ij-dcf-textcontent]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/DiffContentFactoryImpl.java#L839-L860
[ij-ct-guess]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/src/com/intellij/openapi/vfs/CharsetToolkit.java#L293-L418
[ij-ct-bom]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/src/com/intellij/openapi/vfs/CharsetToolkit.java#L424-L432
[ij-ltu-guess]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/core-impl/src/com/intellij/openapi/fileEditor/impl/LoadTextUtil.java#L387-L445
[ij-tbdv]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/binary/TwosideBinaryDiffViewer.java#L110-L147
[ij-tbdv-status]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/binary/TwosideBinaryDiffViewer.java#L201-L213
[ij-bdt]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/diff-impl/src/com/intellij/diff/tools/binary/BinaryDiffTool.java#L16-L29
[ij-registry]: https://github.com/JetBrains/intellij-community/blob/d846a4b059167acb3525a88d1ede9b8bd8dafa59/platform/util/resources/misc/registry.properties#L1077-L1100
[git-binary]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/xdiff-interface.c#L197-L203
[git-maxsize]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/xdiff-interface.h#L10-L14
[git-maxsize-c]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/xdiff-interface.c#L124-L125
[git-isbinary]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L3691-L3713
[git-bft-code]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/diff.c#L4545-L4610
[git-convert]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/convert.c#L90-L103
[git-needmin]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/xdiff/xdiffi.c#L67-L212
[git-mxcost]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/xdiff/xdiffi.c#L351-L359
[git-bft]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/core.adoc#L460-L482
[git-attr-wte]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/gitattributes.adoc#L305-L330
[git-attr-diff]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/gitattributes.adoc#L728-L765
[git-attr-textconv]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/gitattributes.adoc#L942-L975
[git-attr-markbinary]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/gitattributes.adoc#L1031-L1062
[git-attr-binary]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/gitattributes.adoc#L1294-L1321
[git-i18n]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/config/i18n.adoc#L1-L6
[git-text]: https://github.com/git/git/blob/a018953688f1b10bddf91bff8747068f5f4746a4/Documentation/diff-options.adoc#L769-L771
[imara-maxcost]: https://docs.rs/crate/gix-imara-diff/0.3.0/source/src/myers.rs#L37-L78
[imara-split]: https://docs.rs/crate/gix-imara-diff/0.3.0/source/src/myers.rs#L113-L201
[imara-pre]: https://docs.rs/crate/gix-imara-diff/0.3.0/source/src/myers/preprocess.rs
[imara-alg]: https://docs.rs/crate/gix-imara-diff/0.3.0/source/src/lib.rs#L172-L230
[cr-encrs]: https://crates.io/crates/encoding_rs
[cr-chardet]: https://crates.io/crates/chardetng
[cr-ci]: https://crates.io/crates/content_inspector
[encrs-forbom]: https://docs.rs/crate/encoding_rs/0.8.42/source/src/lib.rs#L2909-L2921
[encrs-decode]: https://docs.rs/crate/encoding_rs/0.8.42/source/src/lib.rs#L3039-L3240
[encrs-utf16]: https://docs.rs/crate/encoding_rs/0.8.42/source/src/lib.rs#L201-L216
[encrs-utf32]: https://docs.rs/crate/encoding_rs/0.8.42/source/src/lib.rs#L231-L233
[chardet-readme]: https://github.com/hsivonen/chardetng/blob/master/README.md
[chardet-blog]: https://hsivonen.fi/chardetng/
[ci-src]: https://github.com/sharkdp/content_inspector/blob/master/src/lib.rs#L35-L132
