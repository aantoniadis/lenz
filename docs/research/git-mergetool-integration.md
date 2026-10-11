# git mergetool integration

Research for ticket `.scratch/lenz-merge/issues/03-git-mergetool-integration-research.md`
(map: `.scratch/lenz-merge/map.md`). Git 2.55.0, Linux, 2026-10-02.

**Sources.** Primary only:
- git's own shell implementation, read from the installed files: `/usr/lib/git-core/git-mergetool`
  (cited as *mergetool:LINE*), `/usr/lib/git-core/git-mergetool--lib` (*lib:LINE*) and
  `/usr/lib/git-core/mergetools/<tool>`.
- The git 2.55 man pages: git-mergetool(1), git-config(1), git-merge(1), git-merge-file(1),
  git-status(1), git-ls-files(1), git-checkout(1), git-rebase(1), git-cherry-pick(1),
  gitattributes(5).
- `wt-status.c` (`wt_status_get_state`, `wt_status_check_rebase`) from github.com/git/git master.
- JetBrains, "Command-line merge tool" (jetbrains.com/help/idea/command-line-merge-tool.html).
- Experiments in throwaway repos under `/tmp/lenz-mergetool-exp`, with transcripts below. The
  scripts are inlined in the [Appendix](#appendix-experiment-harness).

## Answer

1. **How git calls a custom tool.** `mergetool.<tool>.cmd` is a shell string that git `eval`s
   (lib:171-177). `$BASE`, `$LOCAL`, `$REMOTE` and `$MERGED` are shell variables. They are **not
   exported**: the tool's environment only gets `GIT_PREFIX=.`. Lenz has to take the four paths
   as argv, for example `cmd = lenz merge "$BASE" "$LOCAL" "$REMOTE" "$MERGED"`.
   - The tool runs with **cwd = repo toplevel** (mergetool:528), even when mergetool was started
     from a subdirectory.
   - **`$MERGED` is the real working-tree file**, given relative to the toplevel (for example
     `src/notes.txt`). It holds git's merge result, conflict markers included, in the user's
     `merge.conflictStyle`.
   - The other three are temp files named `<stem>_{BASE,LOCAL,REMOTE}_<pid><.ext>`. They sit next
     to the file (`./src/notes_LOCAL_1907557.txt`), or in `$TMPDIR/git-mergetool-XXXXXX/` when
     `mergetool.writeToTemp=true`. Their mode is 0600 and the extension is kept, so
     syntax-by-extension works.
   - The temp files are produced by `git checkout-index --temp` (mergetool:230-240). They are
     therefore in **working-tree form**: CRLF and working-tree-encoding have already been applied.
2. **After the tool exits.**
   - **With `trustExitCode=true`:**
     - exit 0: git runs `git add $MERGED` (staged, conflict resolved) and moves the pre-tool file
       to `$MERGED.orig`, unless `mergetool.keepBackup=false`.
     - non-zero exit: git prints `merge of X failed` and **restores `$MERGED` from its backup**
       (mergetool:406-417). Anything the tool saved is thrown away and the path stays `UU`.
   - **Without `trustExitCode` (the default for every user-defined tool, lib:241-243):** git
     ignores the exit code and checks only whether `$MERGED`'s mtime is newer than a backup it
     touched just before the tool ran (lib:143-159, 335-345). A tool that saved and then exited 1
     is still staged (E2a). A tool that did not save leaves git asking `Was the merge successful
     [y/n]?` on the terminal.
   - **git never looks for leftover conflict markers.** An exit 0 or a "y" stages a file full of
     markers (E1c, E2c). Only `git diff --check` / `--cached --check` notices them ("leftover
     conflict marker", exit 2).
   - **Consequence for Lenz:** cancel is only safe when users set `trustExitCode = true`. Lenz's
     documented config must include it. "Done" must exit 0, and "cancel" must exit non-zero.
3. **No base (add/add).** `$BASE` still names a file, but an **empty 0-byte one** (mergetool:238).
   Built-in tools learn "no base" from an internal `$base_present` flag that a custom cmd cannot
   see.
   - Lenz should not guess from the empty file. Since cwd is the toplevel, it can run
     `git ls-files -u -z -- "$MERGED"` and check whether stage 1 is absent. Under
     `mergetool.writeToTemp` that is the only reliable signal.
   - Always pass `$BASE`. IntelliJ, for example, treats a missing base argument as "use the output
     file as the base".
4. **Which conflicts reach the tool.** git sorts each path by the modes of its index stages
   (mergetool:293-388):
   - **Text content conflicts:** go to the tool.
   - **Binary conflicts:** go to the tool unchanged. `$MERGED` holds *ours* (git's built-in
     binary driver keeps our version), and the three temp files hold the raw blobs.
   - **add/add:** goes to the tool with an empty `$BASE`.
   - **Executable-bit differences:** go to the tool. The working tree already has the merged mode.
   - **modify/delete, delete/modify, delete/delete, rename/delete, rename/rename** (any path where
     stage 2 or 3 is missing): git prompts `Use (m)odified or (d)eleted file, or (a)bort?` (or
     `(c)reated` when there's no base) and never starts the tool.
   - **Symlink conflicts:** `Use (l)ocal or (r)emote, or (a)bort?`, no tool.
   - **Submodule conflicts:** the same l/r/a prompt, no tool.
   - The Repository Window's conflict view needs the same split: a 3-way view for text, and
     pickers for the rest.
5. **How the built-in tools are wired** (`mergetools/*`):
   - **meld:** `meld [--auto-merge] --output="$MERGED" "$LOCAL" "$BASE" "$REMOTE"`. It writes the
     result to `$MERGED`, and the middle pane starts from **BASE**, not from the marker file. Its
     exit code is not trusted (mtime check).
   - **kdiff3:** `kdiff3 --auto --L1 "$MERGED (Base)" --L2 "$MERGED (Local)" --L3 "$MERGED (Remote)" -o "$MERGED" "$BASE" "$LOCAL" "$REMOTE"`.
     It drops the base when there is none and has a trusted exit code.
   - **vimdiff:** opens `LOCAL BASE REMOTE MERGED` and edits **`$MERGED` in place**, markers
     included. Its exit code is trusted (`:cq` cancels).
   - **vscode:** `code --wait --merge "$REMOTE" "$LOCAL" "$BASE" "$MERGED"`. Its exit code is not
     trusted.
   - **smerge:** `smerge mergetool "$BASE" "$LOCAL" "$REMOTE" -o "$MERGED"`.
   - **IntelliJ:** git has no built-in script for it. JetBrains documents
     `idea merge <path1> <path2> [<base>] <output>`, so the config a user writes is
     `idea merge "$LOCAL" "$REMOTE" "$BASE" "$MERGED"` plus `trustExitCode = true`. That config is
     derived from the documented syntax; the JetBrains page shows no git config.
   - **The shared convention:** every tool writes **only `$MERGED`**, and the inputs are
     read-only. Tools that start from inputs (meld, kdiff3, IntelliJ) recompute the merge
     themselves and ignore the markers in `$MERGED`. That matches Lenz's charted decision to start
     the result from the base with every non-conflicting change applied.
6. **Detecting an operation in progress.** Follow `wt_status_get_state`. Resolve every path with
   `git rev-parse --git-path <name>`, because a linked worktree keeps these files in
   `.git/worktrees/<name>/` (E4g). The checks, in order:
   - `MERGE_HEAD`: merge.
   - `rebase-apply/` (with `applying` → `git am`, without it → `rebase --apply`).
   - `rebase-merge/` (with `interactive` → interactive rebase, without it → plain rebase).
   - `CHERRY_PICK_HEAD`: cherry-pick.
   - `REVERT_HEAD`: revert.
   - `sequencer/todo`: a multi-commit pick or revert that has stopped.

   During a rebase, `REBASE_HEAD` is the commit being replayed. **`git stash pop`/`apply` leaves no
   state file at all** (only ort's `AUTO_MERGE`, which every ort operation writes), so a conflict
   is detected **from unmerged index entries, never from state files**. State files only name
   the operation and provide labels.
7. **Listing conflicts.** `git status --porcelain=v2 -z` gives one `u` record per path:
   `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>`. A missing stage shows as mode
   `000000` and an all-zero sha. XY is the conflict type: `UU`, `AA`, `UD`, `DU`, `AU`, `UA` or
   `DD`. `<sub>` is `S...` for submodules.
   - That one record has everything Lenz needs. `changeset.rs:428` already parses `u` lines but
     keeps only the path, so no extra `ls-files -u` call is required. `git ls-files -u -z` gives
     the same information as one line per stage: `<mode> <sha> <stage>\t<path>`.
   - Read a stage with `git cat-file -p :N:path` to get the raw index blob (LF, UTF-8), or with
     `git cat-file --filters :N:path` to get working-tree form (CRLF, UTF-16 and so on). This is
     the same form mergetool hands out, and it can go through the existing long-lived `cat-file`
     process.
   - `git merge-file -p --diff3 -L … --object-id :2:p :1:p :3:p` recomputes the marker file
     without touching the tree. Its exit code is the number of conflicts.
8. **Ours / theirs labels.** Stage 2 is always `HEAD` and stage 3 is always "the other side".
   What that means depends on the operation:
   - **merge:** ours = the current branch (`# branch.head`). Theirs = `MERGE_HEAD`, with the
     branch name taken from `MERGE_MSG` ("Merge branch 'topic'").
   - **rebase:** **the sides are swapped** (git-rebase(1), git-checkout(1) `--ours`). Ours = the
     upstream being rebased onto (`rebase-merge/onto`; HEAD is detached). Theirs = the user's
     commit, `REBASE_HEAD` (`76c12fd (topic: edit two)`). The branch is in
     `rebase-merge/head-name` and progress in `msgnum`/`end`.
   - **cherry-pick:** theirs = `CHERRY_PICK_HEAD` (`76c12fd (topic: edit two)`), and the base is
     "parent of 76c12fd".
   - **revert:** theirs = `parent of <REVERT_HEAD>`, and the base is `REVERT_HEAD` itself.
   - **stash pop:** "Updated upstream" (ours), "Stashed changes" (theirs), "Stash base".

   git writes these exact labels into the markers (E4). Lenz should show human labels such as
   "main (upstream)" and "your commit 76c12fd topic: edit two" instead of "ours/theirs".
9. **Writing the result safely.** git hashes whatever bytes are in the file, but `git add` also
   records the **file mode** and passes the bytes **through the checkin filters**:
   - **Mode:** a temp-file-plus-rename save creates a new 0644 inode. `git add` then records
     `mode change 100755 => 100644` (E5c). Lenz must keep the existing mode, by copying
     permissions onto the temp file or by writing in place.
   - **Line endings:** under `core.autocrlf=true` or `text`/`eol` attributes, the working-tree
     file and the temp files are CRLF. LF (or mixed endings) written by a tool is normalised to LF
     in the index on `git add`, with a warning. The working-tree file stays different, though, so
     Lenz should keep `$MERGED`'s EOL style (E5a).
   - **Encoding:** with `working-tree-encoding=UTF-16LE-BOM`, `$MERGED` and the temp files are
     UTF-16 with CRLF, markers included. Writing UTF-8 makes `git add` **fail**:
     `fatal: failed to encode 'u.txt' from UTF-16LE-BOM to UTF-8` (E5g). Lenz must write back in
     the file's working-tree encoding.
   - **Symlinks and submodules:** they have no "content" to write. Resolving one is an index
     operation, not a file write.
10. **Re-merge (restoring markers).**
    - `git checkout --conflict=merge|diff3|zdiff3 -- <path>` rewrites the file from the index
      stages. It works **even after `git add`**, because the resolve-undo index extension restores
      the `UU` stages (E5f). So it can also undo "Mark resolved", up to the commit.
      `git restore --merge` is equivalent.
    - The markers it writes are labelled `ours`/`base`/`theirs`, not branch names.
    - Lenz can also recompute the result in memory from the stages, which needs no git write.
11. **Styles.** `merge` (the default) shows only ours and theirs, and moves lines common to both
    sides out of the hunk. `diff3` adds `||||||| <base label>` followed by the base text. `zdiff3`
    is like diff3 but moves lines that are identical at the start and end of the hunk outside the
    markers (E1e). The base label is the abbreviated merge-base sha in a merge, "parent of …" in
    rebase and cherry-pick, and "constructed fake ancestor" in `rebase --apply`. A Lenz parser of
    an existing marker file must handle all three styles, plus `conflict-marker-size` from
    gitattributes, which changes the marker length from 7. Better: don't parse markers, and use
    the stages.

### Recommendations for the spec

- Document this setup:
  ```
  [mergetool "lenz"]
      cmd = lenz merge "$BASE" "$LOCAL" "$REMOTE" "$MERGED"
      trustExitCode = true
  [merge]
      tool = lenz
  ```
  `lenz merge BASE LOCAL REMOTE MERGED` is positional and absolute-or-cwd-relative, with an
  optional `-L` label for each input. An empty or absent BASE means a 2-way merge. Done exits 0,
  and cancel or close-without-saving exits 1.
- Under mergetool, Lenz runs in the repo toplevel. It can enrich labels and modes by querying git
  (`ls-files -u -- "$MERGED"`, `rev-parse --git-path REBASE_HEAD`, …), but it must work without
  that, because `mergetool.writeToTemp` and non-repo use are possible.
- Repository Window conflict list: from the `u` records (already fetched). Classify each path the
  same way as `merge_file()`:
  - mode `160000` in stage 2 or 3: submodule picker.
  - stage 2 or 3 missing: keep/delete picker.
  - `120000`: symlink picker.
  - otherwise a 3-way text view, or a binary ours/theirs picker when the content is binary.
- Write-back: write `$MERGED` or the working-tree file in place (or temp + rename with the old
  mode copied over), keeping its EOL style, BOM and encoding. Never stage, except through the
  explicit "Mark resolved", which is `git add -- <path>`. For a delete resolution it is
  `git rm -- <path>`. For a symlink or submodule it is `git checkout-index -f --stage=N` + add,
  or `update-index --cacheinfo`, as mergetool does (mergetool:92-228).
- Warn before Done or Mark resolved when markers remain. `git diff --check` semantics is a
  reasonable rule, since git itself won't check.

---

## Evidence

All transcripts below are verbatim, trimmed only where marked `…`. The dummy tool prints
`tool|` lines.

### E1 — text conflict, the four files, styles

```
$ git merge feature
CONFLICT (content): Merge conflict in src/notes.txt
[exit 1]

$ git status --porcelain=v2 -z | tr '\0' '\n'
u UU N... 100644 100644 100644 100644 2019eda9… ced80c4c… 5be68b8d… src/notes.txt

$ git ls-files -u -z | tr '\0' '\n'
100644 2019eda923040a79f52fe4bf49e0a106f4f658ae 1	src/notes.txt
100644 ced80c4c2c435f7d3dc01f3481a4265681696195 2	src/notes.txt
100644 5be68b8db768f5bda00bf253e4e972a0cdf9d414 3	src/notes.txt

$ git config mergetool.log.trustExitCode true
$ TOOL_ACTION=resolve git mergetool
Merging:
src/notes.txt

Normal merge conflict for 'src/notes.txt':
  {local}: modified file
  {remote}: modified file
tool| argv:
tool|   [./src/notes_BASE_1907557.txt]
tool|   [./src/notes_LOCAL_1907557.txt]
tool|   [./src/notes_REMOTE_1907557.txt]
tool|   [src/notes.txt]
tool| cwd: /tmp/lenz-mergetool-exp/text
tool| exported env: GIT_PREFIX=.
tool| --- ./src/notes_BASE_1907557.txt: -rw-------, 34 bytes
tool|     one$ two$ three$ four$ five$ six$ seven$                 (one per line in the real output)
tool| --- ./src/notes_LOCAL_1907557.txt: -rw-------, 48 bytes
tool|     one (main)$ two$ three (main)$ four$ five$ six$ seven$
tool| --- ./src/notes_REMOTE_1907557.txt: -rw-------, 54 bytes
tool|     one$ two$ THREE (feature)$ four$ five$ six$ seven (feature)$
tool| --- src/notes.txt: -rw-r--r--, 111 bytes
tool|     one (main)$
tool|     two$
tool|     <<<<<<< HEAD$
tool|     three (main)$
tool|     =======$
tool|     THREE (feature)$
tool|     >>>>>>> feature$
tool|     four$
tool|     five$
tool|     six$
tool|     seven (feature)$
tool| action=resolve, exiting 0

$ git status --porcelain=v2 --untracked-files=all
1 M. N... 100644 100644 100644 ced80c4c… 2ab19ae6… src/notes.txt
? src/notes.txt.orig
```

`$LOCAL`/`$REMOTE`/`$BASE` are the pure stage contents. `$MERGED` already has the clean
changes from both sides applied ("one (main)", "seven (feature)"). `.orig` is the pre-tool file
(`keepBackup` defaults to true).

`merge.conflictStyle=diff3` (zdiff3 was identical for this input):
```
one (main)
two
<<<<<<< HEAD
three (main)
||||||| 77ae763
three
=======
THREE (feature)
>>>>>>> feature
four
…
```

**E1c**: run from `src/` with `writeToTemp=true`, `keepBackup=false`, `trustExitCode=true`. The
tool **doesn't touch** `$MERGED` and exits 0:
```
tool|   [/tmp/git-mergetool-tDfgr8/notes_BASE_1907727.txt]
tool|   [/tmp/git-mergetool-tDfgr8/notes_LOCAL_1907727.txt]
tool|   [/tmp/git-mergetool-tDfgr8/notes_REMOTE_1907727.txt]
tool|   [src/notes.txt]
tool| cwd: /tmp/lenz-mergetool-exp/text-tmp
…
tool| action=keep, exiting 0

$ git status --short
M  notes.txt          ← staged with the conflict markers still inside
```

**E1d**: `git mergetool --tool-help` lists config-defined tools:
```
	user-defined:
		log.cmd sh /tmp/lenz-mergetool-exp/logtool.sh "$BASE" "$LOCAL" "$REMOTE" "$MERGED"
```

**E1e**: where the styles differ. Base `a b c`, ours `a X b2 Y c`, theirs `a X b3 Y c`:
```
$ git merge-file -p -L ours -L base -L theirs ours base theirs        # merge
a
X
<<<<<<< ours
b2
=======
b3
>>>>>>> theirs
Y
c
[exit 1]

$ git merge-file -p --diff3 …                                          # diff3
a
<<<<<<< ours
X
b2
Y
||||||| base
b
=======
X
b3
Y
>>>>>>> theirs
c

$ git merge-file -p --zdiff3 …                                         # zdiff3
a
X
<<<<<<< ours
b2
||||||| base
b
=======
b3
>>>>>>> theirs
Y
c
```

### E2 — exit codes and trustExitCode

Two conflicted files `a.txt` and `b.txt`. The tool output is reduced to its last line.

```
## E2a: trustExitCode unset; tool WRITES $MERGED, exits 1
$ TOOL_ACTION=resolve TOOL_EXIT=1 git mergetool a.txt
tool| action=resolve, exiting 1
$ git status --short
M  a.txt              ← staged: the exit code was ignored, the mtime decided
UU b.txt
?? a.txt.orig

## E2b: trustExitCode unset; tool exits 0 without touching $MERGED; user answers n
$ printf 'n\n' | git mergetool a.txt
tool| action=keep, exiting 0
a.txt seems unchanged.
Was the merge successful [y/n]?
merge of a.txt failed
$ git status --short
UU a.txt

## E2c: same, user answers y
$ git status --short
M  a.txt
$ git diff --cached --check; echo check-exit=$?
a.txt:2: leftover conflict marker
a.txt:4: leftover conflict marker
a.txt:6: leftover conflict marker
check-exit=2

## E2d: trustExitCode=true; tool WRITES a resolution, then exits 1 (cancel)
$ TOOL_ACTION=resolve TOOL_EXIT=1 git mergetool a.txt
merge of a.txt failed
$ git status --short; cat a.txt
UU a.txt
one
<<<<<<< HEAD          ← git put the backup back; the tool's save is gone
two main
=======
TWO feature
>>>>>>> feature
three

## E2e: trustExitCode=true, keepTemporaries=true, exit 1, two files
$ printf 'n\n' | TOOL_EXIT=1 git -c mergetool.keepTemporaries=true mergetool
merge of a.txt failed
Continue merging other unresolved paths [y/n]?
$ ls
a.txt  a_BASE_1914928.txt  a_LOCAL_1914928.txt  a_REMOTE_1914928.txt  b.txt

## E2f
$ git mergetool
No files need merging
```

With `--no-prompt`/`-y`, git skips only the "Hit return to start merge resolution tool" prompt
(mergetool:393-397), and only when that prompt would have appeared (a guessed tool, or
`mergetool.prompt=true`). The "seems unchanged", m/d/a and l/r/a prompts are not affected.
mergetool exits 1 if any file failed (mergetool:563-576).

### E3 — conflict kinds

```
## E3a add/add
u AA N... 000000 100644 100644 100644 0000000000000000000000000000000000000000 26f343c0… 84f29964… new.txt
$ git ls-files -u
100644 26f343c0… 2	new.txt
100644 84f29964… 3	new.txt
$ git mergetool
Normal merge conflict for 'new.txt':
  {local}: created file
  {remote}: created file
tool| --- ./new_BASE_1916378.txt: -rw-r--r--, 0 bytes      ← empty, not absent
tool| --- ./new_LOCAL_1916378.txt: -rw-------, 10 bytes
tool| --- new.txt: -rw-r--r--, 60 bytes
tool|     <<<<<<< HEAD$ from main$ =======$ from feature$ >>>>>>> feature$   (one per line in the real output)

## E3b modify/delete (we modified, they deleted)
CONFLICT (modify/delete): doc.txt deleted in feature and modified in HEAD.  Version HEAD of doc.txt left in tree.
u UD N... 100644 100644 000000 100644 a999a0c2… 85dadd11… 0000000… doc.txt
$ printf 'm\n' | git mergetool
Deleted merge conflict for 'doc.txt':
  {local}: modified file
  {remote}: deleted
Use (m)odified or (d)eleted file, or (a)bort?          ← no tool started
$ git status --short
?? doc.txt.orig

## E3c delete/modify, answer d
u DU N... 100644 000000 100644 100644 a999a0c2… 0000000… 37fadb2b… doc.txt
  {local}: deleted
  {remote}: modified file
Use (m)odified or (d)eleted file, or (a)bort?
$ git status --short                                    ← (empty: git rm'd)

## E3d binary
warning: Cannot merge binary files: img.bin (HEAD vs. feature)
CONFLICT (content): Merge conflict in img.bin
u UU N... 100644 100644 100644 100644 0b1375af… bdb31866… 78aaa295… img.bin
$ od -c img.bin
0000000   P   N   G  \0 001 002   m   a   i   n   !       ← working tree = ours
$ git mergetool
Normal merge conflict for 'img.bin':                    ← tool IS started
tool| --- ./img_BASE_1916701.bin: -rw-------, 10 bytes   PNG^@^A^Bbase
tool| --- ./img_LOCAL_1916701.bin: -rw-------, 11 bytes  PNG^@^A^Bmain!
tool| --- ./img_REMOTE_1916701.bin: -rw-------, 13 bytes PNG^@^A^Bfeature
tool| --- img.bin: -rw-r--r--, 11 bytes                  PNG^@^A^Bmain!

## E3e symlink
u UU N... 120000 120000 120000 120000 705c4325… 253b7dc7… 12dfabd6… link
$ printf 'r\n' | git mergetool
Symbolic link merge conflict for 'link':
  {local}: a symbolic link -> 'target-main'
  {remote}: a symbolic link -> 'target-feature'
Use (l)ocal or (r)emote, or (a)bort?
$ git status --short; ls -l link
M  link
lrwxrwxrwx … link -> target-feature

## E3f submodule
CONFLICT (submodule): Merge conflict in sub
u UU S... 160000 160000 160000 160000 0ed8f69b… e38e1eaa… d621a996… sub
$ printf 'l\n' | git mergetool
Submodule merge conflict for 'sub':
  {local}: submodule commit e38e1eaa5ad5951d854a33f66a3f2a1317881dd8
  {remote}: submodule commit d621a99699dce8e06ae91c595404400df7c8c7da
Use (l)ocal or (r)emote, or (a)bort?

## E3g rename/rename + rename/delete: one record per path, stages scattered
CONFLICT (rename/rename): r.txt renamed to r-main.txt in HEAD and to r-feature.txt in feature.
CONFLICT (rename/delete): s.txt renamed to s-main.txt in HEAD, but deleted in feature.
u UA N... 000000 000000 100644 100644 000… 000… 8a1218a1… r-feature.txt
u AU N... 000000 100644 000000 100644 000… 8a1218a1… 000… r-main.txt
u DD N... 100644 000000 000000 000000 8a1218a1… 000… 000… r.txt
u UD N... 100644 100644 000000 100644 94053253… 94053253… 000… s-main.txt
$ git mergetool
Deleted merge conflict for 'r-feature.txt':
  {local}: deleted
  {remote}: created file
Use (c)reated or (d)eleted file, or (a)bort?
```

### E4 — operation state, labels, stages (with `merge.conflictStyle=diff3`)

`ls -d` over `MERGE_HEAD MERGE_MSG MERGE_MODE AUTO_MERGE ORIG_HEAD REBASE_HEAD CHERRY_PICK_HEAD REVERT_HEAD rebase-merge rebase-apply sequencer`
inside `$(git rev-parse --git-dir)`:

| operation | files present | `# branch.head` | marker labels (ours / base / theirs) |
|---|---|---|---|
| `git merge topic` | AUTO_MERGE MERGE_HEAD MERGE_MODE MERGE_MSG ORIG_HEAD | `main` | `HEAD` / `d9c4e84` / `topic` |
| `git rebase main` (merge backend) | AUTO_MERGE MERGE_MSG ORIG_HEAD REBASE_HEAD rebase-merge | `(detached)` | `HEAD` / `parent of 76c12fd (topic: edit two)` / `76c12fd (topic: edit two)` |
| `git rebase --apply main` | REBASE_HEAD rebase-apply | (not captured) | `HEAD` / `constructed fake ancestor` / `topic: edit two` |
| `git cherry-pick topic~1 topic` | AUTO_MERGE CHERRY_PICK_HEAD MERGE_MSG sequencer | `main` | `HEAD` / `parent of 76c12fd (topic: edit two)` / `76c12fd (topic: edit two)` |
| `git revert HEAD~1` | AUTO_MERGE MERGE_MSG REVERT_HEAD | `main` | `HEAD` / `0b5644d (main: edit two)` / `parent of 0b5644d (main: edit two)` |
| `git stash pop` | AUTO_MERGE (+ a stale ORIG_HEAD) — **no operation marker** | `main` | `Updated upstream` / `Stash base` / `Stashed changes` |

Rebase details:
```
$ ls .git/rebase-merge
author-script done drop_redundant_commits end git-rebase-todo git-rebase-todo.backup
head-name interactive message msgnum no-reschedule-failed-exec onto orig-head patch stopped-sha
head-name: refs/heads/topic
onto: 0b5644d5cbb30a436291d2f5b3b787f5c70b0cd1
orig-head: 906f7cf530763b4cda552ad7126a3849ddf83544
msgnum: 1
end: 2
stopped-sha: 76c12fd9631eeb703ff81ba4fca57c2aff203a77

$ git log -1 --format='%h %s' REBASE_HEAD; git log -1 --format='%h %s' HEAD
76c12fd topic: edit two
0b5644d main: edit two

$ git show :2:f.txt          # "ours" = upstream main
one
two (main)
three
$ git show :3:f.txt          # "theirs" = the user's commit being replayed
one
two (topic)
three
```

Note: a non-interactive `git rebase` in 2.55 also creates `rebase-merge/interactive` (it runs
on the sequencer), so `wt_status_check_rebase` reports it as interactive.

Cherry-pick of two commits: `.git/sequencer/todo` = `pick 76c12fd topic: edit two` /
`pick 906f7cf topic: add four`.

Linked worktree (E4g):
```
$ cat .git
gitdir: /tmp/lenz-mergetool-exp/wt/.git/worktrees/wt-linked
$ git rev-parse --git-dir --git-path MERGE_HEAD --git-common-dir
/tmp/lenz-mergetool-exp/wt/.git/worktrees/wt-linked
/tmp/lenz-mergetool-exp/wt/.git/worktrees/wt-linked/MERGE_HEAD
/tmp/lenz-mergetool-exp/wt/.git
```

`wt_status_get_state` (wt-status.c, git master), for reference:
```c
if (!stat(git_path_merge_head(r), &st)) {
        wt_status_check_rebase(wt, state);
        state->merge_in_progress = 1;
} else if (wt_status_check_rebase(wt, state)) {
        ;               /* all set */
} else if (refs_ref_exists(get_main_ref_store(r), "CHERRY_PICK_HEAD") && …) {
        state->cherry_pick_in_progress = 1; …
}
wt_status_check_bisect(wt, state);
if (refs_ref_exists(get_main_ref_store(r), "REVERT_HEAD") && …) {
        state->revert_in_progress = 1; …
}
if (!sequencer_get_last_command(r, &action)) { /* REPLAY_PICK / REPLAY_REVERT */ … }
```
`wt_status_check_rebase` checks `rebase-apply` (`applying` → am, otherwise rebase; branch from
`rebase-apply/head-name`, onto from `rebase-apply/onto`) and then `rebase-merge` (`interactive`;
`head-name`, `onto`).

### E5 — write-back

```
## E5a core.autocrlf=true
$ git ls-files --eol w.txt
i/lf    w/crlf  attr/                 	w.txt
$ cat -A w.txt                       # working tree / $MERGED
one^M$
<<<<<<< HEAD^M$
two main^M$
=======^M$
two feature^M$
>>>>>>> feature^M$
three^M$
$ git cat-file -p :2:w.txt | cat -A          # raw index blob
one$
two main$
three$
$ git cat-file --filters :2:w.txt | cat -A   # working-tree form
one^M$
two main^M$
three^M$
tool| --- ./w_LOCAL_1919423.txt: -rw-------, 22 bytes
tool|     one^M$                             ← temp files are CRLF too
$ printf 'one\ntwo both\nthree\n' > w.txt && git add w.txt && git ls-files --eol w.txt
warning: in the working copy of 'w.txt', LF will be replaced by CRLF the next time Git touches it
i/lf    w/lf    attr/                 	w.txt
$ printf 'one\r\ntwo both\nthree\r\n' > w.txt && git add w.txt && git ls-files --eol w.txt && git cat-file -p :w.txt | cat -A
i/lf    w/mixed attr/                 	w.txt
one$
two both$
three$

## E5b .gitattributes: *.txt text working-tree-encoding=UTF-16LE-BOM eol=crlf
$ file u.txt; head -c 40 u.txt | od -c
u.txt: Unicode text, UTF-16, little-endian text, with CRLF line terminators
0000000 377 376   o  \0   n  \0   e  \0  \r  \0  \n  \0   <  \0   <  \0
0000020   <  \0   <  \0   <  \0   <  \0   <  \0      \0   H  \0   E  \0     ← markers re-encoded
$ git cat-file -p :2:u.txt | od -c          → UTF-8, LF
$ git cat-file --filters :2:u.txt | od -c   → 377 376 o \0 n \0 e \0 \r \0 \n \0 …
tool| --- ./u_LOCAL_1919576.txt: -rw-------, 46 bytes   ← UTF-16 + BOM (23 UTF-8 bytes in the blob)

## E5g write UTF-8 into that file, then add
$ printf 'plain utf8\n' > u.txt; git add u.txt
fatal: failed to encode 'u.txt' from UTF-16LE-BOM to UTF-8

## E5c executable file; tool saves via temp file + rename
$ git ls-files -u; ls -l run.sh
100755 abd1b84d… 1	run.sh
100755 84fb8b4d… 2	run.sh
100755 6b6c8041… 3	run.sh
-rwxr-xr-x … run.sh
$ TOOL_ACTION=atomic git mergetool
$ ls -l run.sh; git ls-files -s run.sh; git diff --cached --summary HEAD
-rw-r--r-- … run.sh
100644 2ab19ae6… 0	run.sh
 mode change 100755 => 100644 run.sh

## E5h only theirs made it executable: the working tree gets the merged mode
100644 de980441… 1	s.sh
100644 ab12daa6… 2	s.sh
100755 c2162358… 3	s.sh
-rwxr-xr-x … s.sh
```

Re-merge (E5d/E5f):
```
$ echo 'half-edited' > f.txt; git checkout --conflict=diff3 -- f.txt; cat f.txt
one
<<<<<<< ours
two main
||||||| base
two
=======
two feature
>>>>>>> theirs
three

$ echo resolved > f.txt; git add f.txt; git status --short
M  f.txt
$ git checkout --conflict=merge -- f.txt; git status --short; git ls-files -u; cat f.txt
UU f.txt                                     ← resolve-undo brought the stages back
100644 4cb29ea3… 1	f.txt
100644 02dc0203… 2	f.txt
100644 096c309c… 3	f.txt
one
<<<<<<< ours
two main
=======
two feature
>>>>>>> theirs
three
```
(`git update-index --unresolve <path>` restores the stages without touching the file, and
`git restore --merge -- <path>` is equivalent to `checkout --conflict`.)

Computing markers without touching the tree (E5e):
```
$ git merge-file -p --diff3 -L HEAD -L base -L feature --object-id :2:f.txt :1:f.txt :3:f.txt
one
<<<<<<< HEAD
two main
||||||| base
two
=======
two feature
>>>>>>> feature
three
[exit 1]
```
This works on raw blobs (LF/UTF-8). For working-tree form, Lenz would apply the file's
EOL/encoding itself.

---

## Built-in tool scripts (excerpts)

`mergetools/meld`:
```sh
if test "$meld_has_output_option" = true
then
	"$merge_tool_path" $option_auto_merge --output="$MERGED" \
		"$LOCAL" "$BASE" "$REMOTE"
else
	"$merge_tool_path" $option_auto_merge "$LOCAL" "$MERGED" "$REMOTE"
fi
```
(`--output` is detected from `meld --help` or set by `mergetool.meld.hasOutput`. `--auto-merge`
comes from `mergetool.meld.useAutoMerge`. There is no `exit_code_trustable`, so git uses the
mtime check.)

`mergetools/kdiff3`:
```sh
if $base_present
then
	"$merge_tool_path" --auto \
		--L1 "$MERGED (Base)" --L2 "$MERGED (Local)" --L3 "$MERGED (Remote)" \
		-o "$MERGED" "$BASE" "$LOCAL" "$REMOTE" >/dev/null 2>&1
else
	"$merge_tool_path" --auto \
		--L1 "$MERGED (Local)" --L2 "$MERGED (Remote)" \
		-o "$MERGED" "$LOCAL" "$REMOTE" >/dev/null 2>&1
fi
exit_code_trustable () { true }
```

`mergetools/vimdiff` (default layout `(LOCAL,BASE,REMOTE)/MERGED`; trusted):
```sh
eval '"$merge_tool_path"' -f "$FINAL_CMD" '"$LOCAL"' '"$BASE"' '"$REMOTE"' '"$MERGED"'
# no base: windows for BASE are closed, files are LOCAL REMOTE MERGED
# layouts marking @LOCAL/@REMOTE: on exit 0, cp that buffer's file over $MERGED
```

`mergetools/vscode`: `"$merge_tool_path" --wait --merge "$REMOTE" "$LOCAL" "$BASE" "$MERGED"`.

Tools that declare `exit_code_trustable` true in 2.55: deltawalker, diffmerge, emerge, guiffy,
kdiff3, kompare, tkdiff, vimdiff (and its gvimdiff/nvimdiff variants). meld, vscode, p4merge,
smerge, tortoisemerge, bc and the others do not. `mergetool.<tool>.trustExitCode` overrides
this either way (lib:294-304).

`mergetool.hideResolved=true` (or per tool) rewrites `$LOCAL`/`$REMOTE` with
`git merge-file --ours`/`--theirs` before the tool runs, so that they differ only in the
conflicting hunks (mergetool:242-247, 336-368). It defaults to false. Lenz needs nothing from
it, since it computes its own merge.

IntelliJ (JetBrains docs): `idea merge <path1> <path2> [<base>] <output>`, where
"path1, path2: paths for two modified versions of a file; base (optional): the base revision;
output: the output file to save merge results". If the base is omitted, the current contents of
the output file are used as the common origin.

---

## Appendix: experiment harness

The real home config is kept out by setting `HOME`, `XDG_CONFIG_HOME`, `GIT_CONFIG_GLOBAL` and
`GIT_CONFIG_NOSYSTEM`. Without `XDG_CONFIG_HOME`, `git config --global` writes to
`~/.config/git/config` when no `~/.gitconfig` exists.

`common.sh`:
```sh
X=/tmp/lenz-mergetool-exp
export HOME=$X/home GIT_CONFIG_NOSYSTEM=1 XDG_CONFIG_HOME=$X/home/.config GIT_CONFIG_GLOBAL=$X/home/.gitconfig
export GIT_AUTHOR_DATE='2026-10-02T12:00:00Z' GIT_COMMITTER_DATE='2026-10-02T12:00:00Z' LC_ALL=C
git config --global user.name Lenz
git config --global user.email lenz@example.com
git config --global init.defaultBranch main
git config --global merge.tool log
git config --global mergetool.log.cmd "sh $X/logtool.sh \"\$BASE\" \"\$LOCAL\" \"\$REMOTE\" \"\$MERGED\""
run() { printf '\n$ %s\n' "$*"; ( eval "$*" ) 2>&1; rc=$?; [ $rc -ne 0 ] && echo "[exit $rc]"; }
```

`logtool.sh` (the dummy merge tool):
```sh
#!/bin/sh
echo "tool| argv:"
for a in "$@"; do echo "tool|   [$a]"; done
echo "tool| cwd: $(pwd)"
echo "tool| exported env: $(env | grep -E '^(BASE|LOCAL|REMOTE|MERGED|GIT_PREFIX|GIT_DIR)=' | tr '\n' ' ')"
for a in "$@"; do
  echo "tool| --- $a: $(stat -c '%A, %s bytes' "$a" 2>&1)"
  head -c 400 "$a" | cat -A | sed 's/^/tool|     /'
done
case "${TOOL_ACTION:-keep}" in
  resolve) printf 'resolved\n' > "$4" ;;
  atomic)  printf 'resolved\n' > "$4.lenz-tmp" && mv "$4.lenz-tmp" "$4" ;;
esac
echo "tool| action=${TOOL_ACTION:-keep}, exiting ${TOOL_EXIT:-0}"
exit "${TOOL_EXIT:-0}"
```

Each scenario builds a fresh repo the same way, with `base` on `main`, a `feature` (or `topic`)
branch changing one side and `main` changing the other, and then runs `git merge feature`
(or the operation under test) followed by the commands shown in the transcripts.
