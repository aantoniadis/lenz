# Context

Lenz — a side-by-side diff viewer, git client and merge tool, in a native window or the terminal, that shows the differences between two versions with IntelliJ-grade clarity.

## Language

**Comparison**:
One pairing of a left and a right version of a file, shown together.
_Avoid_: diff (overloaded — means the algorithm output too)

**Side**:
One of the two versions in a **Comparison** — the left (old) or the right (new).
_Avoid_: pane, panel (those are UI)

**Change Set**:
The collection of **Comparisons** opened at once — e.g. every file changed between two commits.
_Avoid_: patch, diff

**Source**:
Where a **Side** comes from: a git revision (commit, branch, index, working tree) or a plain file on disk.

**Archive Member**:
A file inside a changed zip or tar archive, shown as its own read-only **Comparison** under the archive's path, as if the archive were a folder.
_Avoid_: entry (tar/zip jargon)

**Folder Summary**:
A folder's changed files with the lines each added and removed, shown in place of a **Comparison** when a folder heading is chosen.

**Change Block**:
A contiguous run of lines that differ between the two **Sides** — inserted, deleted, or modified.
_Avoid_: hunk (hunk includes context lines in unified-diff format)

**Inline Change**:
A word- or character-level difference inside the lines of a modified **Change Block**.

**Whitespace Mode**:
How whitespace counts when comparing **Sides**: **Default** (exact), **Trim** (leading/trailing whitespace ignored), **Ignore** (all whitespace ignored), **Ignore + trim chunks** (Ignore, and whitespace-only edges of a **Change Block** dropped).
_Avoid_: ignore policy, comparison policy

**Ignore Case**:
A toggle, combinable with any **Whitespace Mode**, under which differences only in letter case are not differences at all.

**Highlight Level**:
How finely a modified **Change Block** is refined: **by word** (default), **by character**, or **by line** (no **Inline Changes**).
_Avoid_: granularity, highlight policy

**Ignored Change**:
A **Change Block** whose Sides are equal under the current **Whitespace Mode** / **Ignore Case** — still shown, but faded.

**Connector**:
The visual band linking a **Change Block** on the left **Side** to its counterpart on the right.
_Avoid_: ribbon, curve

**Scroll Mode**:
How the two **Sides** move together when scrolling. One of **Shared** (default — both Sides follow one position; a Side pauses while the other passes lines it lacks), **Aligned** (matching lines stay level while each Side scrolls its own lines), **Padded** (**Filler Rows** keep Sides aligned row for row), or **Independent** (each Side scrolls on its own; the other never moves).
_Avoid_: sync mode, alignment mode

**Filler Row**:
An empty row inserted into one **Side** in **Padded** mode so a **Change Block** lines up with its counterpart.
_Avoid_: spacer, padding line

**File Navigator**:
The list of a **Change Set**'s **Comparisons**, grouped by directory, used to move between files.
_Avoid_: file tree, sidebar (UI placement, not the concept)

**Collapsed Region**:
A run of unchanged lines hidden behind a single "N unchanged lines" row, leaving a few context lines around each **Change Block**.
_Avoid_: fold, hunk context

**Blame Annotation**:
Per-line attribution shown beside a git-backed **Side**: which commit, author and date last changed that line, or "Not committed".
_Avoid_: annotate (IntelliJ's verb), blame gutter

**Folded Change Block**:
A **Change Block** the user has folded to a single row on both **Sides** (to set aside what they've reviewed); unfolding shows it again. Distinct from a **Collapsed Region**, which hides unchanged lines.
_Avoid_: collapsed block, hidden change

**Changed Region**:
In a **Comparison** of two images, one area of differing pixels; the image counterpart of a **Change Block** (next/previous change steps through them).
_Avoid_: hunk, diff area

**Viewed Mark**:
A reviewer's personal note that a file of a pull request has been looked at, kept locally; it lapses when the file changes again.
_Avoid_: reviewed, approved (those are GitHub review states Lenz never sets)

**Merge View**:
The 3-way view for resolving a conflicted file: the **Ours** and **Theirs** versions either side of the **Result**, the common **Base** on a toggle.
_Avoid_: merge editor, conflict editor

**Overview**:
The repository view showing Branches, History, Files and a diff preview, where the user chooses what to look at.

**Focus**:
The view for working through one selection: Files and the diff with navigation and diff options. Branches and History are hidden; Esc or Overview returns to the **Overview** at the same place. A window without a repository is always in Focus.
_Avoid_: Focus Mode, zen mode

**Diff Layout**:
How a **Comparison** is laid out: **Split** (the two **Sides** next to each other) or **Unified** (one column).
_Avoid_: presentation (that is Markdown Source / Preview), view mode

**Markdown Source**:
The ordinary text comparison of a Markdown file, with every source change visible. Selected with **Source** in the diff header.

**Preview**:
The rendered reading view of a Markdown comparison, in one of two layouts: **Inline** (changes marked in one column, removed words struck in place) or **Side by side** (the two **Sides** rendered next to each other). Change bars mark each changed block, and every source change stays reachable.
_Avoid_: Rendered (the code's internal name), reading mode

**Selection Title**:
The human name of what a view compares: `main ~ feature/x`, `v0.1.0 (a1b2c3d)`, `#12 Title`.

**Conflict**:
In a **Merge View**, a run of lines both sides changed differently from the **Base**; unresolved until the user accepts ours, theirs, both, or keeps the base. Changes made on one side only (or the same on both) are non-conflicting and start applied in the **Result**.
_Avoid_: hunk, merge conflict region

**Result**:
The middle pane of a **Merge View**: the file being written — the **Base** with non-conflicting changes applied and each **Conflict** as resolved.
_Avoid_: output, merged file (that is git's `$MERGED`, which the Result is saved to)

**Conflicted File**:
A path git left unmerged after a merge, rebase, cherry-pick, revert or stash pop. Opens in a **Merge View** when its versions are text, otherwise in a **Version Picker**; a submodule is shown but resolved with git.
_Avoid_: unmerged path, conflict file

**Conflict List**:
The **Conflicted Files** of the merge, rebase, cherry-pick, revert or stash pop in progress, reached from the **Conflicts** entry at the top of the sidebar and shown in the **File Navigator** with each file's kind and whether it is unresolved, saved clean or marked resolved.
_Avoid_: conflicts set, unmerged list

**Version Picker**:
The view for a **Conflicted File** that can only be resolved whole (binary, symlink, deleted on one side, undecodable or very large): one row per available version — Base, Ours, Theirs, or Keep / Delete — each with a preview and **Take**. The version taken is its **Result**.
_Avoid_: binary merge dialog, side chooser

**Repository Window**:
The Lenz window for browsing a repository, shown as the **Overview** or **Focus**. Commands that name what to compare (`lenz A B`, `git difftool`) open it in **Focus**; `lenz log` opens the **Overview**.
_Avoid_: log window, git window, main window

**Change Set Picker**:
The Branches and History panels of the **Overview**, where the user chooses what to compare: a sidebar of places changes come from (local changes, branches, linked worktrees, stashes, pull requests) and the commit log with its branch graph. Selecting entries there picks a **Change Set**.
_Avoid_: source list (**Source** is one **Side**'s origin), history panel


## Relationships

- A **Change Set** holds one or more **Comparisons**
- A **Comparison** has exactly two **Sides**, each from one **Source**
- A **Comparison** has zero or more **Change Blocks**; a modified **Change Block** has zero or more **Inline Changes**
- Each **Change Block** has exactly one **Connector**
- A **Repository Window** shows one **Change Set** at a time, picked in its **Change Set Picker** (in the **Overview**) or given on the command line

## Flagged ambiguities

- "diff" used for the tool, the algorithm output, and a single file view — resolved: tool is "the viewer", output is **Change Blocks**, single file view is a **Comparison**.
