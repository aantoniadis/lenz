# How IntelliJ refines diffs

Research for ticket `02-intellij-diff-refinement-research`. Terms follow `/CONTEXT.md` (Comparison, Side, Change Block, Inline Change, Connector).

Source read: IntelliJ Community at commit [`bc34875`][ij-head] (2026-10-01), package `com.intellij.diff.comparison`. Most of the engine now lives in Kotlin `*Rt.kt` files under `platform/util/diff`. The Java `ByLine`/`ByWord` classes in `platform/diff-impl` are thin wrappers ([ByLine.java][byline-java], [ByWord.java][byword-java]). Crates read: `imara-diff` 0.2.0 and `similar` 3.2.0, both from their published crate sources.

---

## Summary

### The pipeline in steps

Default settings are by-word highlighting, no ignore policy, no aligned mode ([TextDiffSettingsHolder][settings-defaults]). With those defaults, two texts turn into Change Blocks and Inline Changes like this:

1. **Split into lines on `\n` only.** IntelliJ documents are already `\n`-normalised. "Whitespace" everywhere means only `' '`, `'\t'` and `'\n'`, never `'\r'` ([LineOffsetsImpl][lineoffsets], [isSpaceEnterOrTab][ws-pred]).
2. **Line LCS always runs ignoring whitespace first, whatever policy the user picked.** Each line is hashed with all whitespace stripped ([ByLineRt.doCompare][byline-docompare]).
3. **"Unimportant" lines can't anchor the match.** Lines with 3 or fewer non-whitespace characters (`}`, `});`, blank lines) are left out of the first LCS pass. Gaps between matched "big" lines are then diffed again with every line included ([compareSmart][byline-smart], [SmartLineChangeCorrector][cc-smart], [DiffConfig][diffconfig]).
4. **The base algorithm is Myers with a cost cap.** It runs on integer-enumerated tokens after removing the common prefix and suffix, and after discarding tokens that appear on only one Side. If the cap is hit, it falls back to a patience-style unique-LCS ([Diff.kt][diff-build], [MyersLCS][myers-threshold], [PatienceIntLCS][patience], [Reindexer][reindexer]).
5. **Slide pure insertions and deletions toward empty or unimportant lines.** The slide happens only when the change touches one Side; changes with lines on both Sides never move ([ChunkOptimizer.LineChunkOptimizer][co-line]).
6. **Put the real policy back.** Inside each run that is equal when whitespace is ignored, lines are re-matched under the user's policy. Up to 10 candidates are brute-forced to maximise exactly-equal pairs; beyond that, matching is positional ([correctChangesSecondStep][byline-second]). The result is the list of **Change Blocks**.
7. **Refine each modified block at word level across the whole block**, not line by line:
   - Tokenise into words, newlines, and single CJK/Thai/other continuous-script code points. ASCII punctuation and whitespace are not tokens ([getInlineChunks][byword-chunks]).
   - Run the same LCS.
   - Slide changed words to whitespace-delimited "sentence" boundaries ([WordChunkOptimizer][co-word]).
   - Split the block at matched newlines ([LineFragmentSplitter][splitter]).
   - Match punctuation in the gaps by character diff ([AdjustmentPunctuationMatcher][byword-punct]).
   - Fix up whitespace according to the policy ([DefaultCorrector/TrimSpacesCorrector/IgnoreSpacesCorrector][byword-ws]).

   The result is the **Inline Changes**.
8. **Post-process blocks.** Sub-blocks that touch are squashed back into one Change Block, unless the user picked "split" mode. `IGNORE_WHITESPACES_CHUNKS` also drops leading and trailing whitespace-only sub-blocks ([processBlocks][cmi-process], [TwosideTextDiffProviderBase][provider-base]).
9. **Paint.** A block that has Inline Changes gets a muted line background, and each Inline Change gets the full-strength colour ([SimpleDiffChangeUi][ui-highlight], [DiffDrawUtil][draw-ignored], [TextDiffTypeFactory][ignored-color]).
10. **Lay out side by side.**
    - Default mode: two independent editors, sync-scrolled by piecewise-linear line mapping between Change Block boundaries. Curved Connectors are drawn in the divider ([BaseSyncScrollable][sync-base], [SyncScrollSupport][sync-vertical], [DiffDividerDrawUtil][divider-paint]).
    - Opt-in "aligned" mode: block inlays pad the shorter Side so both editors scroll as one ([AlignedDiffModel][aligned-heights]).

Step 2 is what "perfect" mostly comes down to. Steps 2–3 and 5–7 are the heuristics that the plain Myers or Histogram algorithm in a crate doesn't have.

### Reuse vs own

- **Reuse:** only the core sequence-LCS: integer tokens in, matched and unmatched ranges out. That is IntelliJ's `DiffIterableUtil.diff` layer. Both `imara-diff` and `similar` cover it, including prefix/suffix stripping ([imara compute_with][imara-alg], [similar myers.rs][similar-myers]), and `imara-diff` also discards unique tokens like IntelliJ's `Reindexer`.
- **Own:** essentially all of `com.intellij.diff.comparison`, about 3k lines of Kotlin:
  - line hashing per policy
  - two-pass "big line" matching
  - the second-step policy correction
  - the line and word chunk optimizers
  - word tokenisation with Unicode script rules
  - the line-fragment splitter
  - punctuation matching
  - the three whitespace correctors
  - squash and trim post-processing
  - the sync-scroll and alignment geometry
- **Recommendation:** use `imara-diff` with `Algorithm::Myers` for every LCS call: lines, words, code points, punctuation. Its `compute_with(&[Token])` takes the pre-hashed token arrays our layer will build. Its pluggable `SliderHeuristic` can host IntelliJ's empty-line rule. Then port the refinement layer from IntelliJ with attribution, as described in the [license note](#license-note).
  - Watch: `imara-diff` 0.2.0 was last released in 2025-06. gitoxide now ships a maintained fork, `gix-imara-diff` 0.3.0 (2026-09-25).
  - `similar` is the alternative if built-in deadlines matter more. Its inline-refinement and whitespace modes are a different design, not a drop-in for IntelliJ's.

---

## Detailed findings

### 0. Entry points and policies

**Call chain.**
- The viewer calls `myTextDiffProvider.compare(text1, text2)` and wraps each returned `LineFragment` as one change ([SimpleDiffViewer.computeDifferences][viewer-compute]).
- The provider maps the UI settings to a `ComparisonPolicy` plus an `InnerFragmentsPolicy`, and calls `ComparisonManagerImpl.compareLinesInner` ([SimpleTextDiffProvider.compareRange][simple-provider]).
- It then post-processes the result with `processBlocks(squash, trim)` ([TwosideTextDiffProviderBase][provider-base]).

**Data model.**
- A `LineFragment` covers a line range and a char-offset range on each Side, and holds `innerFragments: List<DiffFragment>?`.
- `null` means "not refined". An empty list means "refined; only ignorable whitespace differs" ([createInnerFragments][cmi-inner]).
- Our Rust type needs the same three states (`None` / `Some(vec![])` / `Some(v)`), because the UI paints them differently ([SimpleDiffChangeUi.createHighlighter][ui-highlight]).

**Ignore policies.** The engine has three `ComparisonPolicy` values: `DEFAULT`, `TRIM_WHITESPACES`, `IGNORE_WHITESPACES` ([ComparisonPolicy.kt][policy]). The UI's `IgnorePolicy` has six values and maps them onto those three ([IgnorePolicy.java][ignore-policy]):

| UI `IgnorePolicy` | `ComparisonPolicy` | squash | trim chunks |
|---|---|---|---|
| `DEFAULT` | `DEFAULT` | yes | no |
| `TRIM_WHITESPACES` | `TRIM_WHITESPACES` | yes | no |
| `IGNORE_WHITESPACES` | `IGNORE_WHITESPACES` | yes | no |
| `IGNORE_WHITESPACES_CHUNKS` | `IGNORE_WHITESPACES` | yes | **yes** |
| `FORMATTING` | `DEFAULT` (plus formatter-provided ignored ranges, outside this package) | yes | no |
| `IGNORE_LANGUAGE_SPECIFIC_CHANGES` (experimental) | `DEFAULT` | **no** | no |

There is **no ignore-case policy** in IntelliJ's engine. The MVP charter asks for ignore-case, so that would be our own extension: a case-folded hash and equality next to the whitespace ones.

**Highlight policies** ([HighlightPolicy.java][highlight-policy]):

| Policy | Inline refinement | Squash |
|---|---|---|
| `BY_LINE` | none | — |
| `BY_WORD` (default) | words | yes |
| `BY_WORD_SPLIT` ("split changes") | words | **no** |
| `BY_CHAR` | chars | yes |
| `DO_NOT_HIGHLIGHT` | none | — |

The provider squashes only when both the highlight policy and the ignore policy allow it ([TwosideTextDiffProviderBase][provider-base]).

**Equality and hashing per policy** ([ComparisonUtil][cu-equals]; helpers in [StringsKmp.kt][strings-kmp]):
- `DEFAULT`: exact characters.
- `TRIM_WHITESPACES`: compare after trimming leading and trailing `' ' \t \n`.
- `IGNORE_WHITESPACES`: skip every `' ' \t \n` on both sides; the hash skips them too.
- The whitespace predicate is the same `' ' \t \n` everywhere, so a stray `\r` counts as content. **We must normalise CRLF ourselves before comparing.**

### 1. Line splitting

`LineOffsetsImpl.create` splits on `'\n'` only. Text after the last `\n` is a final line, possibly empty ([LineOffsetsImpl][lineoffsets]). `compareLines` builds the per-line `CharSequence`s from these offsets without the newline and feeds them to `ByLine` ([ComparisonManagerImpl.compareLines][cmi-lines]). When line ranges become `LineFragment`s, the char-offset end includes the trailing newline, except on the last line ([getOffsets][cmi-offsets]).

### 2. Base algorithm (`com.intellij.util.diff`)

`DiffIterableUtil.diff(list1, list2)` → `Diff.buildChanges` ([DiffIterableUtil][diu-diff], [Diff.kt][diff-build]):

1. Strip the common prefix and suffix. If one Side is empty after that, emit a single change.
2. Enumerate objects to `int` ids using `equals`/`hashCode`. Lines carry a policy-specific hash ([ByLineRt.Line][byline-line]).
3. `Reindexer.discardUnique` drops ids that never occur on the other Side, then maps results back afterwards ([Reindexer][reindexer]).
4. Run `MyersLCS.executeWithThreshold()`: Myers O(ND), linear space, with D capped at `max(20000 + 10·√N, 20000)` ([MyersLCS][myers-threshold], [DiffConfig][diffconfig]).
5. If the cap is hit, run `PatienceIntLCS` instead. It recurses on unique-element LCS (`UniqueLCS`) and uses linear Myers in gaps without unique elements. If that recursion doesn't at least halve the problem, it throws `DiffTooBigException` ([PatienceIntLCS][patience], [checkReduction][patience-reduce]).
6. `USE_PATIENCE_ALG = false`: Myers is the primary algorithm. Patience is only a fallback ([DiffConfig][diffconfig]).

So the base algorithm is classic Myers. The quality comes from what is layered on top.

### 3. Line matching: `ByLineRt.doCompare`

From [ByLineRt.doCompare][byline-docompare]:

- **If the policy is `IGNORE_WHITESPACES`:** `compareSmart` on IW-hashed lines → `optimizeLineChunks` → `expandRanges` (trim equal edges off each change).
- **Otherwise (DEFAULT, TRIM):** convert lines to IW mode → `compareSmart` → `optimizeLineChunks` using the *policy* lines → `correctChangesSecondStep`.

**3a. "Unimportant" lines (`compareSmart`).**
- The threshold is `UNIMPORTANT_LINE_CHAR_COUNT = 3` ([DiffConfig][diffconfig]).
- Only lines with `nonSpaceChars > 3` take part in the first LCS ([getBigLines][byline-biglines]).
- `SmartLineChangeCorrector` walks the matched big lines. For each gap between consecutive matches, it trims equal leading and trailing lines (`expand`), then runs a full LCS over all lines in the gap ([compareSmart][byline-smart], [ChangeCorrector.execute][cc-execute], [SmartLineChangeCorrector][cc-smart]).
- Effect: braces, `end`, and blank lines can never pull the alignment off course. They are matched only inside gaps that real content already bounds.

**3b. Second step: restoring the real policy (`correctChangesSecondStep`).** The IW pass may pair lines that are equal only when whitespace is ignored. The source comment gives the cases ([ByLineRt][byline-second]):
- Lines matched by IW and exactly equal stay matched.
- Lines unmatched by IW stay unmatched.
- For an IW-matched but unequal line:
  - collect every IW-equal line on both Sides in the current unmatched block;
  - pick the alignment that maximises exactly-equal pairs, by brute-force combinations ([getBestMatchingAlignment][byline-bestmatch]);
  - if more than 10 candidates (or equal counts), fall back to positional pairing.
- `ExpandChangeBuilder` trims equal edges from every emitted change ([DiffIterableUtil.ExpandChangeBuilder][diu-expand]).
- Result: re-indentation shows up as *modified* lines, paired with their real counterparts, instead of fragmenting the diff.

### 4. Chunk sliding and boundary heuristics: `ChunkOptimizer`

The base class walks consecutive *unchanged* ranges ([ChunkOptimizer.processLastRanges][co-base]):

- It acts only when the two ranges touch on one Side, i.e. the change between them is a pure insertion or deletion. Changes with lines on both Sides never slide.
- It first tries to **merge**: `[A]B[B] → [AB]B` or `[A]A[B] → A[AB]`, minimising the chunk count.
- Otherwise it asks the subclass for a shift within the slidable window (`equalForward`/`equalBackward`).

**`LineChunkOptimizer`** ([source][co-line]) tries four rules in order and takes the first that applies:

1. Put the insertion boundary next to an **empty unchanged line**, threshold 0.
2. Make the inserted block **start or end with an empty line**, threshold 0.
3. Rule 1 again, with "empty" relaxed to ≤ 3 non-space characters.
4. Rule 2 again, with the same relaxed threshold.

If a boundary already sits next to an empty or unimportant line, the block stays where it is. Otherwise a forward shift is preferred over a backward one ([getShift][co-line-getshift]). This is IntelliJ's equivalent of git's slider heuristics. It is based on blank and unimportant lines, not indentation.

**`WordChunkOptimizer`** ([source][co-word]) does the same merging at word level, then prefers boundaries already separated by whitespace. That way `1.0.123 1.0.155` vs `1.0.123 1.0.134 1.0.155` highlights `1.0.134 ` as one unit instead of splitting inside dotted "sentences".

### 5. Word-level refinement (Inline Changes)

`createInnerFragments` runs for each Change Block ([ComparisonManagerImpl][cmi-inner]):

- **Insertion or deletion:** no refinement. The inner list is empty if the inserted text is equal under the policy (e.g. blank lines under ignore-whitespace), else `null`.
- **Modified block:** `ByWord.compareAndSplit` on the *whole block text*, all lines at once ([createInnerWordFragments][cmi-word]).
- After 3 blocks throw `DiffTooBigException`, later blocks are left unrefined (`MAX_BAD_LINES = 3`) ([createInnerFragments][cmi-inner], [DiffConfig][diffconfig]).

`ByWordRt.compareAndSplit` ([source][byword-split]) runs these steps:

1. **Tokenise** ([getInlineChunks][byword-chunks], [TrimUtil char classes][trim-classes]):
   - A `WordChunk` is a maximal run of "alpha" code points. Alpha means not ASCII whitespace and not ASCII punctuation, so digits, `_` and most non-ASCII characters count.
   - "Continuous script" code points each become their own token: ideographic, Hiragana, Katakana, Thai, Javanese, any non-BMP code point, and non-ASCII non-alphabetic characters such as `—` or emoji.
   - `\n` becomes a `NewlineChunk`.
   - ASCII punctuation (`!"#$%&'()*+,-./:;<=>?@[\]^`` ` ``{|}~`, but not `_`) and spaces/tabs are **not tokens**.
   - The Unicode ranges are embedded tables from PropList and Scripts 16.0.0 ([CharacterUtils][charutils]).
   - Words compare by exact content, whatever the policy ([WordChunk][byword-wordchunk]).
2. **Run the word LCS** with the same `Diff.buildChanges`, then `WordChunkOptimizer` (§4).
3. **Split into line blocks** with `LineFragmentSplitter` ([source][splitter]):
   - Cut at each matched `\n`↔`\n` pair, and before matched words that are first in their line on both Sides.
   - Merge neighbouring pieces when both were matched only by newlines, when both differ only in whitespace, or when one has no words at all.
4. **Match punctuation** within each line block with `AdjustmentPunctuationMatcher` ([source][byword-punct]). For each gap between consecutive matched words, it runs a char-level LCS over *punctuation only* (`ByCharRt.comparePunctuation` ([source][bychar-punct])). The gap may sit between two matched words, or have a matched word on one Side only. Example: in `[ X { A ! B } Y ]` vs `( X ... Y )`, the groups compared are `[`↔`(`, `{`+`}`↔`...`, `]`↔`)`.
5. **Correct whitespace** per policy ([matchAdjustmentWhitespaces][byword-ws]):
   - `DEFAULT` → `DefaultCorrector`: shrink each change by equal whitespace at both ends, so only real differences are highlighted.
   - `TRIM_WHITESPACES` → `DefaultCorrector`, then `TrimSpacesCorrector`: also trim whitespace at line start or end, and drop changes that become equal.
   - `IGNORE_WHITESPACES` → `IgnoreSpacesCorrector`: expand over equal whitespace, trim ignored whitespace, drop IW-equal changes.

   The building blocks are `expandWhitespaces*` (extend a change edge over characters that are equal *and* whitespace) and `trim` (strip whitespace from the edges) ([TrimUtil expandIgnored][trim-expand-ignored]).
6. Each line block becomes a `LineFragment` with block-relative `DiffFragment`s. These are the Inline Changes.

**Squash.** `processBlocks(squash=true)` re-merges *adjoining* fragments (end == next start) into one Change Block and concatenates their inner fragments ([processAdjoining/doSquash][cmi-process]). The splitter's pieces are contiguous, so under the default `BY_WORD` a modified block stays one Change Block. Splitting only becomes visible under `BY_WORD_SPLIT`, which also draws separators between the non-squashed pieces ([SimpleDiffChangeUi][ui-highlight]).

**Char mode (`BY_CHAR`)** uses `ByCharRt` ([source][bychar]):
- `compareTwoStep` (`DEFAULT`): LCS on non-whitespace code points, then a fair char diff inside each gap (`DefaultCharChangeCorrector`, [ChangeCorrector][cc-char]).
- `compareTrimWhitespaces`: the same, then the trim corrector.
- `compareIgnoreWhitespaces`: non-whitespace LCS only, with zero-width insertions placed after matching whitespace ([expandForwardW][bychar-iw]).

**Experimental "words first".** `compareWordsFirst` derives line blocks from the word diff (`LineUplifter`) instead of the reverse. It is aimed at inputs like Java→Kotlin, or re-wrapped prose. It is behind registry key `diff.by.word.deprioritize.line.differences`, **default `false`** ([ByWordRt.compareWordsFirst][byword-wordsfirst], [registry][registry]).

### 6. Ignore-whitespace policies, end to end

| Policy | Line LCS | Line correction | Inline whitespace handling | Block post-process |
|---|---|---|---|---|
| `DEFAULT` | IW-hashed + unimportant-line pass | second step: exact re-match | `DefaultCorrector` (whitespace-only edits still highlighted) | squash |
| `TRIM_WHITESPACES` | same | second step: trim-equality re-match | `Default` + `TrimSpacesCorrector` | squash |
| `IGNORE_WHITESPACES` | IW lines; `expandRanges` | none needed | `IgnoreSpacesCorrector`; whitespace-only blocks get empty inner lists | squash |
| `IGNORE_WHITESPACES_CHUNKS` | as `IGNORE_WHITESPACES` | as `IGNORE_WHITESPACES` | as `IGNORE_WHITESPACES` | squash + trim: per adjoining group, drop leading and trailing fragments that are IW-equal or have empty inner lists; drop the group if nothing is left |

Sources: [ByLineRt.doCompare][byline-docompare], [matchAdjustmentWhitespaces][byword-ws], [processAdjoining][cmi-process], [IgnorePolicy][ignore-policy].

### 7. Painting and side-by-side alignment

**Colours.**
- Change Block kind is inserted, deleted, or modified.
- If `innerFragments != null`, the line background uses the "ignored" (muted) paint mode. The colour is the type's foreground attribute, or the type colour mixed with the editor background ([SimpleDiffChangeUi.createHighlighter][ui-highlight], [LineHighlighterBuilder.done][draw-ignored], [getIgnoredColor][ignored-color]).
- Each Inline Change gets the full-strength colour of its own kind ([installHighlighter][ui-highlight]).
- This two-level contrast (soft block, strong words) is a large part of the "IntelliJ look".

**Default mode: synced scrolling, no padding.**
- `MySyncScrollable.processHelper` emits boundary pairs `(0,0)`, `(start1,start2)` and `(end1,end2)` for every change, then `(lineCount1,lineCount2)` ([SimpleDiffViewer][viewer-sync]).
- `transferLine` maps a line inside a span proportionally, clamped to the target span, and offsets it linearly after the span ([BaseSyncScrollable][sync-base]).
- `syncVerticalScroll` anchors at **one third of the viewport height**, converts in *visual* lines (soft wraps and folds), and keeps the sub-line phase so both Sides move smoothly ([SyncScrollSupport][sync-vertical]).
- The divider paints one curved trapezium (the Connector) per change between the Sides' y-ranges ([MyDividerPainter][viewer-sync], [DiffDividerDrawUtil.paintPolygons][divider-paint], [drawCurveTrapezium][curve]).

**Aligned mode** is opt-in through `ENABLE_ALIGNING_CHANGES_MODE` (default `false`), or forced per request ([needAlignChanges][aligned-need], [settings][settings-defaults]). For every change it adds block inlays at the start and end on both Sides ([initInlays][aligned-init]). `updateInlayHeights` then sets the heights from the difference in visual-line height ([updateInlayHeights][aligned-heights]):

- *top inlay:* the gap from the previous change;
- *bottom inlay:* the block body, plus any foreign inlays inside it.

Other inlays, such as inline hints, are mirrored onto the other Side. Sync scroll becomes "forced" (identical y) ([MySyncScrollable.forceSyncVerticalScroll][viewer-sync]).

---

## Crate mapping

| IntelliJ step | `imara-diff` 0.2.0 | `similar` 3.2.0 | We own |
|---|---|---|---|
| Line split on `\n`, CRLF handling | `sources::lines` (keeps the line terminator) | `diff_lines` (keeps the terminator) | CRLF normalisation; line table with offsets |
| Policy hashing (DEFAULT / TRIM / IW, +case) | intern any `T: Hash+Eq`, or pass raw `&[Token]` to `compute_with` ([lib.rs][imara-compute]) | `whitespace_mode`: `Exact`, `IgnoreChanges` (= git `-b`, collapses *internal* runs), `IgnoreAll` (= git `-w`) ([text/mod.rs][similar-ws]) | Our own `Line{hash, non_space}` per policy. `similar`'s `IgnoreChanges` is **not** IntelliJ TRIM, which ignores only leading and trailing whitespace. |
| Prefix/suffix strip + LCS | yes: Myers (gnu/git heuristics), `MyersMinimal`, Histogram ([Algorithm][imara-alg]) | yes: Myers, RawMyers, Patience, Histogram, Lcs, Hunt ([types.rs][similar-alg]) | — |
| Discard one-sided tokens (`Reindexer`) | yes, built into Myers preprocessing ([lib.rs][imara-alg]) | — | — |
| Cost cap / fallback (`DiffTooBig`) | internal heuristics; no deadline or cancellation | `deadline`/`timeout` ([text/mod.rs][similar-deadline]) | cancellation token between passes; `MAX_BAD_LINES` |
| Unimportant-line two-pass (`compareSmart`) | — | — | **own** (calls LCS twice) |
| Second-step policy correction | — | — | **own** |
| Slider / chunk optimizer | `postprocess_with(…, impl SliderHeuristic)`: pluggable; ships git's `IndentHeuristic` ([postprocess.rs][imara-post], [slider_heuristic.rs][imara-slider]) | fixed `Compact` pass in every `capture_diff` (merge and slide, no custom heuristic) ([common.rs][similar-compact], [compact.rs][similar-compact-doc]) | **own** the IntelliJ rule (empty-line, then ≤3-char boundary). Possibly hosted as an imara `SliderHeuristic`, but imara's postprocessor also merges hunks with its own rules, so test equivalence or port `ChunkOptimizer` directly (≈100 lines). |
| Word tokenizer (Unicode scripts, newline tokens) | — | `InlineChangeMode::{Words, UnicodeWords, Chars, Graphemes}` (different rules) | **own**. `char::is_alphabetic` (std) plus `unicode-script` for Han/Hiragana/Katakana/Thai/Javanese, or port the embedded tables. |
| Word LCS + `WordChunkOptimizer` | LCS via `compute_with` on word-hash tokens | LCS only | **own** the optimizer |
| `LineFragmentSplitter` | — | — | **own** |
| Punctuation matching | LCS on punctuation code points | — | **own** the gap logic |
| Whitespace correctors | — | — | **own** (`TrimUtil` ≈ 600 lines, mostly small loops) |
| Inline refinement as a whole | — | `iter_inline_changes_with_options`: ratio-gated (`min_ratio` 0.5), patience on word tokens across the whole replace op, optional `semantic_cleanup` ([inline.rs][similar-inline]) | A different design. Its ratio gate silently drops refinement where IntelliJ always refines; there is no punctuation or whitespace correction. Don't use it if the goal is IntelliJ parity. |
| Squash / trim post-process | — | — | **own** |
| Sync scroll, Connectors, aligned padding | — | — | **own** (UI layer) |

**Algorithm choice.** For IntelliJ fidelity, use `Algorithm::Myers`. imara's Myers ports git and gnu-diff heuristics; it is close to, but not identical with, IntelliJ's capped Myers. Avoid Histogram: imara's own docs say it is a poor fit for character and small-token diffs ([Algorithm docs][imara-alg]), and IntelliJ never uses it.

**Crate health.**
- `imara-diff` 0.2.0 was released 2025-06-14 and is the latest on crates.io.
- `gix-imara-diff` 0.3.0 (2026-09-25) describes itself as "a modified copy of upstream imara-diff for gitoxide". `gix-diff` 0.68 depends on it ([crates.io][gix-imara]).
- `similar` 3.2.0 was released 2026-08-17.
- Licences: imara-diff and gix-imara-diff are Apache-2.0; similar is Apache-2.0; unicode-script is MIT OR Apache-2.0.

**Other crates considered.**
- `dissimilar` 1.0.11 (Apache-2.0) is a diff-match-patch port at char level, with semantic cleanup. Its cleanup philosophy differs from IntelliJ's, so it is not needed if we port `ByCharRt`.
- `gix-diff`: useful only for git blob plumbing, which is ticket 03.

---

## License note

- **Governing licence for the source.** Every file read carries the header *"Copyright 2000-20xx JetBrains s.r.o. and contributors. Use of this source code is governed by the Apache 2.0 license."* (e.g. [ByLineRt.kt L1][byline-line1]).
- **Root `LICENSE.txt` is not the source licence.** At this commit it is the *"JetBrains Open-Source Build Terms" v1.3* (effective 2026-06-15). Those terms cover the *built IDE products*, and they state that the builds "consist of open source software subject to the Apache 2.0 license" ([LICENSE.txt][license]). Don't confuse the two: the per-file Apache-2.0 grant governs code we read or port.
- **`NOTICE.txt`** reads: "This software includes code from IntelliJ IDEA / Copyright (C) JetBrains s.r.o. / https://www.jetbrains.com/idea/" ([NOTICE.txt][notice]).

**What porting implies.** A line-by-line Kotlin→Rust translation of `ByLineRt`, `ChunkOptimizer`, `ByWordRt`, `TrimUtil` and the rest is a derivative work under Apache-2.0. Apache-2.0 §4 then requires us to:
- ship a copy of the Apache-2.0 licence with the distribution;
- keep JetBrains' copyright notices in the ported files;
- mark ported files as modified (a header line such as "Ported from IntelliJ Community `ByLineRt.kt` @bc34875, modified");
- carry the `NOTICE.txt` text into our own NOTICE.

**Project licence.** The ported files themselves stay Apache-2.0 only, so they can't be offered under MIT.
- The project's *own* code can still be MIT, Apache-2.0, or dual "MIT OR Apache-2.0", with the ported module under Apache-2.0.
- Apache-2.0 code can go into a GPLv3 project. It cannot go into a GPLv2-only project.

**Clean-room alternative.** Re-implementing from this document's prose description (the ideas, not the code) arguably avoids derivative-work status, since Apache-2.0 doesn't restrict ideas. That argument is weaker because this document's author has read the code. Given how many small heuristics there are, porting with attribution is the cheaper and more faithful route, and the attribution is light.

This is an engineering reading of the licence, not legal advice.

**Crates.** All candidate crates are Apache-2.0 or MIT/Apache, which is compatible.

**Patent grant.** Apache-2.0 §3 also gives a patent licence from contributors. That licence terminates if we sue over patents in the work, which is not a concern here.

---

## Open questions for the diff-engine design ticket (06)

- **Port shape:** faithful port of `ChunkOptimizer`, or an imara `SliderHeuristic`? Decide with a golden corpus comparing against IntelliJ output; see the map's "testing strategy" item.
- **Ignore-case:** IntelliJ has none. Add it as a fourth hashing and equality mode.
- **Performance:** IntelliJ diffs every policy in two LCS passes (IW, then the gap passes) plus per-block word LCS. Check the 100k-line < 300 ms budget against imara Myers with the extra passes. Refining word diffs lazily, only for visible blocks, would be our own optimisation; IntelliJ refines all blocks up front.

<!-- IntelliJ sources @ bc3487548ccaba36486be2d88f8fd379a1d36258 -->
[ij-head]: https://github.com/JetBrains/intellij-community/tree/bc3487548ccaba36486be2d88f8fd379a1d36258
[byline-java]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ByLine.java
[byword-java]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ByWord.java
[policy]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ComparisonPolicy.kt#L4-L8
[byline-line1]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L1
[byline-docompare]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L60-L82
[byline-second]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L135-L266
[byline-bestmatch]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L268-L319
[byline-smart]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L330-L348
[byline-biglines]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L350-L362
[byline-line]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L417-L455
[cu-equals]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ComparisonUtil.kt#L13-L37
[strings-kmp]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/base/multiplatform/src/com/intellij/openapi/util/text/StringsKmp.kt#L42-L118
[ws-pred]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/TrimUtil.kt#L584
[diffconfig]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/util/diff/DiffConfig.kt#L7-L13
[diff-build]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/util/diff/Diff.kt#L26-L102
[myers-threshold]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/util/diff/MyersLCS.kt#L9-L86
[patience]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/util/diff/PatienceIntLCS.kt#L37-L77
[patience-reduce]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/util/diff/PatienceIntLCS.kt#L153-L158
[reindexer]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/util/diff/Reindexer.kt#L15-L52
[diu-diff]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/iterables/DiffIterableUtil.kt#L24-L69
[diu-expand]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/iterables/DiffIterableUtil.kt#L304-L310
[cc-execute]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ChangeCorrector.kt#L27-L50
[cc-char]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ChangeCorrector.kt#L62-L91
[cc-smart]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ChangeCorrector.kt#L93-L126
[co-base]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ChunkOptimizer.kt#L19-L77
[co-word]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ChunkOptimizer.kt#L90-L163
[co-line]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ChunkOptimizer.kt#L165-L261
[co-line-getshift]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ChunkOptimizer.kt#L254-L259
[byword-split]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L88-L134
[byword-wordsfirst]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L166-L447
[byword-ws]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L488-L507
[byword-chunks]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L637-L683
[byword-punct]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L689-L874
[byword-wordchunk]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L1089-L1128
[bychar]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByCharRt.kt#L15-L89
[bychar-punct]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByCharRt.kt#L91-L134
[bychar-iw]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByCharRt.kt#L153-L214
[splitter]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/LineFragmentSplitter.kt#L10-L168
[trim-classes]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/TrimUtil.kt#L12-L46
[trim-expand-ignored]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/TrimUtil.kt#L375-L426
[charutils]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/CharacterUtils.kt#L20-L196
[lineoffsets]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/tools/util/text/LineOffsetsImpl.kt#L47-L66
[cmi-lines]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L73-L85
[cmi-inner]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L217-L278
[cmi-word]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L280-L312
[cmi-offsets]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L391-L432
[cmi-process]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L447-L544
[ignore-policy]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/base/IgnorePolicy.java#L11-L43
[highlight-policy]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/base/HighlightPolicy.java#L10-L46
[settings-defaults]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/base/TextDiffSettingsHolder.kt#L35-L63
[registry]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/resources/intellij.platform.diff.impl.xml#L40-L41
[provider-base]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/text/TwosideTextDiffProviderBase.java#L59-L82
[simple-provider]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/text/SimpleTextDiffProvider.java#L81-L117
[viewer-compute]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/simple/SimpleDiffViewer.java#L289-L312
[viewer-sync]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/simple/SimpleDiffViewer.java#L875-L917
[ui-highlight]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/simple/SimpleDiffChangeUi.java#L45-L99
[draw-ignored]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/util/DiffDrawUtil.java#L570-L590
[ignored-color]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/util/TextDiffTypeFactory.java#L64-L74
[sync-base]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/BaseSyncScrollable.java#L14-L97
[sync-vertical]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/SyncScrollSupport.java#L352-L413
[divider-paint]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/util/DiffDividerDrawUtil.java#L65-L84
[curve]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/util/DiffDrawUtil.java#L151
[aligned-need]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/simple/AlignedDiffModel.kt#L163-L169
[aligned-init]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/simple/AlignedDiffModel.kt#L206-L260
[aligned-heights]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/simple/AlignedDiffModel.kt#L328-L390
[license]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/LICENSE.txt
[notice]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/NOTICE.txt
<!-- Crates -->
[imara-alg]: https://docs.rs/crate/imara-diff/0.2.0/source/src/lib.rs
[imara-compute]: https://docs.rs/imara-diff/0.2.0/imara_diff/struct.Diff.html#method.compute_with
[imara-post]: https://docs.rs/crate/imara-diff/0.2.0/source/src/postprocess.rs
[imara-slider]: https://docs.rs/crate/imara-diff/0.2.0/source/src/slider_heuristic.rs
[similar-alg]: https://docs.rs/crate/similar/3.2.0/source/src/types.rs
[similar-ws]: https://docs.rs/crate/similar/3.2.0/source/src/text/mod.rs
[similar-deadline]: https://docs.rs/similar/3.2.0/similar/struct.TextDiffConfig.html#method.deadline
[similar-myers]: https://docs.rs/crate/similar/3.2.0/source/src/algorithms/myers.rs
[similar-inline]: https://docs.rs/crate/similar/3.2.0/source/src/text/inline.rs
[similar-compact]: https://docs.rs/crate/similar/3.2.0/source/src/common.rs
[similar-compact-doc]: https://docs.rs/crate/similar/3.2.0/source/src/algorithms/compact.rs
[gix-imara]: https://crates.io/crates/gix-imara-diff
