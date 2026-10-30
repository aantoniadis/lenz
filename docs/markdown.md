# Markdown Preview

![Markdown Preview, Inline: properties, callouts, a code block and a checklist in one reading column, with an inserted word outlined in green, a removed list item struck, change bars and a Markup change pill](images/gui-markdown.png)

Markdown comparisons have three views, chosen in the **Source | Inline | Side by side** switch in the diff header (the same kind of switch as the image modes) or in the command palette. **Source** is the ordinary text diff. **Inline** and **Side by side** are the Preview, a formatted reading view of both versions. Both frontends support it; the terminal specifics are in the [terminal guide](tui.md#markdown-preview).

Source is the default, including for each new independent view; `[view] markdown = "preview"` and `markdown_layout = "inline" | "side-by-side"` in the settings file change the starting view (see the [README](../README.md#settings-and-last-position)). A view keeps its choice across file, commit and pull request switching: ordinary text files use Source, and the next eligible Markdown file restores the Preview. Activating **Show in source** or a source marker chooses Source.

The Preview works for standalone, repository, commit and PR file comparisons, including added and deleted files with an empty opposite side, and in the Version Picker's two-way comparison preview. Markdown is detected by the extensions `.md`, `.markdown`, `.mdown` and `.mkd`, or by `--name`. Images, binary and unreadable files and oversized placeholders keep their own views, as do three-way Merge View panes.

Lenz Code (the code reader) edits Markdown source with a live preview beside it, list continuation and link and heading completion; see [Markdown in Lenz Code](code/README.md#markdown). Lenz Diff's Preview below is a comparison view and stays read-only.

PR conversations reuse the same bounded Markdown parser. The GUI uses the Preview's reading fonts, heading sizes and inline styles in a 30% wider column, shaped asynchronously; terminal conversations use styled cells. Their compact tables, linked image placeholders and timeline cards differ from the file Preview, and they have no image or callout controls. PR descriptions do not enter the comparison's edit or diff state.

## What is rendered

Headings, paragraphs, emphasis, strong and strikethrough text, inline, fenced and indented code, nested ordered, unordered and task lists, quotes, rules, links and tables are formatted natively. Paragraphs wrap; code and wide tables scroll horizontally. Themes apply, and so do the scroll modes in Side by side.

The Obsidian extras are parsed in the shared core, never inside code:

- Callouts (`> [!warning]- Title`) with Obsidian's types and aliases; unknown types show as notes. `-` folds and `+` unfolds. A folded callout containing a change starts open.
- `==highlights==`, `#tags`, wiki links (`[[target]]`, `[[target|alias]]`, `[[target#heading]]`) and footnotes (numbered in order of first use, definitions kept where written and styled as notes).
- Front matter at the start of a file, shown as **Properties**: simple `key: value`, `key: [a, b]` and `- item` lists, indented or not. Nested maps and block strings stay literal.

Marker bytes stay mapped to the source, so Find, Copy Markdown source, Show in source and change markers keep working. HTML and unsupported extensions stay literal text, HTML comments stay hidden, math stays literal (including `$**bold**$` and `$$*math*$$`), and link destinations remain inspectable text. Nothing external is fetched and the Preview never launches a link.

**GUI.** It reads like Obsidian's reading view, colored from the current theme: a centered column 40 em wide (about 70 characters; 600 px at the default font size) and never wider than the pane less 48 px, a proportional body font (`[gui] reading_font`, default the first installed of Inter, Noto Sans, DejaVu Sans, Cantarell and Liberation Sans) two pixels larger than the code font, a heading scale, rounded code blocks with their language, tinted callouts with an icon and title (click a foldable title to fold it), tag and code pills, rounded checkboxes with done items struck and faded, list indent guides, quote bars, bordered tables and footnotes after a rule. External links are underlined in their own color, internal and wiki links are not, and footnote references are bold numbers.

**Terminal.** The same elements use cell styles; callout titles and quote bars take the kind's color, and an untitled callout shows its kind (such as "ⓘ Note").

### Images

Relative image paths that resolve inside the repository are decoded in the background and drawn at their size, scaled to the column. For a directory comparison that root is the compared folder; for two plain files it is the Markdown file's folder; a revision side resolves images where the file is in the working tree. URLs, absolute paths, `data:` images and symlinks leaving the root are refused and keep their alt-text placeholder, as do images beyond 64 per side, files larger than 64 MiB, and images once 64 MiB of decoded pixels are held. Images leave memory (and the GUI's atlas) when the Preview closes or the document changes. The terminal draws them as real pixels in kitty and Ghostty and as colored cells elsewhere.

## Inline and Side by side

`[view] markdown_layout` (default `inline`) sets the layout a Markdown file opens in, and a choice holds for the view across files.

**Inline** is one column. Unchanged blocks appear once. In a changed paragraph, heading, list item or footnote, the removed words stay in place, struck through in red inside a dashed red outline, followed by the inserted words in a solid green outline. Outlines are a shape no Markdown element uses, so a change never looks like a highlight, inline code, a tag or ordinary strikethrough. Removed text is red even where the theme greys deletions (Darcula, Light). Blocks that change as a whole (tables, code, images, properties, and paragraphs rewritten so much that less than a third of the text is unchanged) show the old block struck in red and then the new one. A click or drag selects in one document, so a removed word selects the old text, and Copy, Copy Markdown source and Show in source use that side. Arrow keys move a caret in the new text only. The scroll modes do not apply.

**Side by side** shows both documents in two panes with the scroll modes and the same word outlines (old side dashed red, new side solid green).

In both layouts a 3 px bar left of each changed block marks it as added, modified or removed (added and modified in the theme's status colors, removed in the removed red), and a block that changed as a whole has its bar only: a block added or removed with no counterpart on the other side (a new section, even one written in the same edit as a nearby change) is marked by its bar alone rather than by whichever of its words happen to match. There are no full-width bands or row numbers. A **Markup change** pill marks a source change with no visible formatting effect, once per change in Inline. Next and previous change follow source hunk order in both layouts and scroll a removed block into view. The word-level diff is computed in the background for every change, so outlines appear across the whole document shortly after it opens.

## Source fidelity

The original source diff owns hunk IDs, order, counts, ignored-change options and source ranges. Every source hunk has either a rendered highlight or a **Markup change** marker, so reference targets, hidden comments, formatting delimiters and paragraph reflow all remain reachable. Clicking a marker opens Source at that original hunk; a rendered row is never presented as a source line number.

- **Find and Go to line** use the original source text and order. Visible hits map to the formatted content; a hit in hidden syntax offers **Hidden source match · Show Source**.
- **Copy** copies the displayed text. **Copy Markdown source** copies the enclosing original blocks with their original spelling and line endings rather than rebuilding Markdown from the Preview.
- **Editor and blame** use exact mapped source positions. Normalized entities, escapes and generated glyphs offer **Show in source** when no exact caret exists.
- **PR threads:** GUI context menus offer review threads at a mapped source line. The terminal's PR thread controls show threads for the current file and have no per-line gutter. Viewed marks and PR refresh work as usual.
- Source and Preview selections are independent.

## Read-only Preview

While the Preview is preparing or showing, comparison editing, Apply, Lenz undo/redo and Source caret operations are blocked with **Switch to Source to edit**. Existing Applied and Undone records remain available, also after a repository refresh. File navigation, reload and source inspection keep working, and modal dialogs and context menus take input first. Saving or discarding an existing draft is also available in the Preview.

The guarded Git file actions (**Discard working-tree changes** and **Undo Git discard**) and the Version Picker's whole-version choices, Save, Done, Cancel and undo/redo are unaffected. The Version Picker's comparison preview itself is read-only.

## Preparation and limits

The Preview is prepared lazily on shared background workers. The terminal status shows **Preparing rendered Markdown** until the projection, layout and code syntax have settled. The GUI shapes its layout on a background worker and shows the source with **Preparing Markdown…** until the first layout lands. Later relayouts (width, theme, syntax colors, options) keep the current layout on screen until the replacement is ready, and dragging a window edge relayouts only once the width has held for 100 ms. A source replacement invalidates old results, selections and native resources. Each view keeps at most one parsed pair and one layout per side, and independent views own their documents until closed and do not synchronize edits.

Preparation is limited to 4 MiB of source per side, 100,000 parser events per side, 200,000 mapped text segments per pair and 8 MiB of generated text per side. When a limit is hit, or preparation is cancelled or fails, a complete Source comparison is shown with a notice; partial Preview output is never presented as complete. Cancellation is checked between parser events and cannot interrupt a single parser step.

Core tests and real GUI and terminal input and render tests cover these workflows, including source hunk navigation, exact original bytes, modals and all themes; the terminal's offline PR fixture runs with `cargo test -p lenz-tui --features gui-test markdown_tui_offline_pr` (the `gui-test` feature is for test seams only and must stay out of production TUI builds). Passing assertions do not prove display flicker or latency; see [terminal verification and limits](tui.md#verification-and-capability-limits) and the [benchmark guide](benchmarking.md). Native macOS and live GitHub timing are unverified.
