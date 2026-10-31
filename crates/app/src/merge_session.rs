//! Shared merge loading, resolution, navigation and safe-write decisions.
use crate::{
    conflicts, highlight,
    merge::{BlockState, Labels, MarkerStyle, MergeResult, Pick},
    scrollbar::{Mark, MarkKind, Scrollbar},
    view::{Connector, Content, Kind, Row, ScrollMode, Side, View},
    write::{self, WriteError},
};
use lenz_engine::{Budget, ChangeBlock, MergeOptions, Options, WhitespaceMode};
use std::{
    path::Path,
    sync::{Arc, Weak, mpsc},
};
/// A Merge View ready to show, gathered before the window opens.
pub struct Opened {
    pub result: MergeResult,
    pub labels: Labels,
    /// The labels written on conflict markers: git's own where the file still shows them.
    pub marker_labels: Labels,
    /// The merged file's name as shown (relative to the repository when there is one).
    pub path: String,
    pub snapshot: write::Snapshot,
    pub style: MarkerStyle,
    pub marker_size: usize,
    /// Syntax highlighting language, from the merged file's name.
    pub lang: Option<highlight::Language>,
    /// File-level notices: no base, versions whose encoding or line endings differ from the Result's.
    pub notices: Vec<String>,
    /// git's own marker output for the versions, which the working-tree file is resumed against; `None` outside git.
    pub git_output: Option<String>,
}

/// Something waiting for the user to confirm (pressing the same key again, or Esc to back out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Done with Conflicts left: saving writes markers and git stages them.
    DoneWithConflicts,
    /// Closing with unsaved changes, or (under `git mergetool`) after saving without Done.
    Close,
    /// The file changed on disk while the window was away: `r` reloads, Esc keeps the Result.
    ChangedOnDisk,
    /// A save (or Done) found the file changed on disk: the same key again overwrites, `r` reloads, Esc cancels.
    Overwrite { done: bool },
}

/// Toolbar actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    PrevChange,
    NextChange,
    PrevConflict,
    NextConflict,
    ApplyNonConflicting,
    AllFrom(Pick),
    Undo,
    Redo,
    /// Re-merge from the versions, dropping every choice and what was resumed from the file.
    StartOver,
    /// The file as it now is on disk, resumed afresh.
    Reload,
    Save,
    Done,
    Cancel,
    ToggleBase,
    Whitespace,
    Scroll,
    /// The magic wand: resolve every Conflict whose word-level edits don't overlap.
    ResolveSimple,
    /// Repository Window only: stage the working-tree file.
    MarkResolved,
    Find,
    NextMatch,
    PreviousMatch,
    NextPane,
    ClearFind,
}

/// A gutter control on one block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    Accept(usize, Pick),
    Both(usize),
    KeepBase(usize),
    Reopen(usize),
}

type SyntaxWork = (
    ParseTag,
    mpsc::Receiver<Option<Option<Arc<highlight::Parsed>>>>,
);

pub struct MergeSession {
    pub m: MergeResult,
    find: Option<MergeFind>,
    /// Case, whole word, regex for merge find (seeded from the window when find starts).
    find_options: lenz_content::find::FindOptions,
    focus_pane: usize,
    /// Ours ⟷ Result and Result ⟷ Theirs.
    pub views: [View; 2],
    pub labels: Labels,
    pub marker_labels: Labels,
    pub path: String,
    pub snapshot: write::Snapshot,
    pub style: MarkerStyle,
    pub marker_size: usize,
    /// Changes since the last save.
    pub dirty: bool,
    /// Saved at least once this session.
    pub saved: bool,
    /// The merge block last stepped to or acted on.
    pub current: Option<usize>,
    pub ask: Option<Ask>,
    /// The Base in the middle pane instead of the Result (`b`).
    pub base_shown: bool,
    /// Shared or Padded.
    pub scroll: ScrollMode,
    /// Padded: the row at the viewport top, in rows across all three panes.
    pub padded_top: f32,
    pub options: MergeOptions,
    /// Compact highlights of Ours, Base, Theirs and the Result, and the Result version last parsed.
    pub parsed: [Option<Arc<highlight::Parsed>>; 4],
    pub lang: Option<highlight::Language>,
    pub version: u64,
    pub parsed_version: Option<u64>,
    pub(crate) parsing: [bool; 4],
    /// A parse for a previous file must never colour this one, even at the same Result version.
    pub(crate) parse_id: Arc<()>,
    pub(crate) result_parse_id: Arc<()>,
    syntax_work: Vec<SyntaxWork>,
    /// One `git mergetool` file in its own window (Done and Cancel close it), rather than a file of the
    /// Repository Window's Conflict List.
    pub mergetool: bool,
    pub notices: Vec<String>,
    pub git_output: Option<String>,
    /// The last save failed (red banner); the Result is kept for another try.
    pub error: Option<String>,
    /// "Keep mine" on the changed-on-disk banner: don't raise it again until the next save or reload.
    pub disk_kept: bool,
}

fn lines_of(text: &str) -> Vec<&str> {
    lenz_engine::split_lines(text)
}

fn read(path: &Path) -> Result<crate::encoding::Decoded, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("lenz diff merge: {}: {e}", path.display()))?;
    decode(&bytes, &path.display().to_string())
}

fn decode(bytes: &[u8], what: &str) -> Result<crate::encoding::Decoded, String> {
    if crate::changeset::looks_binary(bytes) {
        return Err(format!(
            "lenz diff merge: {what} is binary; it opens in the Version Picker"
        ));
    }
    lossless(what, crate::encoding::decode(bytes, None))
}

/// Merging on text with `�` in it would write that into the file: the Version Picker takes such files whole.
fn lossless(what: &str, d: crate::encoding::Decoded) -> Result<crate::encoding::Decoded, String> {
    if d.lossy {
        return Err(format!(
            "lenz diff merge: {what} has bytes that aren't valid {}; merging it as text would corrupt them",
            d.encoding
        ));
    }
    Ok(d)
}

/// "Theirs: CRLF · Result keeps LF": how a version's encoding or line endings differ from the Result's.
fn form_notice(
    name: &str,
    d: &crate::encoding::Decoded,
    result: &write::Snapshot,
) -> Option<String> {
    let v = write::Snapshot::missing(Path::new(""), d);
    let encoding =
        |s: &write::Snapshot| format!("{}{}", s.encoding.name(), if s.bom { " BOM" } else { "" });
    let eol = |crlf: bool| if crlf { "CRLF" } else { "LF" }.to_string();
    // ASCII text is the same bytes in every ASCII-compatible encoding
    let same_bytes = d.text.is_ascii()
        && !v.bom
        && !result.bom
        && v.encoding.is_ascii_compatible()
        && result.encoding.is_ascii_compatible();
    let mut differ = Vec::new();
    if !same_bytes && encoding(&v) != encoding(result) {
        differ.push((encoding(&v), encoding(result)));
    }
    if d.text.contains('\n') && v.crlf != result.crlf {
        differ.push((eol(v.crlf), eol(result.crlf)));
    }
    let (version, kept): (Vec<String>, Vec<String>) = differ.into_iter().unzip();
    (!version.is_empty()).then(|| {
        format!(
            "{name}: {} · Result keeps {}",
            version.join(", "),
            kept.join(", ")
        )
    })
}

fn marker_style(style: conflicts::Style) -> MarkerStyle {
    match style {
        conflicts::Style::Merge => MarkerStyle::Merge,
        conflicts::Style::Diff3 => MarkerStyle::Diff3,
        conflicts::Style::Zdiff3 => MarkerStyle::Zdiff3,
    }
}

/// Ours and Theirs as the operation in progress names them, else "Local" / "Remote".
fn operation_labels(root: &Path) -> Labels {
    let (ours, theirs) = match conflicts::in_progress(root) {
        Ok(Some(p)) => (p.ours, p.theirs),
        _ => ("Local".to_string(), "Remote".to_string()),
    };
    Labels {
        ours,
        base: "base".into(),
        theirs,
    }
}

/// How the versions are merged and written: labels, whether there is a base, marker style and size.
struct Setup {
    labels: Labels,
    has_base: bool,
    style: conflicts::Style,
    marker_size: usize,
}

impl Setup {
    pub fn labels(&self) -> [&str; 3] {
        [
            self.labels.ours.as_str(),
            self.labels.base.as_str(),
            self.labels.theirs.as_str(),
        ]
    }
}

/// A Conflicted File of the repository at `root` (its top level), from its index stages: the Repository Window's
/// Conflict List. The Result goes to the working-tree file.
pub fn open_conflict(root: &Path, file: &conflicts::ConflictedFile) -> Result<Opened, String> {
    let target = root.join(&file.path);
    write::check_repository_path(root, &target, false).map_err(|e| e.to_string())?;
    let stage = |n: u8| match &file.stages[n as usize - 1] {
        Some(_) => conflicts::read_stage(root, &file.path, n).and_then(|b| decode(&b, &file.path)),
        None => Ok(crate::encoding::decode(b"", None)),
    };
    let (b, o, t) = (stage(1)?, stage(2)?, stage(3)?);
    let setup = Setup {
        labels: operation_labels(root),
        has_base: file.stages[0].is_some(),
        style: conflicts::conflict_style(root),
        marker_size: conflicts::marker_size(root, &file.path),
    };
    let git_output =
        conflicts::merge_file(root, file, setup.labels(), setup.style, setup.marker_size)
            .ok()
            .map(|(bytes, _)| crate::encoding::decode(&bytes, None).text);
    let mut opened = assemble([b, o, t], &target, file.path.clone(), setup, git_output)?;
    opened.snapshot = opened.snapshot.in_repository(root).map_err(|e| e.to_string())?;
    Ok(opened)
}

/// `lenz diff merge BASE LOCAL REMOTE MERGED`: the three versions git wrote to temp files, the Result to `merged`.
pub fn open_files(
    base: &Path,
    ours: &Path,
    theirs: &Path,
    merged: &Path,
    cwd: &Path,
) -> Result<Opened, String> {
    let (b, o, t) = (read(base)?, read(ours)?, read(theirs)?);
    let root = crate::changeset::toplevel(cwd).ok();
    let rel = root.as_ref().and_then(|r| {
        let merged = std::fs::canonicalize(merged).ok()?;
        merged
            .strip_prefix(std::fs::canonicalize(r).ok()?)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    });
    let name = |p: &Path| {
        p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    };
    let stages = match (&root, &rel) {
        (Some(root), Some(rel)) => conflicts::has_base(root, rel),
        _ => Ok(None),
    };
    // a MERGED git doesn't list as unmerged (files merged by hand in a work tree) is taken as given, as outside git
    let rel = rel.filter(|_| stages != Ok(None));
    let setup = match (&root, &rel) {
        (Some(root), Some(rel)) => Setup {
            labels: operation_labels(root),
            has_base: stages.ok().flatten().unwrap_or(true),
            style: conflicts::conflict_style(root),
            marker_size: conflicts::marker_size(root, rel),
        },
        // outside git: the files as given, an empty BASE being a real empty base
        _ => Setup {
            labels: Labels {
                ours: name(ours),
                base: name(base),
                theirs: name(theirs),
            },
            has_base: true,
            style: conflicts::Style::Merge,
            marker_size: 7,
        },
    };
    // git's output from mergetool's temp files, which are in working-tree form like the file git wrote (the
    // stages' object ids would give index form). Outside git, `$MERGED` is no git merge output to resume from.
    let git_output = rel.as_ref().and_then(|_| {
        let (bytes, _) = conflicts::merge_files(
            cwd,
            [ours, base, theirs],
            setup.labels(),
            setup.style,
            setup.marker_size,
        )
        .ok()?;
        Some(crate::encoding::decode(&bytes, None).text)
    });
    let path = rel.unwrap_or_else(|| {
        merged
            .strip_prefix(cwd)
            .unwrap_or(merged)
            .display()
            .to_string()
    });
    assemble([b, o, t], merged, path, setup, git_output)
}

/// The Merge View of three decoded versions, the Result saved to `merged` (shown as `path`), resumed from what an
/// earlier save or another editor left in that file when it differs from `git_output`.
fn assemble(
    [b, o, t]: [crate::encoding::Decoded; 3],
    merged: &Path,
    path: String,
    setup: Setup,
    git_output: Option<String>,
) -> Result<Opened, String> {
    // the Result keeps the working-tree file's encoding, BOM and line endings; Ours' when there's no file
    let (snapshot, file) = match write::Snapshot::read(merged, None) {
        Ok((s, d)) => (s, Some(lossless(&path, d)?.text)),
        Err(_) if !merged.exists() => (write::Snapshot::missing(merged, &o), None),
        Err(e) => return Err(format!("lenz diff merge: {e}")),
    };
    let style = marker_style(setup.style);
    let (ol, bl, tl) = (lines_of(&o.text), lines_of(&b.text), lines_of(&t.text));
    let mut result = if setup.has_base {
        MergeResult::new(
            &ol,
            &bl,
            &tl,
            lenz_engine::merge_lines(&ol, &bl, &tl, &MergeOptions::default()),
        )
    } else {
        MergeResult::without_base(
            &ol,
            &tl,
            &lenz_engine::compare_lines(&ol, &tl, &Default::default()),
        )
    };
    if let (Some(git), Some(file)) = (&git_output, &file) {
        result.resume(git, file, style, setup.marker_size, &setup.labels);
    }
    let notices = [
        (!setup.has_base).then(|| "Added on both sides — no common base".to_string()),
        form_notice("Ours", &o, &snapshot),
        form_notice("Theirs", &t, &snapshot),
    ];
    let lang = highlight::detect(merged, ol.first().copied().unwrap_or(""));
    // markers as git writes them: read off git's own markers still in the file, else `HEAD` for ours in a repository
    let marker_labels = file
        .as_deref()
        .and_then(|f| crate::merge::marker_labels(f, setup.marker_size))
        .unwrap_or_else(|| {
            let l = setup.labels.clone();
            if git_output.is_some() {
                Labels {
                    ours: "HEAD".into(),
                    ..l
                }
            } else {
                l
            }
        });
    Ok(Opened {
        result,
        labels: setup.labels,
        marker_labels,
        path,
        snapshot,
        style,
        marker_size: setup.marker_size,
        lang,
        notices: notices.into_iter().flatten().collect(),
        git_output,
    })
}

/// Ours ⟷ middle and middle ⟷ Theirs views of `m`, with Inline Changes; the middle pane is the Result, or the
/// Base when `base` is set.
fn build_views(m: &MergeResult, base: bool, rows: f32) -> [View; 2] {
    let n = m.blocks().len();
    let mid = |i: usize| {
        if base {
            m.blocks()[i].base.clone()
        } else {
            m.result_range(i)
        }
    };
    let mid_lines = if base { m.side(None) } else { m.lines() };
    let ours: Vec<_> = (0..n)
        .map(|i| (m.blocks()[i].left.clone(), mid(i)))
        .collect();
    let theirs: Vec<_> = (0..n)
        .map(|i| (mid(i), m.blocks()[i].right.clone()))
        .collect();
    [
        build_view(m, m.side(Some(Pick::Ours)), mid_lines, &ours, rows),
        build_view(m, mid_lines, m.side(Some(Pick::Theirs)), &theirs, rows),
    ]
}

/// One 2-way view whose Change Blocks are the merge blocks at `ranges`.
fn build_view(
    m: &MergeResult,
    left: &[String],
    right: &[String],
    ranges: &[(std::ops::Range<u32>, std::ops::Range<u32>)],
    rows: f32,
) -> View {
    let (l, r): (Vec<&str>, Vec<&str>) = (
        left.iter().map(String::as_str).collect(),
        right.iter().map(String::as_str).collect(),
    );
    let mut blocks: Vec<ChangeBlock> = ranges
        .iter()
        .enumerate()
        .map(|(i, (left, right))| ChangeBlock {
            left: left.clone(),
            right: right.clone(),
            ignored: m.blocks()[i].whitespace_only(),
            inline: None,
        })
        .collect();
    let mut budget = Budget::default();
    for b in &mut blocks {
        if !b.left.is_empty()
            && !b.right.is_empty()
            && let lenz_engine::Refinement::Refined(refined) =
                lenz_engine::refine(&l, &r, b, &Options::default(), &mut budget)
        {
            *b = refined;
        }
    }
    // Build line maps once with the final Inline Changes, instead of allocating all of them twice.
    let mut view = View::new(l.len() as u32, r.len() as u32, &blocks, rows);
    let lens = |s: &[&str]| s.iter().map(|x| x.len()).collect::<Vec<_>>();
    view.attach_line_lengths(&lens(&l), &lens(&r));
    view
}

impl MergeSession {
    /// Keep choices and undo when switching files, but stop work and release reloadable syntax.
    pub fn release_syntax(&mut self) {
        self.parsed = Default::default();
        self.parsed_version = None;
        self.parsing = [false; 4];
        self.parse_id = Arc::new(());
        self.result_parse_id = Arc::new(());
        self.syntax_work.clear();
    }

    pub fn new(o: Opened, rows: f32) -> Self {
        let views = build_views(&o.result, false, rows);
        let current =
            (0..o.result.blocks().len()).find(|&i| o.result.state(i) == BlockState::Unresolved);
        let mut s = MergeSession {
            m: o.result,
            find: None,
            find_options: Default::default(),
            focus_pane: 1,
            views,
            labels: o.labels,
            marker_labels: o.marker_labels,
            path: o.path,
            snapshot: o.snapshot,
            style: o.style,
            marker_size: o.marker_size,
            dirty: false,
            saved: false,
            current,
            ask: None,
            base_shown: false,
            scroll: ScrollMode::Shared,
            padded_top: 0.0,
            options: MergeOptions::default(),
            parsed: Default::default(),
            lang: o.lang,
            version: 0,
            parsed_version: None,
            parsing: [false; 4],
            parse_id: Arc::new(()),
            result_parse_id: Arc::new(()),
            syntax_work: Vec::new(),
            mergetool: false,
            notices: o.notices,
            git_output: o.git_output,
            error: None,
            disk_kept: false,
        };
        if let Some(i) = current {
            s.show_block(i);
        }
        s
    }

    /// Rebuild both views after the Result changed, keeping Ours' line at the anchor.
    pub fn relayout(&mut self) {
        let anchor = self.views[0].anchor_line();
        let rows = self.views[0].viewport_rows();
        self.views = build_views(&self.m, self.base_shown, rows);
        self.views[0].place_left_at(anchor);
        self.sync();
        self.version += 1;
        self.refind();
    }

    /// The middle pane's lines and each block's range in them.
    pub fn mid_lines(&self) -> &[String] {
        if self.base_shown {
            self.m.side(None)
        } else {
            self.m.lines()
        }
    }

    pub fn mid_range(&self, i: usize) -> std::ops::Range<u32> {
        if self.base_shown {
            self.m.blocks()[i].base.clone()
        } else {
            self.m.result_range(i)
        }
    }

    /// `b`: the Base in the middle pane, or the Result again.
    pub fn toggle_base(&mut self) -> bool {
        if !self.m.has_base() {
            return false;
        }
        self.base_shown = !self.base_shown;
        self.relayout();
        true
    }

    /// `w`: the next Whitespace Mode (Default → Trim → Ignore): re-merge, keeping choices on blocks whose text
    /// didn't change.
    pub fn next_whitespace(&mut self) {
        use lenz_engine::WhitespaceMode as W;
        self.options.whitespace = match self.options.whitespace {
            W::Default => W::Trim,
            W::Trim => W::Ignore,
            _ => W::Default,
        };
        let text = |p: Option<Pick>| {
            self.m
                .side(p)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        };
        let (o, b, t) = (text(Some(Pick::Ours)), text(None), text(Some(Pick::Theirs)));
        let mut next = if self.m.has_base() {
            MergeResult::new(
                &o,
                &b,
                &t,
                lenz_engine::merge_lines(&o, &b, &t, &self.options),
            )
        } else {
            let options = Options {
                whitespace: self.options.whitespace,
                ..Options::default()
            };
            MergeResult::without_base(&o, &t, &lenz_engine::compare_lines(&o, &t, &options))
        };
        // blocks resumed from the file may be joined or split differently in the new merge: resume it again, then let
        // the session's choices win where a block's text is unchanged
        if let (Some(git), Ok((_, file))) = (
            &self.git_output,
            write::Snapshot::read(&self.snapshot.path, None),
        ) && !file.lossy
        {
            next.resume(git, &file.text, self.style, self.marker_size, &self.labels);
        }
        self.m.carry_choices_to(&mut next);
        self.dirty |= !self.m.same_output(&next);
        self.m = next;
        self.current = None;
        self.relayout();
    }

    /// `s`: Shared ⟷ Padded (Independent doesn't fit three panes).
    pub fn toggle_scroll(&mut self) {
        let line = self.views[0].anchor_line_on(Side::Right);
        self.scroll = if self.scroll == ScrollMode::Padded {
            ScrollMode::Shared
        } else {
            ScrollMode::Padded
        };
        self.place_mid_at(line);
    }

    /// Put middle line `line` at the anchor in either mode.
    pub fn place_mid_at(&mut self, line: f32) {
        self.views[0].place_line_at(Side::Right, line);
        self.sync();
        if self.scroll == ScrollMode::Padded {
            let rows = self.views[0].viewport_rows();
            let (segs, total) = self.segments();
            let c = segs
                .iter()
                .find(|s| (s.start[1] as f32) + (s.len[1].max(1) as f32) > line)
                .map_or(total as f32, |s| {
                    s.row as f32 + (line - s.start[1] as f32).clamp(0.0, s.len[1] as f32)
                });
            self.padded_top = (c - rows / 3.0).clamp(0.0, total as f32);
        }
    }

    /// Padded layout: equal runs and merge blocks with each pane's start and length, and the first row across
    /// panes; plus the total rows.
    pub fn segments(&self) -> (Vec<Seg3>, u32) {
        let mut segs = Vec::new();
        let (mut at, mut row) = ([0u32; 3], 0u32);
        for i in 0..=self.m.blocks().len() {
            let ends = match self.m.blocks().get(i) {
                Some(b) => [b.left.start, self.mid_range(i).start, b.right.start],
                None => [
                    self.m.side(Some(Pick::Ours)).len() as u32,
                    self.mid_lines().len() as u32,
                    self.m.side(Some(Pick::Theirs)).len() as u32,
                ],
            };
            let gap = ends[0] - at[0];
            if gap > 0 {
                segs.push(Seg3 {
                    block: None,
                    start: at,
                    len: [gap; 3],
                    row,
                });
                row += gap;
            }
            at = ends;
            let Some(b) = self.m.blocks().get(i) else {
                break;
            };
            let (r1, r2) = (self.mid_range(i), &b.right);
            let len = [b.left.len() as u32, r1.len() as u32, r2.len() as u32];
            segs.push(Seg3 {
                block: Some(i),
                start: at,
                len,
                row,
            });
            row += len.into_iter().max().unwrap_or(0).max(1);
            at = [b.left.end, r1.end, r2.end];
        }
        (segs, row)
    }

    /// Rows and Connectors of all three panes, in rows from the viewport top.
    pub fn layout(&self) -> ([Vec<Row>; 3], [Vec<Connector>; 2]) {
        if self.scroll != ScrollMode::Padded {
            let (fa, fb) = (self.views[0].frame(), self.views[1].frame());
            return (
                [fa.left, fa.right, fb.right],
                [fa.connectors, fb.connectors],
            );
        }
        let rows = self.views[0].viewport_rows();
        let top = self.padded_top;
        let (segs, _) = self.segments();
        let mut out: [Vec<Row>; 3] = Default::default();
        let mut connectors: [Vec<Connector>; 2] = Default::default();
        let inline = |p: usize, l: u32| match p {
            0 => self.views[0].inline_on(Side::Left, l),
            1 => self.views[0].inline_on(Side::Right, l),
            _ => self.views[1].inline_on(Side::Right, l),
        };
        for s in segs.iter().filter(|s| {
            s.row as f32 + s.height() as f32 >= top && (s.row as f32) < top + rows + 1.0
        }) {
            for k in 0..s.height() {
                let y = (s.row + k) as f32 - top;
                if y < -1.0 || y > rows + 1.0 {
                    continue;
                }
                for (p, pane) in out.iter_mut().enumerate() {
                    let (content, tint) = if k < s.len[p] {
                        (Content::Line(s.start[p] + k), None)
                    } else {
                        (Content::Filler, s.block.and_then(|i| self.tint(p, i)))
                    };
                    let inline = if let Content::Line(l) = content {
                        inline(p, l)
                    } else {
                        Vec::new()
                    };
                    pane.push(Row {
                        y,
                        content,
                        tint,
                        inline,
                    });
                }
            }
            if let Some(i) = s.block {
                let y = s.row as f32 - top;
                let side = |p: usize| (y, y + s.len[p] as f32);
                for (g, (a, b)) in [(0, 1), (1, 2)].into_iter().enumerate() {
                    connectors[g].push(Connector {
                        left: side(a),
                        right: side(b),
                        kind: Kind::Modified,
                        ignored: false,
                        block: i,
                        folded: false,
                        foldable: false,
                    });
                }
            }
        }
        (out, connectors)
    }

    pub fn tint(&self, pane: usize, i: usize) -> Option<(Kind, bool)> {
        let m = &self.m;
        let b = &m.blocks()[i];
        let c = m.choice(i);
        if m.state(i) == BlockState::Unresolved {
            return Some((Kind::Conflict, false));
        }
        let kind = |side: &std::ops::Range<u32>| match (
            b.base.is_empty() || !m.has_base(),
            side.is_empty(),
        ) {
            (true, _) => Kind::Inserted,
            (_, true) => Kind::Deleted,
            _ => Kind::Modified,
        };
        match pane {
            0 | 2 => {
                let (pick, changed, range) = if pane == 0 {
                    (Pick::Ours, b.left_changed, &b.left)
                } else {
                    (Pick::Theirs, b.right_changed, &b.right)
                };
                changed.then(|| (kind(range), !c.taken.contains(&pick) || b.whitespace_only()))
            }
            _ if self.base_shown => Some((Kind::Modified, true)),
            _ => (!c.taken.is_empty() || c.custom.is_some()).then(|| {
                (
                    if c.custom.is_some() {
                        Kind::Modified
                    } else {
                        kind(&m.result_range(i))
                    },
                    b.whitespace_only(),
                )
            }),
        }
    }

    /// Result ⟷ Theirs follows Ours ⟷ Result at the same Result line.
    pub fn sync(&mut self) {
        let line = self.views[0].anchor_line_on(Side::Right);
        self.views[1].place_line_at(Side::Left, line);
    }

    pub fn set_viewport_rows(&mut self, rows: f32) {
        for v in &mut self.views {
            v.set_viewport_rows(rows);
        }
    }

    pub fn scroll_by(&mut self, rows: f32) {
        if self.scroll == ScrollMode::Padded {
            let (_, total) = self.segments();
            self.padded_top = (self.padded_top + rows).clamp(0.0, total as f32);
            return;
        }
        self.views[0].scroll_by(rows, Side::Left);
        self.sync();
    }

    /// Pane `pane`'s scrollbar (0 Ours, 1 Result or Base, 2 Theirs): the rows its view paints, or in Padded the
    /// rows across all three panes (grown to cover a top scrolled past the end).
    pub fn scrollbar(&self, pane: usize) -> Scrollbar {
        if self.scroll == ScrollMode::Padded {
            let (top, visible) = (f64::from(self.padded_top), f64::from(self.views[0].viewport_rows()));
            return Scrollbar { total: f64::from(self.segments().1).max(top + visible), top, visible };
        }
        let (view, side) = self.pane_view(pane);
        view.scrollbar(side)
    }

    /// Scroll so pane `pane`'s scrollbar top is `top`; the other panes follow. Theirs follows the Result line at the
    /// anchor, so a top inside Theirs lines the Result lacks lands at their start, or just past their end when that
    /// start is no move down.
    pub fn scroll_to(&mut self, pane: usize, top: f64) {
        match (self.scroll, pane) {
            (ScrollMode::Padded, _) => self.scroll_by((top - f64::from(self.padded_top)) as f32),
            // Theirs follows the Result line at the anchor: lead with that line, as a find hit in Theirs does
            (_, 2) => {
                let (down, at) = (top > self.scrollbar(2).top, self.views[0].anchor_line_on(Side::Right));
                self.views[1].scroll_to(Side::Right, top);
                let mid = self.views[1].anchor_line_on(Side::Left);
                // Theirs lines the Result lacks map to the Result line at their start: moving down from there, step
                // just past them
                self.place_mid_at(if down && mid <= at { at + crate::view::PAST_RUN } else { mid });
            }
            _ => {
                let (view, side) = self.pane_view(pane);
                let (rows, _) = view.scroll_units_for(side, top);
                self.scroll_by(rows);
            }
        }
    }

    /// Pane `pane`'s scrollbar marks: per block `Conflict` while unresolved, else `Current` for [`Self::current`],
    /// else `Modified` where the pane changed; then the find hits when `pane` is focused, targets offset by
    /// [`crate::view::FIND_TARGET`].
    pub fn scrollbar_marks(&self, pane: usize) -> Vec<Mark> {
        let padded = (self.scroll == ScrollMode::Padded).then(|| self.segments());
        // each block's (row, rows): Padded has one segment per block, in order
        let spans: Vec<(f64, f64)> = match &padded {
            Some((segs, _)) => segs.iter().filter(|s| s.block.is_some()).map(|s| (f64::from(s.row), f64::from(s.height()))).collect(),
            None => (0..self.m.blocks().len())
                .map(|i| {
                    let range = self.pane_range(pane, i);
                    let row = self.pane_row(pane, range.start);
                    (row, self.pane_row(pane, range.end) - row)
                })
                .collect(),
        };
        let mut marks: Vec<Mark> = spans
            .into_iter()
            .enumerate()
            .filter_map(|(i, (row, rows))| {
                let kind = if self.m.state(i) == BlockState::Unresolved {
                    MarkKind::Conflict
                } else if self.current == Some(i) {
                    MarkKind::Current
                } else {
                    self.tint(pane, i)?;
                    MarkKind::Modified
                };
                Some(Mark { row, rows, kind, target: i })
            })
            .collect();
        let at = self.find.as_ref().and_then(|f| f.search.at);
        marks.extend(self.find_hits(pane).iter().enumerate().map(|(i, (line, _))| Mark {
            row: match &padded { Some(padded) => f64::from(padded_row(padded, pane, *line)), None => self.pane_row(pane, *line) },
            rows: 1.0,
            kind: if at == Some(i) { MarkKind::FindCurrent } else { MarkKind::Find },
            target: crate::view::FIND_TARGET + i,
        }));
        marks
    }

    /// Make block `index` current and show it, as stepping to it does; nothing past the last block.
    pub fn go_to_block(&mut self, index: usize) {
        if index < self.m.blocks().len() { self.show_block(index); }
    }

    /// The view painting pane `pane`, and its Side there.
    fn pane_view(&self, pane: usize) -> (&View, Side) {
        match pane {
            0 => (&self.views[0], Side::Left),
            1 => (&self.views[0], Side::Right),
            _ => (&self.views[1], Side::Right),
        }
    }

    /// Block `i`'s lines in pane `pane`.
    fn pane_range(&self, pane: usize, i: usize) -> std::ops::Range<u32> {
        match pane {
            0 => self.m.blocks()[i].left.clone(),
            1 => self.mid_range(i),
            _ => self.m.blocks()[i].right.clone(),
        }
    }

    /// Pane `pane`'s line `line` in its own view's scrollbar rows (Shared).
    fn pane_row(&self, pane: usize, line: u32) -> f64 {
        let (view, side) = self.pane_view(pane);
        view.display_row(side, line)
    }

    pub fn show_block(&mut self, i: usize) {
        self.current = Some(i);
        self.m.seen(i);
        let r = self.mid_range(i);
        self.place_mid_at(r.start as f32);
    }

    /// The next (or previous) block after the current one, optionally unresolved Conflicts only.
    pub fn step(&mut self, forward: bool, unresolved: bool) -> bool {
        let n = self.m.blocks().len();
        let ok = |i: usize| !unresolved || self.m.state(i) == BlockState::Unresolved;
        let found = match (self.current, forward) {
            (None, true) => (0..n).find(|&i| ok(i)),
            (None, false) => (0..n).rev().find(|&i| ok(i)),
            (Some(c), true) => (c + 1..n).find(|&i| ok(i)),
            (Some(c), false) => (0..c).rev().find(|&i| ok(i)),
        };
        match found {
            Some(i) => {
                self.show_block(i);
                true
            }
            None => false,
        }
    }

    pub fn changed(&mut self, did: bool) -> bool {
        if did {
            self.dirty = true;
            self.ask = None;
            // Start over and its undo change how many blocks there are
            self.current = self.current.filter(|&i| i < self.m.blocks().len());
            self.relayout();
        }
        did
    }

    pub fn control(&mut self, c: Control) -> bool {
        let did = match c {
            Control::Accept(i, p) => self.m.accept(i, p),
            Control::Both(i) => self.m.accept_both(i),
            Control::KeepBase(i) => self.m.keep_base(i),
            Control::Reopen(i) => self.m.reopen(i),
        };
        let (Control::Accept(i, _) | Control::Both(i) | Control::KeepBase(i) | Control::Reopen(i)) =
            c;
        self.current = Some(i);
        self.changed(did)
    }

    pub fn text(&self) -> String {
        self.m
            .render(self.style, self.marker_size, &self.marker_labels)
    }

    /// Writes the Result; refuses (`ChangedOnDisk`) when the file isn't what Lenz last read or wrote.
    pub fn save(&mut self) -> Result<(), WriteError> {
        let snapshot = write::write_text(&self.snapshot, &self.text())?;
        self.snapshot = snapshot;
        self.dirty = false;
        self.saved = true;
        self.ask = None;
        self.error = None;
        self.disk_kept = false;
        Ok(())
    }

    /// **Overwrite**: save over whatever the file holds now.
    pub fn overwrite(&mut self) -> Result<(), WriteError> {
        self.snapshot = self.snapshot.reread()?.0;
        self.save()
    }

    /// A failed save: changed on disk asks Overwrite / Reload / Cancel, anything else is a red banner. The Result
    /// stays as it is either way.
    pub fn refused(&mut self, e: WriteError, done: bool) {
        if e == WriteError::ChangedOnDisk {
            self.ask = Some(Ask::Overwrite { done });
        } else {
            self.ask = None;
            self.error = Some(format!(
                "Not saved (the Result is kept; ctrl-s tries again): {e}"
            ));
        }
    }

    /// **Reload**: the file as it is now, resumed afresh from the versions; the session's choices and undo go.
    pub fn reload(&mut self) -> Result<(), String> {
        let (snapshot, file) = self.snapshot.reread().map_err(|e| e.to_string())?;
        let mut m = self.m.fresh();
        if let Some(file) = file {
            let file = lossless(&snapshot.path.display().to_string(), file)?;
            // outside git the fresh merge as written stands in for git's output
            let git = self
                .git_output
                .clone()
                .unwrap_or_else(|| m.render(self.style, self.marker_size, &self.labels));
            m.resume(&git, &file.text, self.style, self.marker_size, &self.labels);
        }
        self.m = m;
        self.snapshot = snapshot;
        (self.dirty, self.ask, self.error, self.disk_kept) = (false, None, None, false);
        self.current = None;
        self.relayout();
        if let Some(i) =
            (0..self.m.blocks().len()).find(|&i| self.m.state(i) == BlockState::Unresolved)
        {
            self.show_block(i);
        }
        Ok(())
    }

    /// The changed-on-disk banner when the window comes back, unless the user kept their Result already.
    pub fn check_disk(&mut self) -> bool {
        let raise = self.ask.is_none() && !self.disk_kept && self.snapshot.changed();
        if raise {
            self.ask = Some(Ask::ChangedOnDisk);
        }
        raise
    }

    /// Notices for the corner: the file-level ones and how much came from the file.
    pub fn headers(&self) -> [String; 3] {
        let middle = if self.base_shown {
            format!("Base · {} (b: back to the Result)", self.path)
        } else {
            let mut modes = String::new();
            if self.options.whitespace != Default::default() {
                modes += &format!(
                    " · whitespace: {}",
                    whitespace_name(self.options.whitespace)
                );
            }
            if self.scroll == ScrollMode::Padded {
                modes += " · padded";
            }
            format!(
                "Result · {}{}{modes}",
                self.path,
                if self.dirty { " ●" } else { "" }
            )
        };
        [
            format!("Ours · {}", self.labels.ours),
            middle,
            format!("Theirs · {}", self.labels.theirs),
        ]
    }
    pub fn parsed_pane(&self, pane: usize) -> Option<&highlight::Parsed> {
        let slot = match pane {
            0 => 0,
            1 if self.base_shown => 1,
            1 => 3,
            _ => 2,
        };
        if slot == 3 && self.parsed_version != Some(self.version) {
            None
        } else {
            self.parsed[slot].as_deref()
        }
    }
    pub fn notices(&self) -> Vec<String> {
        let mut out = self.notices.clone();
        match self.m.edited_outside() {
            0 => {}
            n => out.push(format!(
                "{n} change{} edited outside Lenz — Start over (ctrl-shift-r) re-merges",
                if n == 1 { "" } else { "s" }
            )),
        }
        out
    }

    /// Counters for the header: "2 conflicts left · 6 changes".
    pub fn counters(&self) -> String {
        let left = self.m.unresolved();
        let changes = self.m.blocks().len();
        let s = |n: usize| if n == 1 { "" } else { "s" };
        if left == 0 {
            format!("all conflicts resolved · {changes} change{}", s(changes))
        } else {
            format!(
                "{left} conflict{} left · {changes} change{}",
                s(left),
                s(changes)
            )
        }
    }
}

/// A run of the Padded layout: an equal run (`block: None`) or a merge block.
#[derive(Debug, Clone, Copy)]
pub struct Seg3 {
    block: Option<usize>,
    start: [u32; 3],
    len: [u32; 3],
    row: u32,
}

/// Pane `pane`'s line `line` in the Padded rows of [`MergeSession::segments`]; past the last line, `total`.
fn padded_row((segs, total): &(Vec<Seg3>, u32), pane: usize, line: u32) -> u32 {
    let i = segs.partition_point(|s| s.start[pane] + s.len[pane] <= line);
    segs.get(i).map_or(*total, |s| s.row + line.saturating_sub(s.start[pane]))
}

impl Seg3 {
    fn height(&self) -> u32 {
        if self.block.is_some() {
            self.len.into_iter().max().unwrap_or(0).max(1)
        } else {
            self.len[0]
        }
    }
}

/// Find one visible line's block in ordered, disjoint ranges, without a full-file line map.
pub fn owner(n: usize, line: u32, range: impl Fn(usize) -> std::ops::Range<u32>) -> Option<usize> {
    let (mut lo, mut hi) = (0, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if range(mid).end <= line {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    (lo < n && range(lo).contains(&line)).then_some(lo)
}

impl MergeSession {
    pub fn changed_by(&mut self, f: impl FnOnce(&mut MergeResult) -> bool) -> bool {
        let did = f(&mut self.m);
        self.changed(did)
    }
}

#[derive(Default)]
pub struct Outcome {
    pub toast: Option<String>,
    pub cross: Option<bool>,
    pub done: bool,
    pub back: bool,
    pub mark: bool,
    pub quit: Option<u8>,
}
impl MergeSession {
    pub fn execute(&mut self, act: Act) -> Outcome {
    crate::diagnostics::breadcrumb(act.label());
        let s = self;
        let mut toast = None;
        let (mut cross, mut done, mut back, mut mark, mut quit) = (None, false, false, false, None);
        match act {
            Act::PrevChange | Act::NextChange => {
                if !s.step(act == Act::NextChange, false) {
                    toast = Some("No more changes".to_string());
                }
            }
            Act::PrevConflict | Act::NextConflict => {
                if !s.step(act == Act::NextConflict, true) {
                    toast = Some(if s.m.unresolved() == 0 {
                        "No conflicts left".into()
                    } else {
                        "No more conflicts this way".into()
                    });
                    // past the file's last Conflict: the next unresolved file of the Conflict List
                    cross = (!s.mergetool).then_some(act == Act::NextConflict);
                }
            }
            Act::Find => s.begin_find(s.find_options),
            Act::NextMatch => {
                if !s.find_step(true) {
                    toast = Some("No find matches".into());
                }
            }
            Act::PreviousMatch => {
                if !s.find_step(false) {
                    toast = Some("No find matches".into());
                }
            }
            Act::NextPane => s.cycle_find_pane(),
            Act::ClearFind => {
                s.dismiss_find();
            }
            Act::MarkResolved => mark = true,
            Act::ApplyNonConflicting => {
                if !s.changed_by(|m| m.apply_non_conflicting()) {
                    toast = Some("Every non-conflicting change is already applied".into());
                }
            }
            Act::AllFrom(p) => {
                s.changed_by(|m| m.take_all(p));
            }
            Act::Undo => {
                if !s.changed_by(|m| m.undo()) {
                    toast = Some("Nothing to undo".into());
                }
            }
            Act::Redo => {
                if !s.changed_by(|m| m.redo()) {
                    toast = Some("Nothing to redo".into());
                }
            }
            Act::ResolveSimple => {
                let open = s.m.unresolved();
                let k = s.m.resolve_simple(lenz_engine::resolve_simple);
                if k > 0 {
                    s.dirty = true;
                    s.ask = None;
                    s.relayout();
                }
                toast = Some(match (k, open) {
                    (_, 0) => "No conflicts left".to_string(),
                    (0, n) => format!(
                        "None of the {n} conflicts is simple enough to resolve automatically"
                    ),
                    (k, n) => format!(
                        "Resolved {k} of {n} conflict{}",
                        if n == 1 { "" } else { "s" }
                    ),
                });
            }
            Act::ToggleBase => {
                if !s.toggle_base() {
                    toast = Some("No common base: added on both sides".into());
                }
            }
            Act::Whitespace => {
                s.next_whitespace();
                toast = Some(format!(
                    "Whitespace: {}",
                    whitespace_name(s.options.whitespace)
                ));
            }
            Act::Scroll => {
                s.toggle_scroll();
                toast = Some(format!("Scroll: {}", scroll_name(s.scroll)));
            }
            Act::StartOver => {
                if !s.changed_by(|m| m.start_over()) {
                    toast = Some("Already the fresh merge".into());
                }
            }
            Act::Reload => match s.reload() {
                Ok(()) => toast = Some(format!("Reloaded {}", s.path)),
                Err(e) => s.error = Some(format!("Not reloaded: {e}")),
            },
            Act::Save => {
                let saved = if s.ask == Some(Ask::Overwrite { done: false }) {
                    s.overwrite()
                } else {
                    s.save()
                };
                match saved {
                    Ok(()) => toast = Some(format!("Saved {}", s.path)),
                    Err(e) => s.refused(e, false),
                }
            }
            Act::Done => {
                let insisted = matches!(
                    s.ask,
                    Some(Ask::DoneWithConflicts | Ask::Overwrite { done: true })
                );
                if s.m.unresolved() > 0 && !insisted {
                    s.ask = Some(Ask::DoneWithConflicts);
                } else {
                    let saved = if s.ask == Some(Ask::Overwrite { done: true }) {
                        s.overwrite()
                    } else {
                        s.save()
                    };
                    match saved {
                        Ok(()) if s.mergetool => {
                            quit = Some(0);
                        }
                        Ok(()) => {
                            s.ask = None;
                            toast = Some(format!("Saved {}", s.path));
                            done = true;
                        }
                        Err(e) => s.refused(e, true),
                    }
                }
            }
            Act::Cancel => {
                let warn = s.dirty || (s.saved && s.mergetool);
                if warn && s.ask != Some(Ask::Close) {
                    s.ask = Some(Ask::Close);
                } else if s.mergetool {
                    quit = Some(1);
                } else {
                    back = true;
                }
            }
        }
        Outcome {
            toast,
            cross,
            done,
            back,
            mark,
            quit,
        }
    }
    pub fn dismiss(&mut self) -> bool {
        if self.ask.is_none() && self.dismiss_find() {
            return true;
        }
        if self.ask.is_some_and(|a| a != Ask::Close) || (self.ask.is_none() && self.error.is_some())
        {
            self.disk_kept |= self.ask == Some(Ask::ChangedOnDisk);
            if self.ask.take().is_none() {
                self.error = None;
            }
            true
        } else {
            false
        }
    }
    pub fn may_close(&mut self) -> bool {
        if !(self.dirty || (self.saved && self.mergetool)) || self.ask == Some(Ask::Close) {
            true
        } else {
            self.ask = Some(Ask::Close);
            false
        }
    }
}

fn whitespace_name(mode: WhitespaceMode) -> &'static str {
    match mode {
        WhitespaceMode::Default => "compare all",
        WhitespaceMode::Trim => "trim",
        WhitespaceMode::Ignore => "ignore",
        WhitespaceMode::IgnoreTrimChunks => "ignore + trim chunks",
    }
}

fn scroll_name(mode: ScrollMode) -> &'static str {
    match mode {
        ScrollMode::Shared => "shared",
        ScrollMode::Aligned => "aligned",
        ScrollMode::Padded => "padded",
        ScrollMode::Independent => "independent",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::git::testing::{commit_all, git, repo, try_git, write};

    #[test]
    fn visible_line_ownership_handles_gaps_insertions_and_deletions() {
        let ranges = [0..0, 2..4, 4..4, 4..6, 9..11, 12..12];
        for line in 0..15 {
            assert_eq!(
                owner(ranges.len(), line, |i| ranges[i].clone()),
                ranges.iter().position(|r| r.contains(&line))
            );
        }
        assert_eq!(owner(0, 0, |_| unreachable!()), None);
    }

    /// A conflicted `f.txt` on `main` merging `feature`, its stages written out as `git mergetool` would.
    fn conflicted(base: Option<&str>) -> (tempfile::TempDir, [PathBuf; 3]) {
        conflicted_in(
            repo(),
            base.map(str::as_bytes),
            b"ONE\nOURS\nthree\nfour\n",
            b"one\nTHEIRS\nthree\nfour\n",
        )
    }

    /// [`conflicted`] with these versions, in `dir` (a repository on `main`, configured as the test needs).
    fn conflicted_in(
        dir: tempfile::TempDir,
        base: Option<&[u8]>,
        ours: &[u8],
        theirs: &[u8],
    ) -> (tempfile::TempDir, [PathBuf; 3]) {
        let d = dir.path();
        if let Some(base) = base {
            write(d, "f.txt", base);
        } else {
            write(d, "other", "x\n");
        }
        commit_all(d, "base");
        git(d, &["checkout", "-q", "-b", "feature"]);
        write(d, "f.txt", theirs);
        commit_all(d, "theirs");
        git(d, &["checkout", "-q", "main"]);
        write(d, "f.txt", ours);
        commit_all(d, "ours");
        assert!(!try_git(d, &["merge", "-q", "feature"]));
        let stage = |n: u8| {
            let p = d.join(format!("stage{n}"));
            std::fs::write(
                &p,
                crate::conflicts::read_stage(d, "f.txt", n).unwrap_or_default(),
            )
            .unwrap();
            p
        };
        let stages = [stage(1), stage(2), stage(3)];
        (dir, stages)
    }

    #[test]
    fn repository_merge_rejects_symlink_targets_before_open_save_and_reload() {
        let (dir, [base, ours, theirs]) = conflicted(Some("one\ntwo\nthree\nfour\n"));
        let path = dir.path().join("f.txt");
        let file = conflicts::list(dir.path()).unwrap().remove(0);
        let opened = open_conflict(dir.path(), &file).unwrap();
        let mut session = MergeSession::new(opened, 40.0);
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("keep.txt");
        let original = std::fs::read(&path).unwrap();
        std::fs::write(&target, &original).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(open_conflict(dir.path(), &file).is_err());
        assert!(session.save().is_err());
        assert!(session.overwrite().is_err());
        assert!(session.reload().is_err());
        assert_eq!(std::fs::read(&target).unwrap(), original);
        // An explicitly supplied standalone merge output retains its symlink semantics.
        let opened = open_files(&base, &ours, &theirs, &path, dir.path()).unwrap();
        MergeSession::new(opened, 40.0).save().unwrap();
        assert!(std::fs::symlink_metadata(&path).unwrap().is_symlink());
    }

    #[test]
    fn opening_a_mergetool_file_labels_the_merge_and_applies_one_sided_changes() {
        let (dir, [b, o, t]) = conflicted(Some("one\ntwo\nthree\nfour\n"));
        let d = dir.path();
        let opened = open_files(&b, &o, &t, &d.join("f.txt"), d).unwrap();
        assert_eq!(
            (opened.labels.ours.as_str(), opened.labels.theirs.as_str()),
            ("main", "feature")
        );
        assert_eq!(opened.path, "f.txt");
        assert!(opened.result.has_base());
        assert_eq!(opened.result.unresolved(), 1);
        // "one" → "ONE" was ours alone (adjacent to the Conflict, so it joins it, as in git)
        let mut m = opened.result;
        m.accept(0, Pick::Theirs);
        assert_eq!(m.lines().join("\n"), "one\nTHEIRS\nthree\nfour\n");
    }

    #[test]
    fn a_file_added_on_both_sides_merges_without_a_base() {
        let (dir, [b, o, t]) = conflicted(None);
        let d = dir.path();
        let opened = open_files(&b, &o, &t, &d.join("f.txt"), d).unwrap();
        assert!(!opened.result.has_base());
        // equal lines settle; the differing run is one Conflict that shows nothing until resolved
        assert_eq!(opened.result.lines().join("\n"), "three\nfour\n");
        assert_eq!(opened.result.unresolved(), 1);
    }

    fn state(base: &str, ours: &str, theirs: &str) -> (tempfile::TempDir, MergeSession) {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let p = |n: &str, text: &str| {
            let p = d.join(n);
            std::fs::write(&p, text).unwrap();
            p
        };
        let (b, o, t, m) = (p("b", base), p("o", ours), p("t", theirs), p("m", ""));
        let opened = open_files(&b, &o, &t, &m, d).unwrap();
        (dir, MergeSession::new(opened, 40.0))
    }

    #[test]
    fn releasing_hidden_syntax_keeps_choices_and_undo() {
        let (_d, mut s) = state("base", "ours", "theirs");
        let old_job = Arc::downgrade(&s.parse_id);
        s.control(Control::Accept(s.current.unwrap(), Pick::Theirs));
        s.release_syntax();
        assert_eq!(old_job.strong_count(), 0);
        assert_eq!(s.m.lines(), ["theirs"]);
        assert!(s.dirty);
        assert!(s.m.undo());
        assert_eq!(s.m.lines(), ["base"]);
        assert_eq!(s.m.unresolved(), 1);
    }

    #[test]
    fn a_shared_added_eof_newline_keeps_merge_blocks_and_controls_aligned() {
        for (base, ours, theirs, resolved) in [
            ("a\nsame", "b\nsame\n", "c\nsame\n", "c\nsame\n"),
            ("a\nsame\n", "b\nsame\n\n", "c\nsame\n\n", "c\nsame\n\n"),
        ] {
            let (_d, mut s) = state(base, ours, theirs);
            assert_eq!(s.m.unresolved(), 1);
            for view in &s.views {
                assert_eq!(view.block_ranges().len(), s.m.blocks().len());
            }
            let conflict = s.current.unwrap();
            s.control(Control::Accept(conflict, Pick::Theirs));
            assert_eq!(s.m.lines().join("\n"), resolved);
            assert_eq!(s.m.unresolved(), 0);
            assert!(s.m.undo());
            assert_eq!(s.m.unresolved(), 1);
        }
    }

    /// 60 lines: Ours alone changes line 5, both change line 30 (the Conflict), Theirs alone changes line 45.
    fn three_blocks() -> (tempfile::TempDir, MergeSession) {
        let base: String = (0..60).map(|i| format!("l{i}\n")).collect();
        let ours = base.replace("l5\n", "O5\n").replace("l30\n", "O30\n");
        let theirs = base.replace("l30\n", "T30\n").replace("l45\n", "T45\n");
        state(&base, &ours, &theirs)
    }

    #[test]
    fn merge_marks_show_unresolved_conflicts_first() {
        use crate::{scrollbar::{place, MarkKind}, view::FIND_TARGET};
        let (_d, mut s) = three_blocks();
        assert_eq!((s.m.blocks().len(), s.m.unresolved(), s.current), (3, 1, Some(1)));
        let kinds = |s: &MergeSession, pane| s.scrollbar_marks(pane).iter().map(|m| (m.target, m.kind)).collect::<Vec<_>>();
        // the current block is the Conflict, which outranks it
        assert_eq!(kinds(&s, 1), [(0, MarkKind::Modified), (1, MarkKind::Conflict), (2, MarkKind::Modified)]);
        assert_eq!(kinds(&s, 0), [(0, MarkKind::Modified), (1, MarkKind::Conflict)], "Ours is unchanged at Theirs' block");
        s.go_to_block(2);
        assert_eq!(kinds(&s, 2), [(1, MarkKind::Conflict), (2, MarkKind::Current)]);
        let marks = s.scrollbar_marks(1);
        assert_eq!(place(&marks, s.scrollbar(1).total, 1), [Some(1)], "the Conflict wins a shared cell");
        s.focus_pane(2);
        s.begin_find(Default::default());
        s.type_find("T");
        let finds = |pane| s.scrollbar_marks(pane).into_iter().filter(|m| m.target >= FIND_TARGET).map(|m| (m.row, m.kind, m.target - FIND_TARGET)).collect::<Vec<_>>();
        assert_eq!(finds(2), [(30.0, MarkKind::FindCurrent, 0), (45.0, MarkKind::Find, 1)]);
        assert!(finds(0).is_empty() && finds(1).is_empty(), "find hits mark the focused pane only");
    }

    #[test]
    fn merge_scroll_to_lands_on_each_pane_past_a_theirs_insertion() {
        // Theirs inserts 20 lines after line 10 (taken into the Result) and changes line 50 into 21 lines where Ours
        // changes it too (the Conflict, so the Result lacks most of them): Theirs runs ahead of the Result
        let base: String = (0..100).map(|i| format!("l{i}\n")).collect();
        let lines = |tag: &str| (0..20).map(|i| format!("{tag}{i}\n")).collect::<String>();
        let ours = base.replace("l50\n", "O50\n");
        let theirs = base.replace("l10\n", &format!("l10\n{}", lines("A"))).replace("l50\n", &format!("T50\n{}", lines("B")));
        let (_d, mut s) = state(&base, &ours, &theirs);
        assert_eq!(s.m.unresolved(), 1);
        s.set_viewport_rows(10.0);
        for padded in [false, true] {
            for pane in 0..3 {
                for top in [5.0, 28.0, 41.5, 60.0, 77.5, 85.0] {
                    s.scroll_to(pane, top);
                    let landed = s.scrollbar(pane).top;
                    assert!((landed - top).abs() <= 1.0, "padded={padded} pane {pane}: {top} landed at {landed}");
                }
            }
            // scrolled as far as it goes, past `total - visible`: the thumb ends at the track end
            s.scroll_by(f32::INFINITY);
            for pane in 0..3 {
                let bar = s.scrollbar(pane);
                assert!(bar.page(true) >= bar.top, "padded={padded} pane {pane}: {bar:?}");
                let (start, size) = bar.thumb(100.0, 4.0).unwrap();
                assert!((start + size - 100.0).abs() < 1e-3, "padded={padded} pane {pane}: {bar:?}");
            }
            s.toggle_scroll();
        }
    }

    #[test]
    fn theirs_pages_past_a_long_run_the_result_lacks() {
        // Ours deletes line 30 where Theirs turns it into 30 lines; taking Ours leaves the Result without them
        let base: String = (0..60).map(|i| format!("l{i}\n")).collect();
        let run: String = (0..30).map(|i| format!("T{i}\n")).collect();
        let (_d, mut s) = state(&base, &base.replace("l30\n", ""), &base.replace("l30\n", &run));
        let i = s.current.unwrap();
        s.control(Control::Accept(i, Pick::Ours));
        assert_eq!((s.mid_range(i).len(), s.m.blocks()[i].right.len()), (0, 30));
        s.set_viewport_rows(10.0);
        let s = std::cell::RefCell::new(s);
        crate::view::tests::page_through("Theirs", || s.borrow().scrollbar(2), |top| s.borrow_mut().scroll_to(2, top));
    }

    #[test]
    fn merge_scroll_to_and_go_to_block() {
        let (_d, mut s) = three_blocks();
        s.set_viewport_rows(10.0);
        let tops = |s: &MergeSession| [0, 1, 2].map(|p| s.scrollbar(p).top);
        for pane in 0..3 {
            s.scroll_to(pane, 20.0);
            assert!((s.scrollbar(pane).top - 20.0).abs() < 1e-3, "pane {pane}: {:?}", tops(&s));
            assert!(tops(&s).iter().all(|t| (t - 20.0).abs() <= 1.0), "the other panes follow: {:?}", tops(&s));
            s.scroll_to(pane, 0.0);
            assert_eq!(tops(&s), [0.0; 3]);
        }
        s.go_to_block(2);
        assert_eq!(s.current, Some(2));
        let (bar, row) = (s.scrollbar(1), s.views[0].display_row(Side::Right, s.mid_range(2).start));
        assert!(bar.top <= row && row < bar.top + bar.visible, "{bar:?} {row}");
        s.go_to_block(3);
        assert_eq!(s.current, Some(2), "there is no block 3");
        s.toggle_scroll();
        assert_eq!(s.scrollbar(0).total, s.segments().1 as f64);
        s.scroll_to(2, 15.0);
        assert_eq!((tops(&s), s.padded_top), ([15.0; 3], 15.0), "Padded panes share one top");
        s.go_to_block(0);
        let (bar, mark) = (s.scrollbar(0), s.scrollbar_marks(0)[0]);
        assert_eq!((s.current, mark.target), (Some(0), 0));
        assert!(bar.top <= mark.row && mark.row < bar.top + bar.visible, "{bar:?} {mark:?}");
    }

    #[test]
    fn padded_rows_line_up_each_block_at_its_tallest_version() {
        let (_d, mut s) = state("a\nb\nc\nd\n", "a\nB1\nB2\nB3\nc\nd\n", "a\nb\nc\nD\n");
        let (segs, total) = s.segments();
        // a | b→B1 B2 B3 (3 rows) | c | d→D | trailing empty line
        let shape: Vec<_> = segs
            .iter()
            .map(|g| (g.block.is_some(), g.len, g.height()))
            .collect();
        assert_eq!(
            shape,
            [
                (false, [1, 1, 1], 1),
                (true, [3, 3, 1], 3),
                (false, [1, 1, 1], 1),
                (true, [1, 1, 1], 1),
                (false, [1, 1, 1], 1)
            ]
        );
        assert_eq!(total, 7);
        // the Base in the middle: the block is as tall as Ours, the Base's one line padded
        assert!(s.toggle_base());
        let (segs, _) = s.segments();
        assert_eq!(segs[1].len, [3, 1, 1]);
        s.toggle_scroll();
        let (rows, connectors) = s.layout();
        assert_eq!(
            rows[1]
                .iter()
                .filter(|r| r.content == Content::Filler)
                .count(),
            2
        );
        assert_eq!(connectors[0].len(), 2);
    }

    #[test]
    fn a_whitespace_re_merge_keeps_choices_on_unchanged_blocks() {
        let (_d, mut s) = state("x\n\ny\n", "X\n\ny \n", "x2\n\ny\n");
        assert_eq!(s.m.unresolved(), 1);
        s.m.accept(0, Pick::Theirs);
        s.next_whitespace(); // Trim: the trailing space on "y " is now a whitespace-only change
        assert_eq!(s.m.choice(0).taken, vec![Pick::Theirs]);
        assert_eq!(s.m.lines().join("\n"), "x2\n\ny \n");
    }

    #[test]
    fn whitespace_keeps_repeated_conflict_choices_separate() {
        for choices in [[Some(Pick::Ours), None], [None, Some(Pick::Theirs)], [Some(Pick::Ours), Some(Pick::Theirs)]] {
            let (_dir, mut s) = state("base\nseparator\nbase\n", "ours\nseparator\nours\n", "theirs\nseparator\ntheirs\n");
            assert_eq!(s.m.blocks().len(), 2);
            for (i, pick) in choices.into_iter().enumerate() {
                if let Some(pick) = pick { s.control(Control::Accept(i, pick)); }
            }
            let expected = s.text();
            let unresolved = s.m.unresolved();
            for _ in 0..3 {
                s.execute(Act::Whitespace);
                assert_eq!(s.m.unresolved(), unresolved);
                assert_eq!(s.text(), expected);
            }
            s.save().unwrap();
            assert_eq!(std::fs::read_to_string(&s.snapshot.path).unwrap(), expected);
        }
    }

    #[test]
    fn whitespace_output_changes_remain_unsaved_until_saved() {
        let (_dir,mut s)=state("a\n"," a\n","a\nb\n");
        s.execute(Act::AllFrom(Pick::Theirs));s.save().unwrap();
        assert_eq!(s.text(),"a\nb\n");assert!(!s.dirty);
        s.execute(Act::Whitespace);
        assert_eq!(s.text()," a\nb\n");assert!(s.dirty);
        let out=s.execute(Act::Cancel);assert!(!out.back);assert_eq!(s.ask,Some(Ask::Close));
        s.ask=None;s.save().unwrap();assert!(!s.dirty);
        assert_eq!(std::fs::read_to_string(&s.snapshot.path).unwrap()," a\nb\n");
        s.execute(Act::Whitespace);assert!(!s.dirty,"unchanged output must stay saved");
    }

    #[test]
    fn merge_find_takes_and_returns_the_window_options() {
        use lenz_content::find::{FindOption, FindOptions};
        let (_dir, mut s) = state("base (x\n", "ours (x\n", "theirs (x\n");
        s.begin_find(FindOptions { case: true, ..Default::default() });
        s.type_find("(");
        assert_eq!(s.find_error(), None, "literal");
        assert_eq!(s.toggle_find_option(FindOption::Regex), FindOptions { case: true, regex: true, word: false });
        assert_eq!(s.find_error(), Some("unclosed group"));
        assert_eq!(s.find_count(), 0);
        assert_eq!(s.find_options(), FindOptions { case: true, regex: true, word: false });
        let notice = s.find_notice().unwrap();
        assert!(notice.starts_with("Find Result [Aa .*]: (") && notice.contains("· unclosed group ·"), "{notice}");
    }

    #[test]
    fn merge_find_tracks_result_choices_unicode_and_base_without_copying_documents() {
        let (_dir, mut s) = state(
            "base\nÄrger base\n",
            "ours\nÄrger ours\n",
            "theirs\nÄrger theirs\n",
        );
        s.begin_find(Default::default());
        s.type_find("ärGER");
        assert_eq!(s.find_pane(), 1);
        assert_eq!(s.find_ranges(1, 1), vec![0..6]);
        s.finish_find();
        s.execute(Act::AllFrom(Pick::Theirs));
        s.begin_find(Default::default());
        s.clear_find_query();
        s.type_find("theirs");
        assert_eq!(s.find_count(), 2);
        s.execute(Act::ToggleBase);
        assert_eq!(s.find_count(), 0);
        s.cycle_find_pane();
        assert_eq!(s.find_pane(), 2);
        assert_eq!(s.find_count(), 2);
        s.find_step(true);
        assert_eq!(s.find_ranges(2, 0), vec![0..6]);
        assert!(s.dismiss_find());
        assert_eq!(s.find_count(), 0);
    }

    /// Opening a 100k-line merge and one action (which re-lays out both views) stay interactive.
    #[test]
    #[ignore = "timing; run by scripts/perf-check.sh in release"]
    pub(crate) fn merge_view_perf() {
        let base: String = (0..100_000)
            .map(|i| format!("let v{i} = compute({i}, {});\n", i % 13))
            .collect();
        let edit = |every: usize, tag: &str| -> String {
            base.lines()
                .enumerate()
                .map(|(i, l)| {
                    if i % every == 0 {
                        format!("{l} // {tag}\n")
                    } else {
                        format!("{l}\n")
                    }
                })
                .collect()
        };
        let (ours, theirs) = (edit(89, "ours"), edit(97, "theirs"));
        let t = std::time::Instant::now();
        let (_d, mut s) = state(&base, &ours, &theirs);
        let open = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        s.control(Control::Accept(s.current.unwrap(), Pick::Theirs));
        let act = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "merge view 100k: open {open:.0} ms, one action {act:.0} ms ({} blocks)",
            s.m.blocks().len()
        );
        crate::perf::record("merge/open", "ms", &[open]);
        crate::perf::record("merge/accept-rebuild", "ms", &[act]);
        crate::perf::check(
            open < 600.0 && act < 60.0,
            "merge open/action exceeds 600/60 ms",
        );
    }

    #[test]
    fn a_conflict_list_file_opens_from_its_index_stages() {
        let (dir, _) = conflicted(Some("one\ntwo\nthree\nfour\n"));
        let d = dir.path();
        let files = conflicts::list(d).unwrap();
        let opened = open_conflict(d, &files[0]).unwrap();
        assert_eq!(
            (opened.labels.ours.as_str(), opened.labels.theirs.as_str()),
            ("main", "feature")
        );
        assert_eq!(opened.path, "f.txt");
        assert!(opened.result.has_base());
        assert_eq!(opened.result.unresolved(), 1);
        // the Result goes to the working-tree file, git's markers and all, as found
        assert_eq!(opened.snapshot.path, d.join("f.txt"));
        let mut m = opened.result;
        m.accept(0, Pick::Ours);
        assert_eq!(m.lines().join("\n"), "ONE\nOURS\nthree\nfour\n");
    }

    #[test]
    fn a_conflict_list_file_added_on_both_sides_has_no_base() {
        let (dir, _) = conflicted(None);
        let d = dir.path();
        let files = conflicts::list(d).unwrap();
        let f = files.iter().find(|f| f.path == "f.txt").unwrap();
        let opened = open_conflict(d, f).unwrap();
        assert!(!opened.result.has_base());
        assert_eq!(opened.result.lines().join("\n"), "three\nfour\n");
    }

    #[test]
    fn outside_git_the_files_name_the_versions() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        let p = |n: &str, text: &str| {
            let p = d.join(n);
            std::fs::write(&p, text).unwrap();
            p
        };
        let (b, o, t, m) = (
            p("base.txt", "a\n"),
            p("mine.txt", "b\n"),
            p("yours.txt", "c\n"),
            p("out.txt", ""),
        );
        let opened = open_files(&b, &o, &t, &m, d).unwrap();
        assert_eq!(
            (opened.labels.ours.as_str(), opened.labels.theirs.as_str()),
            ("mine.txt", "yours.txt")
        );
        assert_eq!(opened.result.unresolved(), 1);
        // outside git the (empty) output file is no merge to resume from
        assert_eq!(
            (opened.git_output, opened.result.edited_outside()),
            (None, 0)
        );
    }

    #[test]
    fn files_merged_by_hand_in_a_work_tree_keep_their_base() {
        // MERGED isn't unmerged in git (here a tracked copy of LOCAL): git has no stages to say there's no base
        let dir = repo();
        let d = dir.path();
        let p = |n: &str, text: &str| {
            write(d, n, text);
            d.join(n)
        };
        let (b, o, t) = (
            p("base.txt", "a\nb\nc\nd\ne\nf\ng\n"),
            p("local.txt", "a\nL\nc\nd\ne\nX\ng\n"),
            p("remote.txt", "a\nb\nc\nR\ne\nY\ng\n"),
        );
        let m = p("merged.txt", "a\nL\nc\nd\ne\nX\ng\n");
        commit_all(d, "files");
        let opened = open_files(&b, &o, &t, &m, d).unwrap();
        assert!(opened.result.has_base());
        assert_eq!((opened.result.unresolved(), opened.git_output), (1, None));
        assert!(opened.notices.is_empty(), "{:?}", opened.notices);
        let mut m = opened.result;
        let i = (0..m.blocks().len()).find(|&i| m.is_conflict(i)).unwrap();
        m.accept_both(i);
        assert_eq!(m.lines().join("\n"), "a\nL\nc\nR\ne\nX\nY\ng\n");
    }

    const BASE: &str = "one\ntwo\nthree\nfour\n";

    #[test]
    fn block_on_row_finds_the_change_under_a_pane_row() {
        let d = tempfile::tempdir().unwrap();
        let paths = [("b", "a\nb\nc\n"), ("o", "a\nX\nc\n"), ("t", "a\nY\nc\n")].map(|(n, t)| { let p = d.path().join(n); std::fs::write(&p, t).unwrap(); p });
        std::fs::write(d.path().join("f.txt"), "a\nX\nc\n").unwrap();
        let s = opened_state(d.path(), &paths);
        for pane in 0..3 {
            let i = s.block_on_row(pane, 1).expect("the conflict on row 1");
            assert!(s.m.is_conflict(i), "pane {pane}");
            assert_eq!(s.block_on_row(pane, 0), None, "an unchanged line");
            assert_eq!(s.block_on_row(pane, 99), None, "below the text");
        }
        assert_eq!(s.block_on_row(7, 1), None, "no such pane");
    }

    #[test]
    fn block_on_row_ignores_rows_below_the_viewport() {
        let d = tempfile::tempdir().unwrap();
        let paths = [("b", "a\nb\nc\n"), ("o", "a\nb\nX\n"), ("t", "a\nb\nY\n")].map(|(n, t)| { let p = d.path().join(n); std::fs::write(&p, t).unwrap(); p });
        std::fs::write(d.path().join("f.txt"), "a\nb\nX\n").unwrap();
        let mut s = opened_state(d.path(), &paths);
        assert!(s.block_on_row(1, 2).is_some(), "the conflict on row 2 while it is shown");
        s.set_viewport_rows(2.0);
        assert_eq!(s.block_on_row(1, 2), None, "row 2 is not drawn in a two-row viewport");
    }

    fn opened_state(d: &Path, [b, o, t]: &[PathBuf; 3]) -> MergeSession {
        MergeSession::new(open_files(b, o, t, &d.join("f.txt"), d).unwrap(), 40.0)
    }

    #[test]
    fn markers_follow_the_user_s_style_and_the_marker_size_attribute_and_git_s_own_file_resumes_to_nothing()
     {
        let dir = repo();
        git(dir.path(), &["config", "merge.conflictStyle", "diff3"]);
        write(
            dir.path(),
            ".gitattributes",
            "f.txt conflict-marker-size=12\n",
        );
        let (dir, stages) = conflicted_in(
            dir,
            Some(BASE.as_bytes()),
            b"ONE\nOURS\nthree\nfour\n",
            b"one\nTHEIRS\nthree\nfour\n",
        );
        let d = dir.path();
        let s = opened_state(d, &stages);
        assert_eq!((s.style, s.marker_size), (MarkerStyle::Diff3, 12));
        // git labelled its markers HEAD and the base commit: that alone isn't an edit
        assert!(
            std::fs::read_to_string(d.join("f.txt"))
                .unwrap()
                .contains("<<<<<<<<<<<< HEAD\n")
        );
        assert_eq!((s.m.unresolved(), s.m.edited_outside()), (1, 0));
        // saved untouched, the file is exactly what git wrote: git's own labels on the markers
        assert_eq!(s.text(), std::fs::read_to_string(d.join("f.txt")).unwrap());
    }

    #[test]
    fn a_file_resolved_in_another_editor_resumes_resolved_and_start_over_re_merges() {
        let (dir, stages) = conflicted(Some(BASE));
        let d = dir.path();
        write(d, "f.txt", "ONE\nTHEIRS\nthree\nfour\n");
        let mut s = opened_state(d, &stages);
        assert_eq!((s.m.unresolved(), s.m.edited_outside()), (0, 1));
        assert_eq!(s.text(), "ONE\nTHEIRS\nthree\nfour\n");
        assert!(
            s.notices()
                .iter()
                .any(|n| n.contains("edited outside Lenz"))
        );
        assert!(s.changed_by(|m| m.start_over()));
        assert_eq!((s.m.unresolved(), s.m.edited_outside()), (1, 0));
        assert!(s.notices().is_empty());
    }

    #[test]
    fn a_file_changed_on_disk_asks_and_overwrite_or_reload_settle_it() {
        let (dir, stages) = conflicted(Some(BASE));
        let d = dir.path();
        let mut s = opened_state(d, &stages);
        s.control(Control::Accept(0, Pick::Theirs));
        assert!(!s.check_disk());

        write(d, "f.txt", "someone else's\n");
        assert!(s.check_disk());
        assert_eq!(s.ask, Some(Ask::ChangedOnDisk));
        // saving refuses, and the Result stays
        let e = s.save().unwrap_err();
        s.refused(e, false);
        assert_eq!(s.ask, Some(Ask::Overwrite { done: false }));
        assert!(s.dirty);
        s.overwrite().unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("f.txt")).unwrap(),
            "one\nTHEIRS\nthree\nfour\n"
        );
        assert!(!s.check_disk());

        // Reload takes the file as it now is, resumed against git's output
        write(d, "f.txt", "ONE\nmine\nthree\nfour\n");
        s.reload().unwrap();
        assert_eq!(s.text(), "ONE\nmine\nthree\nfour\n");
        assert_eq!((s.m.edited_outside(), s.dirty, s.ask), (1, false, None));
        s.save().unwrap();
    }

    #[test]
    fn a_failed_write_shows_the_error_and_keeps_the_result() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, stages) = conflicted(Some(BASE));
        let d = dir.path();
        let mut s = opened_state(d, &stages);
        s.control(Control::Accept(0, Pick::Ours));
        let perms = |p: &Path, mode: u32| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap()
        };
        perms(&d.join("f.txt"), 0o444);
        perms(d, 0o555);
        let saved = s.save();
        perms(d, 0o755);
        perms(&d.join("f.txt"), 0o644);
        if nix_root() {
            return; // root writes anyway
        }
        s.refused(saved.unwrap_err(), false);
        assert!(
            s.error
                .as_deref()
                .is_some_and(|e| e.starts_with("Not saved (the Result is kept")
                    && e.contains("ermission denied"))
        );
        assert_eq!((s.ask, s.dirty, s.m.unresolved()), (None, true, 0));
        s.save().unwrap();
        assert_eq!(s.error, None);
    }

    fn nix_root() -> bool {
        std::fs::metadata("/proc/self").is_ok_and(|m| std::os::unix::fs::MetadataExt::uid(&m) == 0)
    }

    #[test]
    fn versions_with_other_line_endings_merge_on_their_text_and_the_result_keeps_the_file_s() {
        let (dir, stages) = conflicted_in(
            repo(),
            Some(b"a\nb\nc\nd\n"),
            b"a\r\nB\r\nc\r\nd\r\n",
            b"a\nb\nc\nD\n",
        );
        let d = dir.path();
        // git sees every line changed on our side; Lenz merges the text
        assert!(
            std::fs::read_to_string(d.join("f.txt"))
                .unwrap()
                .contains("<<<<<<<")
        );
        let mut s = opened_state(d, &stages);
        assert_eq!((s.m.unresolved(), s.m.edited_outside()), (0, 0));
        assert!(!s.snapshot.crlf);
        assert_eq!(s.notices(), ["Ours: CRLF · Result keeps LF"]);
        s.save().unwrap();
        assert_eq!(
            std::fs::read_to_string(d.join("f.txt")).unwrap(),
            "a\nB\nc\nD\n"
        );
    }

    #[test]
    fn a_utf16_version_merges_with_utf8_ones_and_the_result_keeps_the_file_s_encoding() {
        let utf16 = |s: &str| {
            [0xFF, 0xFE]
                .into_iter()
                .chain(s.encode_utf16().flat_map(u16::to_le_bytes))
                .collect::<Vec<u8>>()
        };
        let ours = utf16("one\nTWO\nthree\nfour\n");
        let (dir, stages) = conflicted_in(
            repo(),
            Some(BASE.as_bytes()),
            &ours,
            b"one\ntwo\nthree\nFOUR\n",
        );
        let d = dir.path();
        // git calls UTF-16 binary and leaves our version in place
        assert_eq!(std::fs::read(d.join("f.txt")).unwrap(), ours);
        let mut s = opened_state(d, &stages);
        assert_eq!(s.m.unresolved(), 0);
        assert_eq!(s.notices(), ["Theirs: UTF-8 · Result keeps UTF-16LE BOM"]);
        s.save().unwrap();
        assert_eq!(
            std::fs::read(d.join("f.txt")).unwrap(),
            utf16("one\nTWO\nthree\nFOUR\n")
        );
    }

    #[test]
    fn added_on_both_sides_says_so_and_lossy_versions_are_refused() {
        let (dir, [b, o, t]) = conflicted(None);
        let d = dir.path();
        let s = opened_state(d, &[b.clone(), o.clone(), t]);
        assert!(!s.m.has_base());
        assert_eq!(s.notices(), ["Added on both sides — no common base"]);
        // invalid UTF-8 that no legacy encoding takes cleanly either
        std::fs::write(&o, b"\xEF\xBB\xBFok\n\xFF\xFE\xFD\n").unwrap();
        let e = open_files(&b, &o, &o, &d.join("f.txt"), d).err().unwrap();
        assert!(e.contains("aren't valid UTF-8"), "{e}");
    }
}

impl Act {
    pub const ALL: [Self; 24] = [
        Self::PrevChange,
        Self::NextChange,
        Self::PrevConflict,
        Self::NextConflict,
        Self::ApplyNonConflicting,
        Self::ResolveSimple,
        Self::AllFrom(Pick::Ours),
        Self::AllFrom(Pick::Theirs),
        Self::Undo,
        Self::Redo,
        Self::StartOver,
        Self::Reload,
        Self::Save,
        Self::Done,
        Self::Cancel,
        Self::ToggleBase,
        Self::Whitespace,
        Self::Scroll,
        Self::MarkResolved,
        Self::Find,
        Self::NextMatch,
        Self::PreviousMatch,
        Self::NextPane,
        Self::ClearFind,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::PrevChange => "Previous change",
            Self::NextChange => "Next change",
            Self::PrevConflict => "Previous unresolved conflict",
            Self::NextConflict => "Next unresolved conflict",
            Self::ApplyNonConflicting => "Apply non-conflicting changes",
            Self::ResolveSimple => "Resolve simple conflicts",
            Self::AllFrom(Pick::Ours) => "Take all Ours",
            Self::AllFrom(Pick::Theirs) => "Take all Theirs",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::StartOver => "Start over",
            Self::Reload => "Reload from disk",
            Self::Save => "Save",
            Self::Done => "Done",
            Self::Cancel => "Cancel",
            Self::ToggleBase => "Toggle Base / Result",
            Self::Whitespace => "Cycle whitespace",
            Self::Scroll => "Toggle shared / padded scroll",
            Self::MarkResolved => "Mark resolved (stage saved file)",
            Self::Find => "Find in focused pane",
            Self::NextMatch => "Next find match",
            Self::PreviousMatch => "Previous find match",
            Self::NextPane => "Focus next pane",
            Self::ClearFind => "Clear find",
        }
    }
}
impl MergeSession {
    pub fn disabled(&self, action: Act) -> Option<&'static str> {
        match action {
            Act::ToggleBase if !self.m.has_base() => Some("No common base"),
            Act::MarkResolved if self.mergetool => Some("Only available in the Conflict List"),
            Act::MarkResolved if self.dirty => Some("Save changes first"),
            Act::NextMatch | Act::PreviousMatch if self.find_count() == 0 => {
                Some("No find matches")
            }
            Act::ClearFind if self.find.is_none() => Some("Find is closed"),
            _ => None,
        }
    }
    pub fn question(&self) -> Option<&'static str> {
        self.ask.map(|a| match a {
            Ask::DoneWithConflicts => "conflicts remain — Done again saves markers; Esc goes back",
            Ask::Close if self.dirty => {
                "Unsaved Result — Save keeps it; Esc again closes without saving"
            }
            Ask::Close => {
                "git will restore its original — Done keeps saved work; Esc again discards"
            }
            Ask::ChangedOnDisk => "File changed on disk — r reloads; Esc keeps your Result",
            Ask::Overwrite { .. } => {
                "File changed on disk — repeat Save/Done to overwrite; r reloads; Esc cancels"
            }
        })
    }
}

impl Control {
    pub fn label(self) -> &'static str {
        match self {
            Self::Accept(_, Pick::Ours) => "Take Ours",
            Self::Accept(_, Pick::Theirs) => "Take Theirs",
            Self::Both(_) => "Take both, Ours first",
            Self::KeepBase(_) => "Keep the Base",
            Self::Reopen(_) => "Reopen the Conflict",
        }
    }
}
impl MergeSession {
    pub fn focus_block(&mut self, i: usize) {
        if i >= self.m.blocks().len() { return; }
        self.current = Some(i);
        self.m.seen(i);
    }
    /// The change block on visible `row` of `pane` (0 ours, 1 result or base, 2 theirs), as [`layout`](Self::layout)
    /// draws it; `None` on an unchanged line, a filler or past the text.
    pub fn block_on_row(&self, pane: usize, row: u16) -> Option<usize> {
        let (rows, _) = self.layout();
        let line = rows.get(pane)?.iter().find(|r| r.y >= 0.0 && r.y as u16 == row).and_then(|r| match r.content { Content::Line(n) => Some(n), _ => None })?;
        owner(self.m.blocks().len(), line, |i| match pane {
            0 => self.m.blocks()[i].left.clone(),
            1 if self.base_shown => self.m.blocks()[i].base.clone(),
            1 => self.m.result_range(i),
            _ => self.m.blocks()[i].right.clone(),
        })
    }

    pub fn controls_for(&self, i: usize) -> Vec<Control> {
        let Some(b) = self.m.blocks().get(i) else {
            return Vec::new();
        };
        let mut controls = Vec::new();
        if b.left_changed {
            controls.push(Control::Accept(i, Pick::Ours));
        }
        if b.right_changed {
            controls.push(Control::Accept(i, Pick::Theirs));
        }
        if self.m.is_conflict(i) {
            controls.push(Control::Both(i));
        }
        controls.push(Control::KeepBase(i));
        if self.m.is_conflict(i) && self.m.state(i) == BlockState::Resolved {
            controls.push(Control::Reopen(i));
        }
        controls
    }
}

/// Syntax jobs and stale-result validation shared by both frontend schedulers.
#[derive(Clone)]
pub struct ParseTag {
    slot: usize,
    version: Option<u64>,
    source: Weak<()>,
}
pub struct ParseJob {
    tag: ParseTag,
    lang: highlight::Language,
    text: String,
    current: Weak<()>,
}
impl ParseJob {
    pub fn run(self) -> (ParseTag, Option<Arc<highlight::Parsed>>) {
        let current = self.current;
        let parsed = highlight::parse_while(self.lang, self.text.into(), move || {
            current.strong_count() > 0
        })
        .map(Arc::new);
        (self.tag, parsed)
    }
}
impl MergeSession {
    pub fn parse_jobs(&mut self) -> Vec<ParseJob> {
        let Some(lang) = self.lang else {
            return Vec::new();
        };
        let mut jobs = Vec::new();
        for (slot, pick) in [(0, Some(Pick::Ours)), (1, None), (2, Some(Pick::Theirs))] {
            if (slot != 1 || self.base_shown) && !self.parsing[slot] && self.parsed[slot].is_none()
            {
                self.parsing[slot] = true;
                let source = Arc::downgrade(&self.parse_id);
                jobs.push(ParseJob {
                    tag: ParseTag {
                        slot,
                        version: None,
                        source: source.clone(),
                    },
                    lang,
                    text: self.m.side(pick).join("\n"),
                    current: source,
                });
            }
        }
        if !self.base_shown && self.parsed_version != Some(self.version) {
            self.result_parse_id = Arc::new(());
            self.parsed_version = Some(self.version);
            self.parsed[3] = None;
            jobs.push(ParseJob {
                tag: ParseTag {
                    slot: 3,
                    version: Some(self.version),
                    source: Arc::downgrade(&self.parse_id),
                },
                lang,
                text: self.m.lines().join("\n"),
                current: Arc::downgrade(&self.result_parse_id),
            });
        }
        jobs
    }
    pub fn finish_parse(&mut self, job: &ParseTag, parsed: Option<Arc<highlight::Parsed>>) -> bool {
        if job.source.ptr_eq(&Arc::downgrade(&self.parse_id))
            && (job.version.is_none() || job.version == Some(self.version))
        {
            self.parsed[job.slot] = parsed;
            true
        } else {
            false
        }
    }
    pub fn syntax_busy(&self) -> bool {
        !self.syntax_work.is_empty()
    }
    pub fn poll_syntax(&mut self) -> bool {
        for job in self.parse_jobs() {
            let tag = job.tag.clone();
            let current = job.current.clone();
            let rx = crate::work::detached(current, move || job.run().1);
            self.syntax_work.push((tag, rx));
        }
        let mut finished = Vec::new();
        let mut i = 0;
        while i < self.syntax_work.len() {
            match self.syntax_work[i].1.try_recv() {
                Ok(parsed) => {
                    let (job, _) = self.syntax_work.swap_remove(i);
                    finished.push((job, parsed.flatten()));
                }
                Err(mpsc::TryRecvError::Empty) => i += 1,
                Err(_) => {
                    self.syntax_work.swap_remove(i);
                }
            }
        }
        let mut changed = false;
        for (job, parsed) in finished {
            changed |= self.finish_parse(&job, parsed);
        }
        changed
    }
}

struct MergeFind {
    search: crate::find::Find,
    editing: bool,
}
impl MergeSession {
    pub fn find_pane(&self) -> usize {
        self.focus_pane
    }
    pub fn find_editing(&self) -> bool {
        self.find.as_ref().is_some_and(|f| f.editing)
    }
    pub fn find_options(&self) -> lenz_content::find::FindOptions {
        self.find_options
    }
    /// Why merge find's query cannot be matched, if it cannot.
    pub fn find_error(&self) -> Option<&str> {
        self.find.as_ref().and_then(|f| f.search.error())
    }
    /// Flip one find option and re-run merge find; returns the options for the window to keep.
    pub fn toggle_find_option(&mut self, option: lenz_content::find::FindOption) -> lenz_content::find::FindOptions {
        self.find_options = self.find_options.toggled(option);
        self.refind();
        self.find_options
    }
    pub fn find_count(&self) -> usize {
        self.find.as_ref().map_or(0, |f| f.search.hits.len())
    }
    pub fn begin_find(&mut self, options: lenz_content::find::FindOptions) {
        self.find_options = options;
        if let Some(f) = &mut self.find {
            f.editing = true;
        } else {
            self.find = Some(MergeFind {
                search: crate::find::Find::new(
                    String::new(),
                    options,
                    Side::Right,
                    &[],
                    crate::selection::Pos::new(0, 0),
                ),
                editing: true,
            });
        }
        self.refind();
    }
    pub fn type_find(&mut self, text: &str) {
        if let Some(f) = &mut self.find {
            for c in crate::document::one_line(text).chars() {
                if f.search.query.len() + c.len_utf8() > 4096 {
                    break;
                }
                f.search.query.push(c);
            }
            self.refind();
        }
    }
    pub fn backspace_find(&mut self) {
        if let Some(f) = &mut self.find {
            f.search.query.pop();
            self.refind();
        }
    }
    pub fn clear_find_query(&mut self) {
        if let Some(f) = &mut self.find {
            f.search.query.clear();
            self.refind();
        }
    }
    pub fn finish_find(&mut self) {
        if let Some(f) = &mut self.find {
            f.editing = false;
        }
    }
    pub fn dismiss_find(&mut self) -> bool {
        self.find.take().is_some()
    }
    pub fn cycle_find_pane(&mut self) {
        self.focus_pane = (self.focus_pane + 1) % 3;
        self.refind();
    }
    pub fn focus_pane(&mut self, pane: usize) {
        self.focus_pane = pane.min(2);
        self.refind();
    }
    pub fn find_hits(&self, pane: usize) -> &[(u32, std::ops::Range<usize>)] {
        self.find
            .as_ref()
            .filter(|_| pane == self.focus_pane)
            .map_or(&[], |f| f.search.hits.as_slice())
    }
    pub fn find_ranges(&self, pane: usize, line: u32) -> Vec<std::ops::Range<usize>> {
        let hits = self.find_hits(pane);
        let start = hits.partition_point(|(l, _)| *l < line);
        hits[start..]
            .iter()
            .take_while(|(l, _)| *l == line)
            .map(|(_, r)| r.clone())
            .collect()
    }
    fn refind(&mut self) {
        let Some(f) = &self.find else { return };
        let query = f.search.query.clone();
        let lines = match self.focus_pane {
            0 => self.m.side(Some(Pick::Ours)),
            1 => self.mid_lines(),
            _ => self.m.side(Some(Pick::Theirs)),
        };
        let from = crate::selection::Pos::new(0, 0);
        let query = lenz_content::find::Query { text: query, options: self.find_options };
        let found = crate::find::Find::search(lines, &query, from);
        if let Some(f) = &mut self.find {
            (f.search.hits, f.search.at, f.search.capped, f.search.error) = match found { Ok((hits, at, capped)) => (hits, at, capped, None), Err(e) => (Vec::new(), None, false, Some(e)) };
        }
        self.show_find_hit();
    }
    pub fn find_step(&mut self, forward: bool) -> bool {
        let Some(f) = &mut self.find else {
            return false;
        };
        let Some((at, _)) = f.search.next_index(forward) else {
            return false;
        };
        f.search.at = Some(at);
        self.show_find_hit();
        true
    }
    fn show_find_hit(&mut self) {
        let Some((line, _)) = self
            .find
            .as_ref()
            .and_then(|f| f.search.at.and_then(|at| f.search.hits.get(at)))
        else {
            return;
        };
        let line = *line;
        let n = self.m.blocks().len();
        let pane = self.focus_pane;
        let i = owner(n, line, |i| match pane {
            0 => self.m.blocks()[i].left.clone(),
            1 => self.mid_range(i),
            _ => self.m.blocks()[i].right.clone(),
        });
        if let Some(i) = i {
            self.focus_block(i);
        }
        if self.scroll == ScrollMode::Padded {
            let rows = self.views[0].viewport_rows();
            let at = padded_row(&self.segments(), pane, line);
            self.padded_top = (at as f32 - rows / 3.0).max(0.0);
        } else {
            match pane {
                0 => {
                    self.views[0].place_line_at(Side::Left, line as f32);
                    self.sync();
                }
                1 => self.place_mid_at(line as f32),
                _ => {
                    self.views[1].place_line_at(Side::Right, line as f32);
                    let mid = self.views[1].anchor_line_on(Side::Left);
                    self.place_mid_at(mid);
                }
            }
        }
    }
    pub fn find_notice(&self) -> Option<String> {
        let f = self.find.as_ref()?;
        let pane = match self.focus_pane {
            0 => "Ours",
            1 if self.base_shown => "Base",
            1 => "Result",
            _ => "Theirs",
        };
        let o = self.find_options;
        let marks: Vec<&str> = [(o.case, "Aa"), (o.word, "W"), (o.regex, ".*")].into_iter().filter(|(on, _)| *on).map(|(_, m)| m).collect();
        let marks = if marks.is_empty() { String::new() } else { format!(" [{}]", marks.join(" ")) };
        let count = f.search.counter();
        Some(format!(
            "Find {pane}{marks}: {}{} · {count} · Tab pane · F3 next · Alt+C/W/R options · Esc clear",
            f.search.query,
            if f.editing { "▎" } else { "" },
        ))
    }
}
