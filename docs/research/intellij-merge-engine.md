# How IntelliJ computes a 3-way merge

Research for ticket `02-intellij-merge-engine-research` (map: `.scratch/lenz-merge/map.md`). Terms follow `/CONTEXT.md` (Side, Change Block, Inline Change, Whitespace Mode). Words like "merge block" and "result" are informal here. Naming them is a job for the spec.

Source read: IntelliJ Community at commit [`bc34875`][ij-head], the same commit `lenz-engine` was ported from.
- The comparison layer is in `platform/util/diff` and `platform/diff-impl`, from the local sparse checkout.
- The merge viewer package, `platform/diff-impl/src/com/intellij/diff/merge/`, is not in that checkout. It was fetched from GitHub raw at the same SHA.
- The tests are in `platform/diff-impl/tests/testSrc/com/intellij/diff/{comparison,merge}`.

All of it is Apache-2.0.

Path shorthands used below:
- `U/` = `platform/util/diff/src/com/intellij/diff/`
- `D/` = `platform/diff-impl/src/com/intellij/diff/`
- `T/` = `platform/diff-impl/tests/testSrc/com/intellij/diff/`

---

## Summary

### The pipeline in steps

1. **Load.** The merge viewer reads the left, base and right texts, then sets the result document to a copy of base. Each merge block's result range starts out as its base line range ([MergeConflictModel.rediff][mcm-rediff], [buildResultLineRanges][mcm-ranges]).
2. **Line merge = two 2-way line diffs zipped on base.**
   - `ComparisonManager.mergeLines(left, base, right, policy)` → `ByLineRt.merge` runs the existing 2-way line pipeline twice: base→left and base→right. That pipeline is IW-first `compareSmart`, then `optimizeLineChunks`, then `correctChangesSecondStep` ([ByLineRt.doCompare(3)][byline-do3]).
   - `ComparisonMergeUtil.FairMergeBuilder` then walks the two lists of unchanged runs together. It intersects them on base. Every gap between runs that are unchanged in all three becomes one `MergeRange(left, base, right)` ([ComparisonMergeUtil][cmu]).
3. **Whitespace-only blocks stay separate.** Under Trim or Ignore, `mergeLines` (`keepIgnoredChanges = true`) uses `IgnoringChangeBuilder`. It re-checks every "unchanged" line triple with exact equality and emits runs that differ only in whitespace as their own blocks. They are never folded into a conflict. `compareLines(3)` is the same code without that step and serves the read-only 3-side diff viewer.
4. **Each block gets a type.** `MergeRangeUtil.getLineMergeType` is a pure function of:
   - which of the three ranges are empty,
   - pairwise line-by-line equality under the policy,
   - exact equality, which decides which Side counts as changed when all three are policy-equal.

   It yields INSERTED / DELETED / MODIFIED / CONFLICT, a `leftChanged` / `rightChanged` pair, and for conflicts a resolution strategy ([getMergeType][mru-type]).
5. **Each conflict is test-resolved at load time.** A CONFLICT is marked resolvable (strategy `TEXT`) only when all of these hold:
   - base is non-empty,
   - left and right are both non-empty,
   - `ComparisonMergeUtil.tryResolveConflict(leftLines, baseLines, rightLines)` returns non-null ([canResolveLineConflict][mru-can]).

   Insert/insert and delete/modify conflicts are therefore never resolvable.
6. **The resolver ("magic wand") is `MergeResolveUtil.tryResolve`.**
   - It runs a 3-way **word** merge of the three block texts under DEFAULT. If any word-level conflict is left it fails, and the whole thing is retried under IGNORE_WHITESPACES.
   - Otherwise it rebuilds the text: base for unchanged gaps, the changed Side for one-sided word changes ([SimpleHelper][mrs-simple]).
   - A "greedy" variant exists but is switched off: `DiffConfig.USE_GREEDY_MERGE_MAGIC_RESOLVE = false` ([DiffConfig][diffconfig]).
7. **Inline highlighting of a conflict is *not* the 3-way word merge.** `DiffUtil.compareThreesideInner` runs two plain **2-way** `ByWord.compare` calls, base→left and base→right. Sides that are unchanged, already resolved or empty are left out ([compareThreesideInner][du-inner], [InnerChunkData][mtv-chunk]). The 3-way `ByWordRt.compare` and its three `Merge*Corrector`s are used only by the resolver and by tests.
8. **Actions.**
   - **Apply non-conflicting (left / all / right)** copies the chosen Side's lines into every non-conflict that changed on that Side. "All" (BASE) takes left when left changed, else right ([resolveChangeAutomatically][mcm-resolve]).
   - **Resolve simple conflicts** (the magic wand) runs the same routine with BASE over *every* change. It applies all non-conflicts and every TEXT-resolvable conflict ([applyResolvableConflictedChanges][mtv-wand]).
   - Both skip any block whose result text no longer equals base ([canResolveChangeAutomatically][mcm-can]).
   - Auto-applying non-conflicts on open is a setting that is **off by default** in IntelliJ (`MERGE_AUTO_APPLY_NON_CONFLICTED_CHANGES = false`, [TextDiffSettingsHolder][settings]). Lenz's map turns it on, which is a deliberate divergence.
9. **Accepting one Side of a conflict.** The first accept replaces the result range with that Side and marks only that Side done. Accepting the other Side then *appends* its lines, so "both" comes out in click order. If the other Side is empty, or the user used the "resolve" modifier, the block is resolved at once ([replaceChange][mcm-replace], [MergeModelBase.appendChange][mmb-append]).

### What it means for `lenz-engine`

- About **1.1k lines of Kotlin** form the pure engine part. They sit directly on code we already ported: the 2-way `ByLineRt` pipeline, `ByWordRt` tokens, the word chunk optimizer, punctuation matching, and the TrimUtil expand/trim helpers. The new pieces are:
  - the 3-way zip (`ComparisonMergeUtil`),
  - conflict typing (`MergeRangeUtil`),
  - the 3-way word merge and its whitespace correctors,
  - the simple resolver.
- About **0.4k more lines** of the viewer model are worth porting as *logic*, not code: initial result, apply non-conflicting, accept / append, wand eligibility, and line-range shifting. Lenz only edits whole blocks, so IntelliJ's document listener, undo states and PSI/import/semantic/AI resolution all drop out.
- **About 85–100 IntelliJ golden cases** convert mechanically, like the 2-way ones. Eight `MergeTest` functions (about 30 scenarios) are worth hand-porting as model tests. See [Tests](#4-tests-and-whether-they-convert).

---

## 1. Entry points

| Entry | Where | What it does |
|---|---|---|
| `ComparisonManager.mergeLines(t1, t2, t3, policy, indicator)` | [`D/comparison/ComparisonManagerImpl.java`][cmi-merge] (+ the [`ComparisonManager`][cm-api] API) | Splits the three texts into lines, calls `ByLine.merge` → `ByLineRt.merge`, and wraps each `MergeRange` as a `MergeLineFragment`. **This is what the merge viewer calls** ([MergeDiffBuilder][mdb-merge]). |
| `ComparisonManager.compareLines(t1, t2, t3, …)` | same | The same code with `keepIgnoredChanges = false`, used by the read-only 3-side diff viewer (`SimpleThreesideDiffViewer`). Not needed for merging. |
| `mergeLinesWithinRange(…, boundaryRange)` | same | `mergeLines` restricted to a sub-range and shifted back. Used only to merge outside an imports block (`MergeImportUtil`). Skip. |
| `mergeLinesAdditions(t1, t3)` | same | Builds a fake base from the common lines of two texts. Not part of the merge viewer. Skip. |
| `ByLineRt.merge` / `ByLineRt.compare(l1, l2, l3)` | [`U/comparison/ByLineRt.kt`][byline-merge] | Line-level 3-way, returning `List<MergeRange>`. |
| `ByWordRt.compare(t1, t2, t3, policy)` | [`U/comparison/ByWordRt.kt`][byword-3] | Word-level 3-way, returning `List<MergeWordFragment>`. Used by `MergeResolveUtil` and tests only. |
| `ComparisonMergeUtil.buildSimple / buildMerge / tryResolveConflict` | [`U/comparison/ComparisonMergeUtil.kt`][cmu] | Zips two fair 2-way iterables into merge ranges, and the resolver dispatch. |
| `MergeRangeUtil.getLineMergeType / getWordMergeType` | [`U/util/MergeRangeUtil.kt`][mru-type] | Conflict typing. |
| `MergeResolveUtil.tryResolve / tryGreedyResolve` | [`U/comparison/MergeResolveUtil.kt`][mrs] | The "resolve simple conflicts" text resolver. |
| `DiffUtil.compareThreesideInner` | [`D/util/DiffUtil.java#L1129`][du-inner] | Inline Changes for one merge block. |

Inputs and outputs follow IntelliJ's numbering everywhere: **1 = left, 2 = base, 3 = right** (`MergeRange.start1/end1 … start3/end3`, `ThreeSide.LEFT/BASE/RIGHT`). The two 2-way iterables passed to the zip are **base→left** and **base→right**, so in each of them "Side 1" is base.

## 2. Classes, one by one

### Data types: `MergeRange`, `MergeLineFragment(Impl)`, `MergeWordFragment(Impl)`, `MergeConflictType`, `MergeConflictResolutionStrategy`

[`U/util/MergeRange.kt`][mr] (46 lines), [`U/fragments/MergeLineFragment*.kt`][mlf] (10 + 30), [`U/fragments/MergeWordFragment*.kt`][mwf] (10 + 26), [`U/util/MergeConflictType.kt`][mct] (44), [`U/util/MergeConflictResolutionStrategy.kt`][mcrs] (13).

- `MergeRange` holds six ints, three half-open ranges. Lines for line merges, chars for word merges.
- `MergeConflictType` holds:
  - `type`: INSERTED, DELETED, MODIFIED or CONFLICT.
  - `isChange(LEFT/RIGHT)`: whether that Side differs from base. `isChange(BASE)` is always true.
  - `resolutionStrategy`. For a non-conflict this is `DEFAULT`. For a conflict it is `TEXT`, `SEMANTIC` (language-aware, PSI) or `null`, meaning not resolvable. `canBeResolved() = strategy != null`.

**Rust:** one `MergeBlock` struct and one `MergeKind` enum (see [API](#rust-api-sketch)). About 40 lines.

### `ComparisonMergeUtil`: the 3-way zip (189 lines)

[`U/comparison/ComparisonMergeUtil.kt`][cmu]

- **Inputs:** two *fair* 2-way iterables, base→left and base→right, both over the same base length. "Fair" means each unchanged run has equal length on both Sides.
- **Output:** `List<MergeRange>`, in order, non-empty.
- **`FairMergeBuilder.execute`:**
  - Keep a cursor into each list of unchanged runs.
  - At each step, intersect the two current runs on base: `[max(start), min(end))`.
  - If the intersection is non-empty, map it back to left and right offsets (each run is fair, so the shift is the same) and call `markEqual(left, base, right)`.
  - Advance the run that ends first on base (the left one on ties).
  - At the end, `finish(lenLeft, lenBase, lenRight)`.
- **`ChangeBuilder`:** whatever lies between two consecutive `markEqual` triples (or the ends) becomes a `MergeRange`, unless all three parts are empty. The algorithm is diff3's "stable/unstable chunk" idea, done on IntelliJ's own 2-way diffs.
- **`IgnoringChangeBuilder`** (`buildMerge`, used when `keepIgnoredChanges && policy != DEFAULT`):
  - Before emitting each gap, scan the equal region since the last emitted range, line triple by line triple, with a `trueEquality(left, base, right)` callback.
  - For lines, that callback is "base == left and base == right under DEFAULT" (`ByLineRt.equalsDefaultPolicy`).
  - Each maximal run of triples that fail it becomes its own range. Such a range is equal under the policy but not exactly equal: a whitespace-only change.
- **`tryResolveConflict(left, base, right)`:** calls `MergeResolveUtil.tryResolve`, or `tryGreedyResolve` if the compile-time flag is set (it isn't).
- **Depends on:** nothing new. Lenz's `by_line` already produces change lists and has `unchanged()` to turn them into fair unchanged runs.
- **Rust:** about 90 lines.

### `ByLineRt` 3-way: `compare(l1, l2, l3)`, `merge(l1, l2, l3)`, `doCompare(…, keepIgnoredChanges)` (about 90 of 456 lines)

[`U/comparison/ByLineRt.kt#L33-L133`][byline-merge]

```
iw = lines converted to IGNORE_WHITESPACES keys (for all three)
iwChanges1 = compareSmart(iwBase, iwLeft);  optimizeLineChunks(base, left, …);  it1 = correctChangesSecondStep(base, left, …)
iwChanges2 = compareSmart(iwBase, iwRight); optimizeLineChunks(base, right, …); it2 = correctChangesSecondStep(base, right, …)
if keepIgnoredChanges && policy != DEFAULT: buildMerge(it1, it2, equalsDefaultPolicy) else buildSimple(it1, it2)
```

- `Line` objects carry the user's policy (DEFAULT/TRIM/IGNORE) for hashing and equality, exactly as in 2-way.
- **The trap for the port:** the 3-way path goes through `correctChangesSecondStep` under **every** policy, IGNORE_WHITESPACES included. The 2-way `doCompare` takes a different branch for IGNORE: `compareSmart` on IW lines, then `expandRanges`. Lenz's `by_line::compare` mirrors the 2-way branch. So the 3-way port needs a variant that always runs the second step, with the IW keys as the "policy" keys when the mode is Ignore. Port it literally. The `testIgnoreWhitespaces` cases below pin it down.
- There is no ignore-case in merging. IntelliJ's `ComparisonPolicy` has no case flag, so prepare keys with `fold_case = false`.
- **Rust:** about 50 lines in `by_line.rs`, plus about 10 to expose a "fair unchanged runs" result from the existing 2-way code. Base→left and base→right are independent, so two separate `by_line::prepare` calls are enough. No three-way interning table is needed.

### `MergeRangeUtil`: conflict typing (249 lines, about 190 relevant)

[`U/util/MergeRangeUtil.kt`][mru-type]

`getMergeType(emptiness, equality, trueEquality, conflictResolver)` is a decision table (L = left, B = base, R = right):

| B empty? | L / R | Result |
|---|---|---|
| yes | L empty, R not | INSERTED, right changed |
| yes | R empty, L not | INSERTED, left changed |
| yes | both non-empty, L == R under policy | INSERTED, both changed |
| yes | both non-empty, L != R | CONFLICT, **never resolvable** (insert/insert order is unknowable) |
| no | both empty | DELETED, both changed |
| no | B == L and B == R under policy (a whitespace-only block) | MODIFIED; changed Sides = those that differ from base *exactly* (`trueEquality`, DEFAULT) |
| no | B == L only | right changed: DELETED if R empty, else MODIFIED |
| no | B == R only | left changed: DELETED if L empty, else MODIFIED |
| no | L == R under policy | MODIFIED, both changed |
| no | otherwise | CONFLICT; strategy TEXT iff L and R are both non-empty **and** `tryResolveConflict(L, B, R) != null`, else null |

- **Line equality** (`compareLineMergeContents`) requires equal line counts, then compares line by line with `ComparisonUtil.isEqualTexts(policy)`. That function is the same DEFAULT / trim-each-line / strip-all-whitespace test as Lenz's `equal_under`.
- **Block text for the resolver** comes from `getLinesContent`: the lines joined with their newlines, **without** the last newline.
- `getWordMergeType` is the same table on char ranges, with no `trueEquality` and a resolver that always says no. `MergeResolveUtil` uses it.
- Skip `getLineLeftToRightThreeSideDiffType`. It serves the 3-side diff viewer's left-to-right mode.
- **Rust:** about 100 lines. The callbacks become a tiny trait or two closures.

### `ByWordRt` 3-way + `Merge{Default,Trim,Ignore}SpacesCorrector` + TrimUtil 3-way (about 150 + 155 lines)

[`U/comparison/ByWordRt.kt#L61`][byword-3], [correctors][byword-mcorr], [`U/comparison/TrimUtil.kt`][trimutil]

```
words = getInlineChunks(each text)
it1 = matchAdjustmentDelimiters(base, left,  optimizeWordChunks(diff(wordsBase, wordsLeft)))
it2 = matchAdjustmentDelimiters(base, right, optimizeWordChunks(diff(wordsBase, wordsRight)))
conflicts = ComparisonMergeUtil.buildSimple(it1, it2)
result = matchAdjustmentWhitespaces(left, base, right, conflicts, policy)
```

The last step depends on the policy:
- **DEFAULT** (`MergeDefaultCorrector`): shrink each range by the whitespace that is identical in all three at its ends (`expandWhitespacesForward/Backward` 3-way). Drop ranges that become empty.
- **TRIM** (`MergeTrimSpacesCorrector`): DEFAULT first, then trim leading/trailing whitespace on each Side where it touches a line edge. Drop ranges that become equal.
- **IGNORE** (`MergeIgnoreSpacesCorrector`): expand over whitespace matching all three, trim, and drop ranges equal ignoring whitespace.

The TrimUtil 3-way helpers are `trim`, `expandWhitespaces*` and `isEquals*` over three texts, plus the private `expandIgnored*`. They are straight 3-way generalisations of the 2-way helpers in `by_word.rs`.

- **Depends on:** everything already in `by_word.rs` (`inline_chunks`, `optimize_word_chunks`, `match_adjustment_delimiters`, the 2-way `expand_ws_*` / `trim_*`) and `lcs::diff`.
- **Note:** `inline_chunks` currently interns two texts at once. The 3-way version needs base's ids to be shared with each other Side, so either call it twice (base+left, base+right) or widen it to N texts.
- **Rust:** about 150 lines.

### `MergeResolveUtil`: "resolve simple conflicts" (280 lines; about 130 without greedy)

[`U/comparison/MergeResolveUtil.kt`][mrs]

- **`tryResolve(left, base, right)`:** `SimpleHelper` with DEFAULT, then with IGNORE_WHITESPACES if that failed. `DiffTooBigException` → null.
- **`SimpleHelper.execute(policy)`** ([L82][mrs-simple]):
  1. `changes = ByWordRt.compare(left, base, right, policy)`.
  2. Walk the word merge ranges and the gaps between them.
  3. **Gap** (unchanged under the policy): if base == left == right *exactly*, append base. Otherwise it is a whitespace-only difference: append left if the DEFAULT word type says left changed, else right if right changed, else base.
  4. **Range:** compute `getWordMergeType` under the policy. CONFLICT → **return null**. Otherwise append left if left changed, else right. When both changed identically, left's text wins.
  5. Append the trailing gap. Return the new text.
- In words: the conflict is resolvable iff the two Sides' word-level edits never overlap, under exact whitespace or failing that ignoring whitespace. Whitespace edits on the unchanged parts are kept.
- **`GreedyHelper`** ([L175][mrs-greedy]): two 2-way word diffs. Overlapping deletions merge, insertions apply, and only differing insertions at the same spot fail. It resolves more, and less safely. It is **off in production**: `DiffConfig.USE_GREEDY_MERGE_MAGIC_RESOLVE = false`. Skip it, and record that.
- **Used by:**
  - `MergeRangeUtil.canResolveLineConflict`, at load, to set strategy TEXT;
  - `TextAutoResolver`, when the wand fires. It runs on (left lines, current result lines, right lines) for the block. Because of the eligibility rule below, the result lines still equal base at that point.
- **Output:** a string that replaces the block's result lines via `LineTokenizer.tokenize(text, false)`. An empty result gives **zero lines** (the block is deleted from the result), and a trailing newline gives no extra empty line.
- **Rust:** about 80 lines.

### `DiffUtil.compareThreesideInner`: Inline Changes in a merge block (about 70 lines Java)

[`D/util/DiffUtil.java#L1129`][du-inner]

- **Input:** three optional block texts. A Side is `null` when it is unchanged, already resolved, or empty ([`InnerChunkData`][mtv-chunk]). In the viewer, the BASE text is the *result* document's current lines for the block.
- **Policy IGNORE, all three equal ignoring whitespace:** empty lists. This is a whitespace-only block.
- **Two or more Sides null:** no inline highlighting.
- **All three present:** `ByWord.compare(base, left)` and `ByWord.compare(base, right)`. Base collects ranges from both.
- **One Side null:** one 2-way `ByWord.compare` between the remaining two.
- **Depends on:** only the existing 2-way `by_word::compare`.
- **Rust:** about 40 lines.
- The merge viewer offers no character-level highlighting: `HighlightPolicy` is BY_LINE or BY_WORD only ([viewer settings][mtv-policies]).

### Viewer model: `MergeConflictModel`, `MergeModelBase`, `TextMergeChange`, `MergeDiffBuilder`, `MergeThreesideViewer`

| File | Lines | Port? |
|---|---|---|
| [`D/merge/MergeDiffBuilder.kt`][mdb] | 151 | **Logic only:** `mergeLines` → `getLineMergeType` per block. The rest is PSI, imports and semantic patching. |
| [`D/merge/MergeConflictModel.kt`][mcm] | 552 | **Logic only**, about 150 lines matter: `initWithData`, `resolveChangeAutomatically`, `canResolveChangeAutomatically`, `replaceChange`, `ignoreChange`, `markChangeResolved`, `hasNonConflictedChanges`, `hasAutoResolvableConflictedChanges`, `TextAutoResolver`, `isChangeRangeModified`. |
| [`D/merge/MergeModelBase.kt`][mmb] | 402 | **Logic only:** the result line range per block, `replaceChange`, `appendChange`, `moveChangesAfterInsertion`. Skip the document listener (`processDocumentChange`) and the undo-state machinery. They exist because the result is a free-text editor. |
| [`D/merge/TextMergeChange.kt`][tmc] | 127 | **Logic only:** per-Side `resolved[2]` and `isOnesideAppliedConflict`. Skip the AI flag. |
| [`D/tools/simple/ThreesideDiffChangeBase.kt`][tdcb] | 29 | `isConflict` and `isChange(side)` helpers. |
| [`D/merge/MergeThreesideViewer.java`][mtv] | 1263 | UI. Read for behaviour only: `apply()` at L614-L673, `applyNonConflictedChanges` at L972, `applyResolvableConflictedChanges` at L982, the inner-diff worker at L790-L835. |
| `ApplyNonConflictsAction.kt` / `MagicResolvedConflictsAction.kt` / `MergeThreesideViewerActions.kt` | 37 / 23 / 221 | Enabled-state rules (below). |
| `LangSpecificMergeConflictResolver*.kt`, `MergeImportUtil.kt`, `ChangeReferenceProcessor.kt`, `IterativeResolveData.kt` | 212 / 161 / 139 / 21 | **Skip.** PSI-based semantic resolution, import blocks, reference transfer, iterative multi-file resolve. |

The behaviour to reproduce, extracted from those files:

- **Initial state.**
  - result = base text.
  - `range[i]` = `blocks[i].base`.
  - Each block is unresolved on both Sides, with `onesideApplied = false`.
  - If auto-apply is on, `applyNonConflictedChanges(BASE)` runs once ([L669][mtv-auto]).
- **`canResolveChangeAutomatically(i, side)`** ([L365][mcm-can]):
  - Never during a modify/delete *file* conflict (one Side deleted the file).
  - A **conflict** qualifies iff `side == BASE`, the strategy is not null, neither Side is resolved, and (for TEXT) result text == base text.
  - A **non-conflict** qualifies iff it is unresolved, `isChange(side)` holds (always true for BASE), and result text == base text.
  - In block-only Lenz, "result text == base text" is the same as "nothing applied to this block yet".
- **`resolveChangeAutomatically(i, side)`** ([L211][mcm-resolve]):
  - A conflict with TEXT strategy gets `tryResolve(L, resultLines, R)` written into its range, then is marked resolved.
  - A non-conflict gets `replaceChange(i, master, resolve = false)`, where master = LEFT for side LEFT, RIGHT for RIGHT, and for BASE it is LEFT if left changed, else RIGHT.
- **Apply non-conflicting (LEFT / BASE / RIGHT)** = `resolveChangeAutomatically(side)` for every `!isConflict` block. The button is enabled iff some non-conflict qualifies for that Side.
- **Resolve simple conflicts (wand)** = `resolveChangeAutomatically(BASE)` for **every** block, conflicts and non-conflicts alike. It is enabled iff any block qualifies for BASE, despite the name `hasAutoResolvableConflictedChanges`.
- **`replaceChange(i, side, resolveChange)`** ([L282][mcm-replace]):
  - Already resolved on that Side → no-op.
  - `!isChange(side)` (the Side equals base) → just mark the block resolved.
  - **Non-conflict:** replace the result range with that Side's lines and mark resolved.
  - **Conflict:** if `onesideApplied`, *append* the Side's lines after the current result range; otherwise *replace*. Then, if `resolveChange` is set or the opposite Side is empty, mark fully resolved. Otherwise mark only this Side and set `onesideApplied`.
- **`ignoreChange(i, side, resolveChange)`:** a non-conflict, or `resolveChange` set → mark resolved and keep the current result. Otherwise mark only that Side resolved.
- **Line bookkeeping** (`MergeModelBase`):
  - Replacing range `i` with n lines shifts every later range by `n - old_len`.
  - Inserting into an *empty* range (or appending) runs `moveChangesAfterInsertion`. Neighbouring ranges that touch the insertion point are pushed outside it, so ranges stay ordered and the new lines belong to block `i`.
- **Auto-resolution of deletions:** a DELETED block whose result range becomes empty is marked resolved (`processDocumentChange` override, L460).

## 3. Whitespace policies in merge

- **Modes offered.** The merge viewer offers `IgnorePolicy` DEFAULT, TRIM_WHITESPACES and IGNORE_WHITESPACES only. There is no "ignore + trim chunks", no formatting mode and no ignore-case ([L170][mtv-policies]). `IgnorePolicy.getComparisonPolicy()` maps them to `ComparisonPolicy`.
- **What Trim/Ignore changes:**
  1. Line matching: both 2-way passes use the policy keys.
  2. Blocks equal under the policy but not exactly equal are kept as separate MODIFIED blocks (`IgnoringChangeBuilder`). Their changed Sides come from exact equality. They are never merged into a neighbouring conflict.
  3. Conflict typing compares under the policy, so `x` vs ` x ` on the two Sides counts as "equal modifications" under Trim.
  4. The word-level correctors used by the resolver, and the inline highlighting, follow the policy.
- **Consequence for "apply non-conflicting":** under Trim/Ignore it also applies whitespace-only blocks, taking left's whitespace when both changed. `MergeTest.testIgnored` pins this down.
- **The resolver ignores the viewer's policy.** It always tries DEFAULT, then IGNORE_WHITESPACES.
- **Changing the policy** re-runs the whole merge and **resets the result to base** (`rediff` → `setInitialOutputContent`, [L86][mcm-rediff]). IntelliJ asks for confirmation when the result is modified. Lenz will need the same rule or will need to forbid it.

## 4. Tests and whether they convert

| Test file | Size | Cases | What it calls | Convertible? |
|---|---|---|---|---|
| [`T/comparison/LineComparisonMergeUtilTest.kt`][t-line] | 144 | 16 `lines_merge` + 4 `lines_diff` | `MANAGER.mergeLines` / `compareLines(3)` under DEFAULT and IGNORE | **Yes, mechanically.** The DSL is `("L" - "B" - "R")`, `.matching()` with three patterns, `changes(mod(l, b, r, cl, cb, cr))`, `test(policy)`. This is a three-string variant of `intellij_lines/convert.py`: a triple regex and a 6-int `mod`. Drop the 4 `lines_diff` cases unless `compare_lines3` is ported (it is a flag away). |
| [`T/comparison/CharComparisonMergeUtilTest.kt`][t-char] | 185 | 26 `chars` blocks (DEFAULT only) | two 2-way `ByCharRt.compare` → `buildSimple` | **Yes.** It is a direct test of the zip (`build_simple`), using the existing `by_char::compare`. Cheap and valuable. `checkConsistency` (ordered, non-empty, non-squashed) ports as a few asserts. |
| [`T/comparison/WordMergeComparisonUtilTest.kt`][t-word] | 107 | 13 `words` blocks × `testAll` = up to 39 runs | 3-way `ByWordRt.compare` | **Yes.** It uses the `ComparisonUtilTestBase` `words {}` DSL already handled by `intellij_inner/convert.py`, extended to three strings and three matchings. Note the `// TODO` case in `testNewlines`: its expectation encodes current behaviour, not intent. Offsets are UTF-16 in IntelliJ, but all of these cases are ASCII. |
| [`T/comparison/MergeResolveUtilTest.kt`][t-resolve] | 297 | 16 `test(…)` (simple + greedy, one with separate expectations) + 1 `testSimple` + 9 `testGreedy` | `MergeResolveUtil.tryResolve` / `tryGreedyResolve` | **Yes, 17 cases** for the simple resolver. Watch out for three things. **Argument order is `(base, left, right, expected)`**. `testRegressions` uses Kotlin raw `"""` strings that span many lines, so the converter needs a raw-string branch. `null` means "can't resolve". Drop the 9 greedy-only cases if greedy is skipped. |
| [`T/comparison/ComparisonMergeUtilTestBase.kt`][t-base] | 225 | (DSL) | — | Defines `mod`, matching and consistency checks. Read it when writing the converter. |
| [`T/comparison/ComparisonUtilAutoTest.kt`][t-auto] `testMerge` / `testThreeWay` | 3-way part ≈ 250 | random | `mergeLines` + `ByWordRt.compare(3)` | **As a property test.** Its invariants port without IntelliJ's RNG: `checkLineConsistency3`, `checkValidRanges3` (no block has all three Sides policy-equal), `checkCantTrimLines3` and `checkDiffConsistency3`. Use a tiny xorshift over a small alphabet, as the 2-way corpus tests do. |
| [`T/merge/MergeTest.kt`][t-merge] | 1099 | 27 functions, 93 `doTest*` | the real viewer (`HeavyDiffTestCase`, editors, undo) | **Partly, by hand, as model tests.** Worth porting: `testChangeTypes` (types and changed Sides for 15 small triples), `testLastLine`, `testModifications`, `testModificationsModifiers`, `testResolve` (the parts without `replaceText`), `testNonConflictsActions` (apply non-conflicting LEFT/BASE/RIGHT on a realistic text), `testIgnored` (ranges, types and apply under IGNORE), `testApplyModifiedDeletedConflict`. Skip everything that types into the result, tests undo through the editor, or tests UI data (`testRangeModification`, `testUndo*`, `testError*`, `testUiDataSnapshot…`, `testInvalidatingChange`). The DSL arguments are `(left, base, right)`. |
| [`T/merge/MergeAutoTest.kt`][t-mauto] | 130 | random | random apply / ignore / resolve / edit + undo | **Skip.** It fuzzes editor undo. A block-only model can do its own small fuzz: apply random actions, check ranges stay ordered, undo restores the snapshot. |

**Total for mechanical conversion:** 16 + 26 + up to 39 + 17 ≈ **85–100 cases**, from about 730 lines of Kotlin test source. Add about 30 hand-ported `MergeTest` scenarios. A single `tests/intellij_merge/convert.py` can handle the three DSL files. `MergeResolveUtilTest` is easier to convert with its own small regex block in the same script. The files are Apache-2.0, so the provenance header goes in as in `intellij_lines/cases.rs`.

## 5. Port plan for `lenz-engine`

Order matters: each step is testable on its own.

1. **Zip** (`merge3.rs`): `build_simple(base_left: &[R], base_right: &[R], lens) -> Vec<MergeRange>` and `build_merge(…, true_eq)`.
   - `R` here are *fair unchanged runs*, as returned by the existing `by_line::unchanged`.
   - **Tests:** `CharComparisonMergeUtilTest` through `by_char::compare`.
2. **Line merge:** `by_line::compare_fair(prepared, policy)`. This is the 3-way branch that always runs `correct_changes_second_step`. Then `merge_lines` builds two `Prepared` (base+left, base+right), zips them, and types each block.
   - **Tests:** `LineComparisonMergeUtilTest`, then `MergeTest.testChangeTypes` / `testLastLine` / `testIgnored` (types and ranges).
3. **Typing** (`merge_type.rs`): the decision table, with line equality via the existing `equal_under` logic (per line; no case folding).
4. **3-way word merge** in `by_word.rs`: `compare3(left, base, right, policy) -> Vec<MergeRange>` with the three merge correctors and the TrimUtil 3-way helpers.
   - **Tests:** `WordMergeComparisonUtilTest`.
5. **Resolver:** `resolve_simple(left, base, right) -> Option<String>` (SimpleHelper, DEFAULT then IGNORE). Wire it into typing (`resolvable`).
   - **Tests:** `MergeResolveUtilTest` (17 cases), `MergeTest.testResolve`.
6. **Inline Changes** for a merge block: `merge_inline(…)` over the existing `by_word::compare`. Since there is no character level, `HighlightLevel::Char` maps to Word or is hidden in merge mode.
7. **Result model:** a pure, UI-free `MergeResult` holding the result lines, per-block result ranges and per-Side resolved flags. It provides apply non-conflicting, wand, accept/append, ignore and eligibility, and is `Clone` for snapshot undo.
   - **Tests:** the hand-ported `MergeTest` scenarios and a tiny fuzz.
   - **Where it lives:** the map wants the same machinery for moving Change Blocks in 2-way diffs. So put it next to the engine, either in a `merge` module of `lenz-engine` or in its own small crate. Keep it out of `crates/app`. Decide at spec time.

**Estimated Rust:** about 600–700 lines of engine (steps 1–6) + about 250 lines of model (step 7) + converters and tests. That is roughly half the size of the 2-way port.

### Rust API sketch

```rust
/// Whitespace handling for a merge. `IgnoreTrimChunks` is treated as `Ignore`; there is no ignore-case in merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MergeOptions {
    pub whitespace: WhitespaceMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind { Inserted, Deleted, Modified, Conflict }

/// One run of lines where left, base and right are not all equal (IntelliJ's MergeLineFragment + MergeConflictType).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeBlock {
    pub left: Range<u32>,
    pub base: Range<u32>,
    pub right: Range<u32>,
    pub kind: MergeKind,
    /// The Side differs from base (exactly, for whitespace-only blocks).
    pub left_changed: bool,
    pub right_changed: bool,
    /// A `Conflict` that `resolve_simple` resolves (IntelliJ's TEXT strategy). Always false otherwise.
    pub resolvable: bool,
}

impl MergeBlock {
    /// Equal under the Whitespace Mode on all Sides that changed: shown faded, like an Ignored Change.
    pub fn whitespace_only(&self) -> bool;
}

/// ComparisonManager.mergeLines + MergeRangeUtil.getLineMergeType. Lines as from `split_lines`.
pub fn merge_lines(left: &[&str], base: &[&str], right: &[&str], options: &MergeOptions) -> Vec<MergeBlock>;

/// MergeResolveUtil.tryResolve: the block texts (lines joined with "\n", no trailing newline).
/// `Some(text)` when the Sides' word-level edits don't overlap; `None` otherwise.
pub fn resolve_simple(left: &str, base: &str, right: &str) -> Option<String>;

/// DiffUtil.compareThreesideInner. `None` for a Side that is unchanged, resolved or empty.
/// Ranges are byte ranges into each block text, as for `InlineChange`.
pub struct MergeInline { pub left: Vec<Range<usize>>, pub base: Vec<Range<usize>>, pub right: Vec<Range<usize>> }
pub fn merge_inline(left: Option<&str>, base: Option<&str>, right: Option<&str>, options: &MergeOptions) -> Option<MergeInline>;

// ---- result model (block-level editing only; Clone = undo snapshot) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick { Left, Right }

/// Owns copies of left, base and right (the Sides never change), so `Clone` is a complete undo snapshot.
#[derive(Debug, Clone)]
pub struct MergeResult { /* left/base/right lines, result lines, per-block result range, per-Side resolved, one_side_applied */ }

impl MergeResult {
    /// Result = base; every block unresolved.
    pub fn new(left: &[&str], base: &[&str], right: &[&str], blocks: Vec<MergeBlock>) -> Self;
    /// `None` = "all" (IntelliJ's BASE: left if left changed, else right). Returns whether anything changed.
    pub fn apply_non_conflicting(&mut self, from: Option<Pick>) -> bool;
    /// The magic wand: every non-conflict plus every resolvable conflict not yet touched.
    pub fn resolve_simple_conflicts(&mut self) -> bool;
    /// replaceChange: replace (or, for the second Side of a conflict, append); `whole` resolves the block at once.
    pub fn accept(&mut self, block: usize, side: Pick, whole: bool);
    /// ignoreChange.
    pub fn ignore(&mut self, block: usize, side: Pick, whole: bool);
    pub fn can_apply_non_conflicting(&self, from: Option<Pick>) -> bool;
    pub fn can_resolve_simple_conflicts(&self) -> bool;
    pub fn result_range(&self, block: usize) -> Range<u32>;
    pub fn is_resolved(&self, block: usize) -> (bool, bool);
    pub fn unresolved_conflicts(&self) -> usize;
    pub fn lines(&self) -> &[String];
}
```

## 6. What to skip

- **`GreedyHelper` / `tryGreedyResolve`.** Off in production, and its 9 greedy-only tests go with it. Note this in the port header.
- **SEMANTIC strategy** (`LangSpecificMergeConflictResolver*`, PSI). **Import-block handling** (`MergeImportUtil`, `mergeLinesWithinRange`, `ChangeReferenceProcessor`). **AI resolution** (`replaceChangeWithAi`, `isResolvedWithAI`). **Iterative multi-file resolve.**
- **`compareLines(3)`** (the 3-side diff viewer), `getLineLeftToRightThreeSideDiffType`, `FineMergeLineFragment`, `SimpleThreesideTextDiffProvider`. Not merge. If ever needed, `compareLines(3)` is the same function with `keep_ignored = false`.
- **`mergeLinesAdditions`.**
- **Cancellation** (`CancellationChecker`), as in the 2-way port.
- **`MergeModelBase`'s document listener, `DiffUtil.updateRangeOnModification` and undo states.** Lenz has no free typing in the result. Undo is a snapshot of `MergeResult`.
- **The "modify/delete file conflict" guard.** Keep it as a model flag only if the mergetool research says Lenz will open such files.

## 7. Risks

- **Line-level tie-breaking.** `lenz-engine` uses gix-imara-diff's Myers at line level, not IntelliJ's `MyersLCS`. The 3-way zip inherits whatever 2-way matching it gets. The 2-way line goldens pass, so the 3-way ones probably will, but equal-cost alternatives can shift block borders. Mitigation: if a `LineComparisonMergeUtilTest` case fails on a tie, route that call through `lcs::diff` (already ported, exact).
- **The IGNORE branch difference** (see `ByLineRt` above). The 3-way path is not "2-way twice" under Ignore. Port the 3-way branch literally rather than reusing `by_line::compare`.
- **Offsets.** IntelliJ's word and char goldens are UTF-16. The merge test cases are ASCII, so there is no issue today, but the converter should reject non-ASCII cases or convert them the way `intellij_inner` does.
- **Resolve output to lines.** `LineTokenizer.tokenize(text, false)` gives zero lines for `""`, splits on `\n`, `\r` and `\r\n`, and drops a trailing empty line. `split_lines` behaves differently (`"a\n"` gives two lines). Match IntelliJ in `MergeResult` or the deleted/last-line cases will be off by one.
- **EOL, BOM and the trailing newline of the whole file.** The engine works on `\n`-split lines with `\r` stripped. Writing the result back with the right line endings and BOM is the app's job (the map's "writing files safely"). `MergeTest.testLastLine` shows the cases to cover, such as `"x"` vs `"x_"` vs `"x_y"`.
- **Cost.** `getLineMergeType` runs `resolve_simple`, a 3-way word diff, for **every** conflict at load time. That is fine for normal files, but pathological conflicts should hit the existing `MAX_REFINE_TOKENS` cap and report "not resolvable" rather than stall. IntelliJ's `DiffTooBigException` → `null` maps to the same thing.
- **Policy change resets the result** in IntelliJ. The Lenz spec must decide: forbid it, confirm it, or re-map resolved blocks.
- **Auto-apply on open diverges from IntelliJ's default.** That is intended, but `MergeTest`'s expectations assume it is off. Port those tests with an explicit "apply non-conflicting" step instead of relying on load behaviour.

---

[ij-head]: https://github.com/JetBrains/intellij-community/tree/bc3487548ccaba36486be2d88f8fd379a1d36258
[cmu]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ComparisonMergeUtil.kt
[byline-merge]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L33-L133
[byline-do3]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByLineRt.kt#L88-L120
[byword-3]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L61-L86
[byword-mcorr]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/ByWordRt.kt#L908-L1078
[trimutil]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/TrimUtil.kt
[mrs]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/MergeResolveUtil.kt
[mrs-simple]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/MergeResolveUtil.kt#L82
[mrs-greedy]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/comparison/MergeResolveUtil.kt#L175
[mru-type]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/util/MergeRangeUtil.kt#L15
[mru-can]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/util/MergeRangeUtil.kt#L109
[mr]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/util/MergeRange.kt
[mct]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/util/MergeConflictType.kt
[mcrs]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/util/MergeConflictResolutionStrategy.kt
[mlf]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/fragments/MergeLineFragmentImpl.kt
[mwf]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/diff/fragments/MergeWordFragmentImpl.kt
[diffconfig]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/util/diff/src/com/intellij/util/diff/DiffConfig.kt#L9
[cmi-merge]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/comparison/ComparisonManagerImpl.java#L87-L127
[cm-api]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-api/src/com/intellij/diff/comparison/ComparisonManager.java#L46-L80
[du-inner]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/util/DiffUtil.java#L1129-L1188
[settings]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/util/base/TextDiffSettingsHolder.kt#L35
[tdcb]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/tools/simple/ThreesideDiffChangeBase.kt
[mdb]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeDiffBuilder.kt
[mdb-merge]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeDiffBuilder.kt#L60-L98
[mcm]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeConflictModel.kt
[mcm-rediff]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeConflictModel.kt#L86-L114
[mcm-ranges]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeConflictModel.kt#L540
[mcm-resolve]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeConflictModel.kt#L211
[mcm-replace]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeConflictModel.kt#L282
[mcm-can]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeConflictModel.kt#L365
[mmb]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeModelBase.kt
[mmb-append]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeModelBase.kt#L289-L351
[tmc]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/TextMergeChange.kt
[mtv]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeThreesideViewer.java
[mtv-policies]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeThreesideViewer.java#L170
[mtv-auto]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeThreesideViewer.java#L669
[mtv-wand]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeThreesideViewer.java#L972-L995
[mtv-chunk]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/src/com/intellij/diff/merge/MergeThreesideViewer.java#L1215
[t-line]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison/LineComparisonMergeUtilTest.kt
[t-char]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison/CharComparisonMergeUtilTest.kt
[t-word]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison/WordMergeComparisonUtilTest.kt
[t-resolve]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison/MergeResolveUtilTest.kt
[t-base]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison/ComparisonMergeUtilTestBase.kt
[t-auto]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/comparison/ComparisonUtilAutoTest.kt
[t-merge]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/merge/MergeTest.kt
[t-mauto]: https://github.com/JetBrains/intellij-community/blob/bc3487548ccaba36486be2d88f8fd379a1d36258/platform/diff-impl/tests/testSrc/com/intellij/diff/merge/MergeAutoTest.kt
