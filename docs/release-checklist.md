# Release checklist

Run on **Linux and macOS** before each release, on the release commit, with a release build. Tick each box on both and note the machine (CPU, display refresh rate) and the numbers next to the timed items. The automated checks run locally through `scripts/ci-local.sh`; this list covers what they cannot: physical-display behavior, mouse input and packaging.

## Setup

```sh
cargo build --release --locked -p lenz
scripts/perf-check.sh                    # engine, views, syntax, search, images and history gates: must exit 0
cargo bench -p lenz-engine --bench engine -- --write-fixtures /tmp/lenz-fx
REG=$(dirname "$(ls -d ~/.cargo/registry/src/*/tokio-1.52.1 | head -1)")   # where tokio 1.52.1/1.53.1 live, if present
```

## Performance

Run the [full or stress suite](benchmarking.md) and keep its result directory for each machine. A passing suite does not replace the manual items below.

- [ ] **First paint < 300 ms warm.** Run `LENZ_TIMINGS=1 target/release/lenz /tmp/lenz-fx/generated-100k/left.rs /tmp/lenz-fx/generated-100k/right.rs` three times, closing the window each time. Treat the first run as a warmup, not a cold-boot measurement; the `lenz: first paint N ms` line on stderr of runs 2 and 3 must be under 300. Note `load` and `diff` too.
- [ ] **Scrolling at display refresh rate.** In that 100k window, hold PageDown from top to bottom, then scroll back up with the trackpad/wheel, fast. `LENZ_TIMINGS` prints no frame times, so judge by eye: motion is even with no hitches or stalls, text, gutters and Connectors stay locked together, and no blank frames appear at the edges. Repeat in each Scroll Mode below.
- [ ] **Pathological pair stays usable.** `target/release/lenz /tmp/lenz-fx/different-100k/left.rs /tmp/lenz-fx/different-100k/right.rs` opens (note first paint) and scrolls.
- [ ] **Memory after highlighting.** On Linux, `python3 scripts/startup-bench.py -- /tmp/lenz-fx/generated-100k/left.rs /tmp/lenz-fx/generated-100k/right.rs` measures first paint and RSS on a private Xvfb display. Repeat for a large merge, and compare with a saved binary using `--baseline`.
- [ ] **Memory while browsing.** Generate disposable inputs with `scripts/perf-fixtures.py`, then use `scripts/interaction-bench.py` for repeated text and image file switching and merge shortcut repaints.
- [ ] **Stripped binary ≤ 80 MB.** `strip -o /tmp/lenz-stripped target/release/lenz && wc -c < /tmp/lenz-stripped` prints at most 80000000.

## UI walkthrough

On the generated 100k pair, then on a Change Set (`target/release/lenz "$REG/tokio-1.52.1" "$REG/tokio-1.53.1"`, or any repository with `lenz HEAD~5`):

- [ ] Scroll Modes: `s` cycles Shared, Aligned, Padded and Independent; in each, scroll and check the Sides and Connectors behave as that mode promises. `--scroll=<mode>` starts in that mode.
- [ ] `w` cycles Whitespace Mode (default → trim → ignore → ignore + trim chunks, shown in the status corner); the diff updates and the view stays in place.
- [ ] `i` toggles Ignore Case (`ignoring case`); `h` cycles Highlight Level (word → char → line) and Inline Changes update.
- [ ] `]` / `[` (and Ctrl/Cmd+] / Ctrl/Cmd+[) switch to the next / previous file; the window title follows; nothing happens past the first or last file.
- [ ] Syntax highlighting appears on `.rs` files, including after scrolling far down and after switching files.
- [ ] `git difftool -d HEAD~5` with the README's difftool config opens one window for the whole Change Set; closing it returns to the shell.
- [ ] `n` / `p` (F7, Alt+↓/↑) step through Change Blocks; at a file's edge a toast asks to press again and the second press opens the next/previous file with changes at its first/last change; the header shows `change j/M`.
- [ ] `z` toggles Collapsed Regions keeping the view in place; **clicking** a `⋯  N unchanged lines` band expands it.
- [ ] **Mouse:** click places the caret, drag / shift-click / double-click / triple-click select, `ctrl-c` copies (Linux: middle-click pastes the selection elsewhere); right-click menu offers Copy selection and Copy reference (path:line) with a selection, Fold / Unfold Change Block, Move change, Open focused side in editor and the blame entries; a right-click on a file in the File Navigator offers Copy file path and Show file history.
- [ ] `ctrl-g` `120:5` goes to line 120 column 5 on the focused Side (right if none); a target in a Collapsed Region expands it.
- [ ] `ctrl-f` finds in the focused Side (Enter / F3 / shift-F3); `ctrl-shift-f` searches the Change Set into the bottom panel; **clicking** a result opens it; F3 steps across files.
- [ ] `alt-b` shows Blame Annotations on git Sides; **hovering** one shows its commit; **clicking** opens the commit popup, whose three buttons (show changes, copy hash, annotate previous revision) work — the first and last open new windows.
- [ ] **Toolbar:** every button (☰, ↑ ↓ ← →, ⚙, ?; in a repository also Focus and ◀ ▶) works by click and has a tooltip with its shortcut where it has one; mouse back/forward buttons, Alt+Left/Right and ◀ ▶ go Back/Forward (show a file's history, Back returns to the commit, file and line; Forward goes again); `[` `]` switch files; `?` lists every binding.
- [ ] Switching the system between light and dark while Lenz is open repaints it in IntelliJ Light / Darcula.
- [ ] Hard inputs: a binary file shows its placeholder; a > 20 MB file asks first and **Compare anyway** (click or Enter) diffs it; a latin-1 file shows `[windows-1252]` in the header; a line over 10,000 characters ends in `… N more characters` and shift+wheel scrolls both Sides sideways.
- [ ] `q` quits (so do `ctrl-w`, `ctrl-q`); `lenz` in an empty repository prints `no differences` and exits 0.

## The Repository Window

On this repository, then on a large one (e.g. a blobless clone of the Linux kernel):

- [ ] `lenz log` opens the Overview: the log with its branch graph streams in (first rows at once with a commit-graph file; note the time without one, and that **Ctrl/Cmd+G** offers "Speed up history" and writes it only when pressed). Scrolling the log is smooth.
- [ ] Click / ctrl-click / shift-click commits (one, two, a range); merges show "vs parent 1 of 2" and Ctrl/Cmd+M switches; double-click or `f` / Enter → Focus (Branches and History hidden); Esc → the panels come back; `alt-1/2/3` show/hide Files, Branches, History (Branches and History leave Focus); Tab moves between sidebar, log and diff.
- [ ] `lenz` in a clean repository opens the Repository Window on HEAD; `lenz A B` and `git difftool` open with branches and history hidden (Files shown for several files), and Esc reveals them.
- [ ] Filter: Ctrl/Cmd+/ in the log, typing, `author:` `path:` `after:` `before:`, Enter for full-message search, Esc; Ctrl/Cmd+B branch menu; right-click a commit: compare with working tree, compare with branch…, copy hash/subject, open on GitHub, show in graph.
- [ ] Sidebar: Local changes (Staged / Unstaged / Untracked groups in the File Navigator), branches (click walks one; "Show all"), remotes, tags, worktrees (right-click → open in new window), stashes, typing filters; right-click a branch → Compare with Current, Ctrl/Cmd+T toggles tip ⟷ tip.
- [ ] File history: right-click a file → Show file history, and `lenz log --follow <file>`.
- [ ] **Toolbar (GUI):** F10, `:` and Ctrl/Cmd+Shift+P open the palette (the bottom bar's right end shows the hint "Commands  Ctrl+Shift+P"; no toolbar button); typing filters; Enter runs; a disabled entry shows its reason. **⚙** opens the Diff options popover (rows Layout, Whitespace, Highlight, Scroll, Show, Compare): choices change the diff and the popover stays open; Esc and a click outside close it. The bottom bar's **Branches / History / Files** cells toggle and highlight (Branches and History are disabled in Focus); the toolbar's **Focus** button enters Focus, where **Overview** (or Esc) restores the panels. Markdown header **Source | Inline | Side by side** switches. Right-click a change block: at most eight items, no submenus. Below 640 px the toolbar drops the file arrows and ⚙; the log drops its author, short id and date columns below 800, 640 and 540 px.
- [ ] **Folding:** `x` folds the current multiline change into one tinted row, the gutter chevron toggles it, `shift-x` unfolds all, folds survive `w`/`i`/`h`. Single-line changes have no fold chevron or menu action and do not fold with `x`; a block is multiline if either Side has more than one line.
- [ ] **Images:** a changed PNG/JPEG/SVG opens in 2-up with headers; **wheel zoom and drag** stay in sync; Swipe (**the split follows the pointer**), Onion skin (`,` `.`), Difference with boxed regions, `n`/`p` regions, `0` / `1`, sharp pixels above 300 %.
- [ ] **Open in another app:** `o` opens your editor at the line (set `LENZ_EDITOR` if `$EDITOR` is a wrapper), `shift-o` the default app, a past version opens as a read-only copy.
- [ ] **Pull requests:** `lenz pr <n>` in a clone, and from the sidebar list; header, description (`d`), the log narrowed to the PR, **Viewed boxes** and `v`, review thread dots and their popup (**click the line number**), `T`, `r`; after closing the window `git for-each-ref refs/lenz` is empty.

## The merge tool

In a throwaway repository with a merge conflict in a text file, a binary (PNG) file, a file deleted on one side, a symlink and (optionally) a submodule; git config for `lenz` as in the README's "git mergetool" section, set **in that repository only** while testing.

- [ ] `git mergetool` on the text file opens the Merge View: Ours | Result | Theirs labelled with the branches, one-sided changes already applied, the Conflict red. **Click ≫ / ≪ / ✕ / ↺ in the gutters**; the second arrow appends ("both" in click order); **right-click a block** for the same; `alt-shift-→ ← ↓ backspace`; `n`/`p`, `shift-n`/`shift-p`; **click the counters**.
- [ ] `b` shows the Base in the middle; `w` re-merges keeping choices; `s` Padded lines every block up; the palette's **Resolve simple conflicts** resolves the simple ones with a dotted edge until looked at; `ctrl-z` / `ctrl-shift-z`.
- [ ] Done with a Conflict left asks first; Done → the file is staged by git. Cancel → "merge of … failed", the file still conflicted. **Closing the window with the title bar** after a change asks first.
- [ ] macOS: `python3 scripts/check-mergetool-cancel.py --app <lenz>` (opens one window and quits it like ⌘Q) reports the cancelled merge unresolved. Then by hand: ⌘Q and the Dock's **Quit** after a change keep the file conflicted, and Done still stages it.
- [ ] The binary conflict opens the Version Picker: rows with sizes, the image compare below, **click a row** or `1`–`3`, `b` puts the Base left; Done stages the exact bytes.
- [ ] `lenz diff merge` (and plain `lenz diff`) during the merge opens the Conflict List at the top of the sidebar; **Mark resolved** and **Re-open**; deleted-on-one-side, symlink and submodule files open in the Version Picker (submodule: commands only); after `git merge --continue` in a terminal the entry goes away when the window is focused again.
- [ ] Moving changes: `lenz` with local changes shows ≫ / ≪ in the gutter of each Change Block (not for files under the temp directory); **click them**, `alt-shift-→ / ←`, right-click entries; `ctrl-z` writes the file back; the File Navigator marks "● edited in Lenz".
- [ ] A file saved with Conflicts left, then edited by hand and reopened, resumes (marker-free Conflicts resolved, "edited outside Lenz"); a file changed on disk while open shows the banner on focus.

## Package and publish

Packaging is manual and needs Python 3. Each tarball holds the launcher `lenz` and its four apps, `lenz-code`, `lenz-code-tui`, `lenz-diff` and `lenz-diff-tui`, each built on its own.

- [ ] From the unpacked tarball (and, on macOS, through the Homebrew-style symlinks into `Lenz.app`): `lenz --help` lists the commands; `lenz <folder>` opens the Code Reader; `lenz --tui <file>` opens the terminal reader; `lenz diff <left> <right>` and `lenz diff --tui <left> <right>` open the comparisons; `lenz diff --bogus` prints `usage: lenz diff`.
- [ ] macOS: the Dock shows the Lenz logo from launch to quit for both a Code and a Diff window, launched from Finder (`Lenz.app` opens the Reader) and from a terminal through the symlinks; `codesign -v --strict Lenz.app` passes.
- [ ] Linux: the **Lenz Code** and **Lenz Diff** desktop entries start their apps and group their windows under the right icon.

- [ ] Linux: `packaging/build-linux.sh <version>` builds `x86_64` and `aarch64` tarballs in Ubuntu 22.04 containers (glibc 2.35; `aarch64` runs under qemu-user and is slow). A tarball built directly on a newer distribution needs that distribution's glibc.
- [ ] macOS: `packaging/package.sh <version>` on an Apple Silicon Mac. It builds the `dist` profile (thin LTO, function symbols kept for crash reports, `panic = abort`) and writes `target/package/lenz-<version>-<target>.tar.gz` and its `.sha256`. The GUI goes into an ad-hoc signed `Lenz.app` (Info.plist with the version, `assets/lenz.icns`) with a top-level `lenz` symlink; check `codesign -v --strict` on it and that the Dock shows the logo from launch to quit. The release identifier and Git revision are embedded in crash diagnostics; the version in About comes from the workspace version, not the filename.
- [ ] Copy every tarball and `.sha256` into `target/package/` on one machine, then run `packaging/bump.sh <version>` to point the Homebrew formula (`packaging/homebrew/lenz.rb`) and the AUR `PKGBUILD` (`packaging/aur/PKGBUILD`) at that release's checksums.
- [ ] Upload the tarballs to the GitHub release, then publish the Homebrew formula to the tap and the AUR package.

## Making the repository public

The development history stays in the private `lenz-private` repository. The public `lenz` repository starts from a single commit of the release tree:

- [ ] `git grep -nIE '/home/|ghp_|github_pat|BEGIN .*PRIVATE KEY'` on the release commit finds nothing that should not be public.
- [ ] Rename the private repository to `lenz-private`, create the public `lenz`, and push one commit made from `git archive` of the release commit (or an orphan branch), never this history.
- [ ] Re-check the README links and screenshots on the hosted page.
- [ ] Test strings name no private repositories (`git grep -n 'aantoniadis/'` shows only the public project).
