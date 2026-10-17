//! gpui-free Merge View model: the **Result** of a 3-way merge as block-level choices, its text, undo, and the
//! git-style rendering of Conflicts left unresolved.
//!
//! The Result is the Base with every non-conflicting change applied; each **Conflict** shows the Base until the
//! user takes Ours, Theirs, both (in the order taken) or keeps the Base. Between merge blocks the three versions
//! are exactly equal, so those runs are copied from Ours.

use std::ops::Range;

use lenz_engine::{MergeBlock, MergeKind};

/// One of the two versions being merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pick {
    Ours,
    Theirs,
}

/// What the user (or the open, or the magic wand) decided for one merge block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Choice {
    /// Versions taken, in the order taken; empty = the Base.
    pub taken: Vec<Pick>,
    /// Text that is neither version: the magic wand's merge, or the file's own text when resuming.
    pub custom: Option<Vec<String>>,
    pub resolved: bool,
    /// Resolved by the magic wand and not looked at since.
    pub auto: bool,
    /// Resolved from the working-tree file when resuming ("edited outside Lenz").
    pub outside: bool,
}

/// Where a merge block stands, for tints and gutter controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockState {
    /// A Conflict still showing the Base.
    Unresolved,
    /// Applied at open or by the user, or a Conflict resolved.
    Resolved,
}

/// Conflict markers as `merge.conflictStyle` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkerStyle {
    #[default]
    Merge,
    Diff3,
    Zdiff3,
}

/// The names written after conflict markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Labels {
    pub ours: String,
    pub base: String,
    pub theirs: String,
}

#[derive(Debug, Clone)]
pub struct MergeResult {
    ours: Vec<String>,
    base: Vec<String>,
    theirs: Vec<String>,
    /// No common base (added on both sides): "keep the base" means neither version.
    no_base: bool,
    blocks: Vec<MergeBlock>,
    choices: Vec<Choice>,
    /// The blocks as merged, before resuming reshaped them: what **Start over** returns to.
    merged: Vec<MergeBlock>,
    /// The Result's lines and each block's range in them, rebuilt after every change.
    lines: Vec<String>,
    ranges: Vec<Range<u32>>,
    undo: Vec<Step>,
    redo: Vec<Step>,
}

/// An undo snapshot: the blocks (resuming and Start over reshape them) and their choices.
type Step = (Vec<MergeBlock>, Vec<Choice>);

fn owned(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

fn slice(lines: &[String], r: &Range<u32>) -> Vec<String> {
    lines[r.start as usize..r.end as usize].to_vec()
}

impl MergeResult {
    /// The Result of `blocks` (from `merge_lines`) with every non-conflicting change applied.
    pub fn new(ours: &[&str], base: &[&str], theirs: &[&str], blocks: Vec<MergeBlock>) -> Self {
        let choices = blocks.iter().map(Self::opening_choice).collect();
        let mut m = MergeResult {
            ours: owned(ours),
            base: owned(base),
            theirs: owned(theirs),
            no_base: false,
            merged: blocks.clone(),
            blocks,
            choices,
            lines: Vec::new(),
            ranges: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        };
        m.rebuild();
        m
    }

    /// A merge of two versions without a common base, as git does it: equal runs settled, each differing run a
    /// Conflict. `blocks` are the 2-way Change Blocks between Ours (left) and Theirs (right).
    pub fn without_base(ours: &[&str], theirs: &[&str], blocks: &[lenz_engine::ChangeBlock]) -> Self {
        // the Base stands in as Ours so the runs between blocks line up; its text is never used
        let blocks = blocks
            .iter()
            .map(|b| MergeBlock::new(b.left.clone(), b.left.clone(), b.right.clone(), MergeKind::Conflict, true, true))
            .collect();
        let mut m = Self::new(ours, ours, theirs, blocks);
        m.no_base = true;
        m.rebuild();
        m
    }

    fn opening_choice(b: &MergeBlock) -> Choice {
        if b.kind == MergeKind::Conflict {
            return Choice::default();
        }
        let side = if b.left_changed { Pick::Ours } else { Pick::Theirs };
        Choice { taken: vec![side], resolved: true, ..Choice::default() }
    }

    pub fn has_base(&self) -> bool {
        !self.no_base
    }

    pub fn blocks(&self) -> &[MergeBlock] {
        &self.blocks
    }

    pub fn choice(&self, block: usize) -> &Choice {
        &self.choices[block]
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn side(&self, pick: Option<Pick>) -> &[String] {
        match pick {
            Some(Pick::Ours) => &self.ours,
            Some(Pick::Theirs) => &self.theirs,
            None => &self.base,
        }
    }

    /// Block `block`'s lines in the Result.
    pub fn result_range(&self, block: usize) -> Range<u32> {
        self.ranges[block].clone()
    }

    pub fn is_conflict(&self, block: usize) -> bool {
        self.blocks[block].kind == MergeKind::Conflict
    }

    pub fn state(&self, block: usize) -> BlockState {
        if self.choices[block].resolved { BlockState::Resolved } else { BlockState::Unresolved }
    }

    pub fn unresolved(&self) -> usize {
        self.choices.iter().filter(|c| !c.resolved).count()
    }

    fn side_lines(&self, block: usize, pick: Pick) -> &[String] {
        let b = &self.blocks[block];
        match pick {
            Pick::Ours => &self.ours[b.left.start as usize..b.left.end as usize],
            Pick::Theirs => &self.theirs[b.right.start as usize..b.right.end as usize],
        }
    }

    fn base_lines(&self, block: usize) -> &[String] {
        if self.no_base {
            return &[];
        }
        let b = &self.blocks[block].base;
        &self.base[b.start as usize..b.end as usize]
    }

    /// The text block `block` contributes to the Result.
    fn block_text(&self, block: usize) -> Vec<String> {
        let c = &self.choices[block];
        if let Some(custom) = &c.custom {
            return custom.clone();
        }
        if !c.resolved || c.taken.is_empty() {
            return self.base_lines(block).to_vec();
        }
        c.taken.iter().flat_map(|&p| self.side_lines(block, p).iter().cloned()).collect()
    }

    fn rebuild(&mut self) {
        let mut lines = Vec::with_capacity(self.base.len());
        let mut ranges = Vec::with_capacity(self.blocks.len());
        let mut at = 0u32;
        for i in 0..self.blocks.len() {
            let start = self.blocks[i].left.start;
            lines.extend(slice(&self.ours, &(at..start)));
            let r0 = lines.len() as u32;
            lines.extend(self.block_text(i));
            ranges.push(r0..lines.len() as u32);
            at = self.blocks[i].left.end;
        }
        lines.extend(slice(&self.ours, &(at..self.ours.len() as u32)));
        self.lines = lines;
        self.ranges = ranges;
    }

    /// Record the current choices for undo, then apply `f`; returns whether anything changed.
    fn change(&mut self, f: impl FnOnce(&mut Vec<Choice>)) -> bool {
        let before = self.choices.clone();
        f(&mut self.choices);
        if self.choices == before {
            return false;
        }
        self.undo.push((self.blocks.clone(), before));
        self.redo.clear();
        self.rebuild();
        true
    }

    /// ≫ / ≪ : the first version taken replaces the block, a second one is appended after it.
    pub fn accept(&mut self, block: usize, pick: Pick) -> bool {
        self.change(|cs| {
            let c = &mut cs[block];
            if c.resolved && c.custom.is_none() && c.taken.len() == 1 && c.taken[0] != pick {
                c.taken.push(pick);
            } else if !(c.resolved && c.custom.is_none() && c.taken.contains(&pick)) {
                *c = Choice { taken: vec![pick], resolved: true, ..Choice::default() };
            }
        })
    }

    /// Ours then Theirs.
    pub fn accept_both(&mut self, block: usize) -> bool {
        self.change(|cs| cs[block] = Choice { taken: vec![Pick::Ours, Pick::Theirs], resolved: true, ..Choice::default() })
    }

    /// ✕ : the block keeps the Base (for a change applied at open: drops it).
    pub fn keep_base(&mut self, block: usize) -> bool {
        self.change(|cs| cs[block] = Choice { resolved: true, ..Choice::default() })
    }

    /// ↺ : a Conflict back to unresolved.
    pub fn reopen(&mut self, block: usize) -> bool {
        if !self.is_conflict(block) {
            return false;
        }
        self.change(|cs| cs[block] = Choice::default())
    }

    /// Take a whole version: every block becomes that version's text.
    pub fn take_all(&mut self, pick: Pick) -> bool {
        let changed: Vec<bool> = self
            .blocks
            .iter()
            .map(|b| if pick == Pick::Ours { b.left_changed } else { b.right_changed })
            .collect();
        self.change(|cs| {
            for (c, changed) in cs.iter_mut().zip(changed) {
                let taken = if changed { vec![pick] } else { Vec::new() };
                *c = Choice { taken, resolved: true, ..Choice::default() };
            }
        })
    }

    /// Re-apply each non-conflicting change whose block still shows the Base.
    pub fn apply_non_conflicting(&mut self) -> bool {
        let todo: Vec<usize> = (0..self.blocks.len())
            .filter(|&i| !self.is_conflict(i) && self.block_text(i) == self.base_lines(i))
            .collect();
        let opening: Vec<Choice> = todo.iter().map(|&i| Self::opening_choice(&self.blocks[i])).collect();
        self.change(|cs| {
            for (i, c) in todo.into_iter().zip(opening) {
                cs[i] = c;
            }
        })
    }

    /// The magic wand: every unresolved Conflict `resolve` can merge word by word, marked `auto`.
    /// Returns how many were resolved.
    pub fn resolve_simple(&mut self, resolve: impl Fn(&str, &str, &str) -> Option<String>) -> usize {
        let found: Vec<(usize, Vec<String>)> = (0..self.blocks.len())
            .filter(|&i| self.is_conflict(i) && !self.choices[i].resolved && self.blocks[i].resolvable)
            .filter_map(|i| {
                let text = resolve(
                    &self.side_lines(i, Pick::Ours).join("\n"),
                    &self.base_lines(i).join("\n"),
                    &self.side_lines(i, Pick::Theirs).join("\n"),
                )?;
                Some((i, text.split('\n').map(str::to_string).collect()))
            })
            .collect();
        let n = found.len();
        self.change(|cs| {
            for (i, lines) in found {
                cs[i] = Choice { custom: Some(lines), resolved: true, auto: true, ..Choice::default() };
            }
        });
        n
    }

    /// The user looked at an auto-resolved Conflict: drop its "auto" tint (not an undo step).
    pub fn seen(&mut self, block: usize) {
        if let Some(choice) = self.choices.get_mut(block) { choice.auto = false; }
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        let now = self.restore(prev);
        self.redo.push(now);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        let now = self.restore(next);
        self.undo.push(now);
        true
    }

    /// Put `step` in place, returning the state it replaced.
    fn restore(&mut self, (blocks, choices): Step) -> Step {
        let now = (std::mem::replace(&mut self.blocks, blocks), std::mem::replace(&mut self.choices, choices));
        self.rebuild();
        now
    }

    /// The merge as it opened (before any resume), without history.
    pub fn fresh(&self) -> MergeResult {
        let mut m = self.clone();
        m.blocks = m.merged.clone();
        m.choices = m.blocks.iter().map(Self::opening_choice).collect();
        m.undo.clear();
        m.redo.clear();
        m.rebuild();
        m
    }

    /// **Start over**: back to the fresh merge, dropping every choice and what resuming took from the file.
    /// One undo step.
    pub fn start_over(&mut self) -> bool {
        let f = self.fresh();
        if (&f.blocks, &f.choices) == (&self.blocks, &self.choices) {
            return false;
        }
        let now = self.restore((f.blocks, f.choices));
        self.undo.push(now);
        self.redo.clear();
        true
    }

    /// Blocks resolved from the working-tree file when resuming.
    pub fn edited_outside(&self) -> usize {
        self.choices.iter().filter(|c| c.outside).count()
    }

    /// The three texts of a block, which identify it across re-merges.
    fn key(&self, block: usize) -> (Vec<String>, Vec<String>, Vec<String>) {
        (
            self.side_lines(block, Pick::Ours).to_vec(),
            self.base_lines(block).to_vec(),
            self.side_lines(block, Pick::Theirs).to_vec(),
        )
    }

    /// Carry choices over to a re-merge of the same versions at the same source ranges.
    /// Repeated identical conflicts retain separate choices. Undo history starts afresh.
    pub fn carry_choices_to(&self, next: &mut MergeResult) {
        use std::collections::HashMap;
        let ranges = |b: &MergeBlock| (b.left.clone(), b.base.clone(), b.right.clone());
        let mine: HashMap<_, _> = self.blocks.iter().zip(&self.choices).map(|(b, c)| (ranges(b), c)).collect();
        for (b, choice) in next.blocks.iter().zip(&mut next.choices) {
            if let Some(&c) = mine.get(&ranges(b)) {
                *choice = c.clone();
            }
        }
        next.rebuild();
    }

    /// The Result as text (lines joined with `\n`), each unresolved Conflict written as git conflict markers.
    pub fn render(&self, style: MarkerStyle, marker_size: usize, labels: &Labels) -> String {
        self.rendered(style, marker_size, labels).0.join("\n")
    }

    /// [`render`](Self::render)'s lines, and each block's lines in them (markers included).
    fn rendered(&self, style: MarkerStyle, marker_size: usize, labels: &Labels) -> (Vec<String>, Vec<Range<u32>>) {
        let mut out: Vec<String> = Vec::with_capacity(self.lines.len());
        let mut ranges = Vec::with_capacity(self.ranges.len());
        let mut at = 0usize;
        let marker = |c: char, label: &str| {
            let m = c.to_string().repeat(marker_size);
            if label.is_empty() { m } else { format!("{m} {label}") }
        };
        for (i, r) in self.ranges.iter().enumerate() {
            out.extend(self.lines[at..r.start as usize].iter().cloned());
            at = r.end as usize;
            let start = out.len() as u32;
            // resolved, or still conflicted in the file's own words (edited between its markers): as it stands
            if self.choices[i].resolved || self.choices[i].custom.is_some() {
                out.extend(self.lines[r.start as usize..r.end as usize].iter().cloned());
                ranges.push(start..out.len() as u32);
                continue;
            }
            let (mut ours, base, mut theirs) = self.key(i);
            // zdiff3: lines both versions share at the edges move outside the markers
            let (mut head, mut tail) = (Vec::new(), Vec::new());
            if style == MarkerStyle::Zdiff3 {
                let pre = ours.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
                head = ours[..pre].to_vec();
                ours.drain(..pre);
                theirs.drain(..pre);
                let suf = ours.iter().rev().zip(theirs.iter().rev()).take_while(|(a, b)| a == b).count();
                tail = ours[ours.len() - suf..].to_vec();
                ours.truncate(ours.len() - suf);
                theirs.truncate(theirs.len() - suf);
            }
            out.extend(head);
            out.push(marker('<', &labels.ours));
            out.extend(ours);
            if style != MarkerStyle::Merge {
                out.push(marker('|', &labels.base));
                out.extend(base);
            }
            out.push(marker('=', ""));
            out.extend(theirs);
            out.push(marker('>', &labels.theirs));
            out.extend(tail);
            ranges.push(start..out.len() as u32);
        }
        out.extend(self.lines[at..].iter().cloned());
        (out, ranges)
    }

    /// Resume a fresh merge from `file`, the working-tree file, where it isn't git's own marker output for the same
    /// versions (`git_output`). The file is lined up against this Result as written: each stretch the user changed
    /// is resolved with the file's text, marked `outside`, unless it still holds a whole conflict (`<`, `=`, `>`
    /// marker lines in order): then it stays a Conflict in the file's own words, saved as they stand, so an edit
    /// between markers survives (a lone marker-shaped line of text, `=======` under a heading, doesn't count). A
    /// change between blocks becomes a block of its own; one spanning several blocks joins them.
    /// Marker labels never count as changes. Not an undo step. Returns how many blocks come from the file.
    pub fn resume(&mut self, git_output: &str, file: &str, style: MarkerStyle, marker_size: usize, labels: &Labels) -> usize {
        let (written, ranges) = self.rendered(style, marker_size, labels);
        let file = lenz_engine::split_lines(file);
        let f = bare(file.iter().copied(), marker_size);
        let hunks = lenz_engine::diff_lines(&bare(written.iter().map(String::as_str), marker_size), &f);
        let mine = lenz_engine::diff_lines(&bare(lenz_engine::split_lines(git_output), marker_size), &f);
        // where the file isn't git's output: the user's edits, rather than git and Lenz merging differently
        let edited = |r: &Range<u32>| {
            mine.iter().any(|h| {
                if h.right.is_empty() { (r.start..=r.end).contains(&h.right.start) } else { h.right.start < r.end && r.start < h.right.end }
            })
        };

        // each change grows over the blocks it overlaps; changes sharing a block form one group
        struct Group {
            rows: Range<u32>,
            blocks: Range<usize>,
            file: Range<u32>,
        }
        let mut groups: Vec<Group> = Vec::new();
        for h in &hunks {
            let r = &h.left;
            // an insertion at a block's edge stays apart, so it isn't lost with a block still holding markers
            let touches = |b: &Range<u32>| if r.is_empty() { b.start < r.start && r.start < b.end } else { b.start < r.end && r.start < b.end };
            let (rows, blocks) = match ranges.iter().position(touches) {
                Some(i) => {
                    let j = i + ranges[i..].iter().take_while(|b| touches(b)).count();
                    (r.start.min(ranges[i].start)..r.end.max(ranges[j - 1].end), i..j)
                }
                None => {
                    let k = ranges.partition_point(|b| b.end <= r.start);
                    (r.clone(), k..k)
                }
            };
            // outside the changes the file runs parallel to the written Result
            let shift = |row: u32, from: u32, to: u32| (row as i64 + to as i64 - from as i64) as u32;
            let file = shift(rows.start, h.left.start, h.right.start)..shift(rows.end, h.left.end, h.right.end);
            match groups.last_mut() {
                Some(last) if rows.start < last.rows.end => {
                    last.rows.end = last.rows.end.max(rows.end);
                    last.blocks = last.blocks.start.min(blocks.start)..last.blocks.end.max(blocks.end);
                    last.file.end = file.end;
                }
                _ => groups.push(Group { rows, blocks, file }),
            }
        }

        // a row's place in Ours, Base and Theirs, counted from the end of block `k` (equal and aligned after it)
        let at = |k: Option<usize>, row: u32| match k {
            None => [row; 3],
            Some(k) => {
                let (b, d) = (&self.blocks[k], row - ranges[k].end);
                [b.left.end + d, b.base.end + d, b.right.end + d]
            }
        };
        let (mut blocks, mut choices, mut next) = (Vec::new(), Vec::new(), 0);
        for g in groups {
            let text: Vec<String> = file[g.file.start as usize..g.file.end as usize].iter().map(|l| l.to_string()).collect();
            if !edited(&g.file) {
                continue;
            }
            // edited but still between markers: a Conflict yet, in the file's words, so saving keeps the edit
            let conflicted = conflict_markers(&text, marker_size);
            blocks.extend_from_slice(&self.blocks[next..g.blocks.start]);
            choices.extend_from_slice(&self.choices[next..g.blocks.start]);
            next = g.blocks.end;
            if g.blocks.len() == 1 && ranges[g.blocks.start] == g.rows {
                blocks.push(self.blocks[g.blocks.start].clone());
            } else {
                let [o0, b0, t0] = at(g.blocks.start.checked_sub(1), g.rows.start);
                let [o1, b1, t1] = at(g.blocks.end.checked_sub(1), g.rows.end);
                let base = &self.base[b0 as usize..b1 as usize];
                let left_changed = self.no_base || self.ours[o0 as usize..o1 as usize] != *base;
                let right_changed = self.no_base || self.theirs[t0 as usize..t1 as usize] != *base;
                let kind = if g.blocks.clone().any(|i| self.is_conflict(i)) { MergeKind::Conflict } else { MergeKind::Modified };
                blocks.push(MergeBlock::new(o0..o1, b0..b1, t0..t1, kind, left_changed, right_changed));
            }
            choices.push(Choice { custom: Some(text), resolved: !conflicted, outside: true, ..Choice::default() });
        }
        blocks.extend_from_slice(&self.blocks[next..]);
        choices.extend_from_slice(&self.choices[next..]);
        self.blocks = blocks;
        self.choices = choices;
        self.rebuild();
        self.edited_outside()
    }
}

/// The marker character when `line` is a conflict marker of `size` by git's rule: `<`, `=`, `>` or `|` exactly `size`
/// times, then nothing or whitespace.
fn marker_char(line: &str, size: usize) -> Option<u8> {
    let b = line.as_bytes();
    let c = *b.first()?;
    let run = matches!(c, b'<' | b'=' | b'>' | b'|') && b.len() >= size && b[..size].iter().all(|&x| x == c);
    (run && b.get(size).is_none_or(u8::is_ascii_whitespace)).then_some(c)
}

/// The labels on the first conflict's markers in `text` (git's own, e.g. `HEAD` / `parent of 1a2b (msg)` /
/// `feature`), if it has one; a missing base marker leaves `base` empty.
pub fn marker_labels(text: &str, size: usize) -> Option<Labels> {
    let label = |l: &str| l.get(size..).unwrap_or("").trim().to_string();
    let mut found: [Option<String>; 3] = Default::default();
    for l in text.lines() {
        let l = l.strip_suffix('\r').unwrap_or(l);
        match marker_char(l, size) {
            Some(b'<') if found[0].is_none() => found[0] = Some(label(l)),
            Some(b'|') if found[0].is_some() && found[1].is_none() => found[1] = Some(label(l)),
            Some(b'>') if found[0].is_some() => {
                found[2] = Some(label(l));
                break;
            }
            _ => {}
        }
    }
    let [Some(ours), base, Some(theirs)] = found else { return None };
    Some(Labels { ours, base: base.unwrap_or_default(), theirs })
}

/// Whether `lines` hold a whole conflict: a `<` marker line, then `=`, then `>` — not just a marker-shaped line of
/// text (`=======` under a heading).
fn conflict_markers(lines: &[String], size: usize) -> bool {
    let mut want = b"<=>".iter().copied().peekable();
    for l in lines {
        if want.peek().is_some_and(|&c| marker_char(l, size) == Some(c)) {
            want.next();
        }
    }
    want.peek().is_none()
}

/// Lines with each marker line cut to its marker, so labels (git's `HEAD`, Lenz's branch name) don't count.
fn bare<'a>(lines: impl IntoIterator<Item = &'a str>, size: usize) -> Vec<&'a str> {
    lines.into_iter().map(|l| if marker_char(l, size).is_some() { &l[..size] } else { l }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<&str> {
        lenz_engine::split_lines(s)
    }

    fn block(left: Range<u32>, base: Range<u32>, right: Range<u32>, kind: MergeKind, l: bool, r: bool) -> MergeBlock {
        MergeBlock::new(left, base, right, kind, l, r)
    }

    /// ours changes line 1, theirs changes line 3, both change line 5 differently.
    fn sample() -> MergeResult {
        let base = lines("a\nb\nc\nd\ne\n");
        let ours = lines("a\nB\nc\nd\nE1\n");
        let theirs = lines("a\nb\nc\nD\nE2\n");
        let blocks = vec![
            block(1..2, 1..2, 1..2, MergeKind::Modified, true, false),
            block(3..4, 3..4, 3..4, MergeKind::Modified, false, true),
            block(4..5, 4..5, 4..5, MergeKind::Conflict, true, true),
        ];
        MergeResult::new(&ours, &base, &theirs, blocks)
    }

    fn text(m: &MergeResult) -> String {
        m.lines().join("\n")
    }

    #[test]
    fn opens_with_non_conflicting_changes_applied_and_conflicts_showing_the_base() {
        let m = sample();
        assert_eq!(text(&m), "a\nB\nc\nD\ne\n");
        assert_eq!(m.unresolved(), 1);
        assert_eq!(m.state(2), BlockState::Unresolved);
        assert_eq!(m.result_range(2), 4..5);
    }

    #[test]
    fn the_second_version_taken_is_appended_in_click_order() {
        let mut m = sample();
        assert!(m.accept(2, Pick::Theirs));
        assert_eq!(text(&m), "a\nB\nc\nD\nE2\n");
        assert!(m.accept(2, Pick::Ours));
        assert_eq!(text(&m), "a\nB\nc\nD\nE2\nE1\n");
        assert_eq!(m.result_range(2), 4..6);
        assert_eq!(m.unresolved(), 0);
        // taking a version already taken changes nothing
        assert!(!m.accept(2, Pick::Ours));
    }

    #[test]
    fn keep_base_drops_an_applied_change_and_the_arrow_retakes_it() {
        let mut m = sample();
        m.keep_base(0);
        assert_eq!(text(&m), "a\nb\nc\nD\ne\n");
        m.accept(0, Pick::Ours);
        assert_eq!(text(&m), "a\nB\nc\nD\ne\n");
    }

    #[test]
    fn reopen_undo_and_redo() {
        let mut m = sample();
        m.accept_both(2);
        assert_eq!(text(&m), "a\nB\nc\nD\nE1\nE2\n");
        m.reopen(2);
        assert_eq!(m.unresolved(), 1);
        assert!(m.undo());
        assert_eq!(m.unresolved(), 0);
        assert!(m.undo());
        assert_eq!(text(&m), "a\nB\nc\nD\ne\n");
        assert!(!m.undo());
        assert!(m.redo());
        assert_eq!(text(&m), "a\nB\nc\nD\nE1\nE2\n");
    }

    #[test]
    fn take_all_makes_the_result_that_version() {
        let mut m = sample();
        m.take_all(Pick::Theirs);
        assert_eq!(text(&m), "a\nb\nc\nD\nE2\n");
        m.take_all(Pick::Ours);
        assert_eq!(text(&m), "a\nB\nc\nd\nE1\n");
    }

    #[test]
    fn apply_non_conflicting_skips_blocks_already_changed() {
        let mut m = sample();
        m.keep_base(0);
        m.keep_base(1);
        m.accept(1, Pick::Ours); // ours == base here, so this block still shows the Base
        assert!(m.apply_non_conflicting());
        assert_eq!(text(&m), "a\nB\nc\nD\ne\n");
        assert!(!m.apply_non_conflicting());
    }

    #[test]
    fn the_wand_resolves_resolvable_conflicts_as_one_undo_step() {
        let mut m = sample();
        m.blocks[2].resolvable = true;
        let n = m.resolve_simple(|o, b, t| {
            assert_eq!((o, b, t), ("E1", "e", "E2"));
            Some("E12".into())
        });
        assert_eq!(n, 1);
        assert_eq!(text(&m), "a\nB\nc\nD\nE12\n");
        assert!(m.choice(2).auto);
        m.seen(2);
        assert!(!m.choice(2).auto);
        m.undo();
        assert_eq!(m.unresolved(), 1);
    }

    #[test]
    fn choices_carry_over_to_a_re_merge_by_block_text() {
        let mut m = sample();
        m.accept(2, Pick::Theirs);
        m.keep_base(0);
        let mut next = sample();
        // a block whose text differs in the re-merge keeps its opening choice
        next.blocks[1] = block(2..4, 2..4, 2..4, MergeKind::Modified, false, true);
        m.carry_choices_to(&mut next);
        assert_eq!(next.choice(0), m.choice(0));
        assert_eq!(next.choice(2), m.choice(2));
        assert_eq!(next.choice(1).taken, vec![Pick::Theirs]);
        assert!(!next.clone().undo());
    }

    #[test]
    fn a_real_merge_applies_one_sided_changes_and_leaves_the_clash() {
        let base = lines("fn a() {}\n\nfn b() {}\n\nfn c() {}\n");
        let ours = lines("fn a() { 1 }\n\nfn b() {}\n\nfn c() { ours }\n");
        let theirs = lines("fn a() {}\n\nfn b() { 2 }\n\nfn c() { theirs }\n");
        let blocks = lenz_engine::merge_lines(&ours, &base, &theirs, &Default::default());
        let m = MergeResult::new(&ours, &base, &theirs, blocks);
        assert_eq!(m.unresolved(), 1);
        assert_eq!(
            m.render(MarkerStyle::Merge, 7, &labels()),
            "fn a() { 1 }\n\nfn b() { 2 }\n\n<<<<<<< HEAD\nfn c() { ours }\n=======\nfn c() { theirs }\n>>>>>>> feature\n"
        );
    }

    #[test]
    fn the_wand_merges_edits_to_different_words_of_one_line() {
        let base = lines("let x = 1 + 2;\n");
        let ours = lines("let y = 1 + 2;\n");
        let theirs = lines("let x = 1 + 3;\n");
        let clash = lines("let z = 1 + 2;\n");
        let blocks = lenz_engine::merge_lines(&ours, &base, &theirs, &Default::default());
        let mut m = MergeResult::new(&ours, &base, &theirs, blocks);
        assert_eq!(m.resolve_simple(lenz_engine::resolve_simple), 1);
        assert_eq!(text(&m), "let y = 1 + 3;\n");
        // the same word changed on both sides stays a Conflict
        let blocks = lenz_engine::merge_lines(&ours, &base, &clash, &Default::default());
        let mut m = MergeResult::new(&ours, &base, &clash, blocks);
        assert_eq!(m.resolve_simple(lenz_engine::resolve_simple), 0);
        assert_eq!(m.unresolved(), 1);
    }

    fn labels() -> Labels {
        Labels { ours: "HEAD".into(), base: "base".into(), theirs: "feature".into() }
    }

    #[test]
    fn unresolved_conflicts_render_as_markers_in_each_style() {
        let m = sample();
        assert_eq!(
            m.render(MarkerStyle::Merge, 7, &labels()),
            "a\nB\nc\nD\n<<<<<<< HEAD\nE1\n=======\nE2\n>>>>>>> feature\n"
        );
        assert_eq!(
            m.render(MarkerStyle::Diff3, 3, &labels()),
            "a\nB\nc\nD\n<<< HEAD\nE1\n||| base\ne\n===\nE2\n>>> feature\n"
        );
        let mut resolved = sample();
        resolved.accept(2, Pick::Ours);
        assert_eq!(resolved.render(MarkerStyle::Merge, 7, &labels()), "a\nB\nc\nD\nE1\n");
    }

    #[test]
    fn zdiff3_moves_shared_edges_outside_the_markers() {
        let base = lines("x\n");
        let ours = lines("p\no\nq\n");
        let theirs = lines("p\nt\nq\n");
        let m = MergeResult::new(&ours, &base, &theirs, vec![block(0..3, 0..1, 0..3, MergeKind::Conflict, true, true)]);
        assert_eq!(
            m.render(MarkerStyle::Zdiff3, 7, &labels()),
            "p\n<<<<<<< HEAD\no\n||||||| base\nx\n=======\nt\n>>>>>>> feature\nq\n"
        );
    }

    /// Conflicts at `b` and `j`, Theirs alone changing `f`, unchanged runs between.
    fn two_conflicts() -> MergeResult {
        let base = lines("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n");
        let ours = lines("a\nB1\nc\nd\ne\nf\ng\nh\ni\nJ1\nk\n");
        let theirs = lines("a\nB2\nc\nd\ne\nF\ng\nh\ni\nJ2\nk\n");
        let blocks = lenz_engine::merge_lines(&ours, &base, &theirs, &Default::default());
        let m = MergeResult::new(&ours, &base, &theirs, blocks);
        assert_eq!(m.unresolved(), 2);
        m
    }

    /// The labels git itself writes, which differ from Lenz's.
    fn git_labels() -> Labels {
        Labels { ours: "HEAD".into(), base: "1a2b3c4".into(), theirs: "feature".into() }
    }

    #[test]
    fn resuming_from_git_s_own_output_changes_nothing_whatever_the_labels() {
        let mut m = two_conflicts();
        let git = m.render(MarkerStyle::Diff3, 7, &git_labels());
        assert_eq!(m.resume(&git, &git, MarkerStyle::Diff3, 7, &labels()), 0);
        assert_eq!(m.unresolved(), 2);
        assert_eq!(m.blocks().len(), 3);
    }

    #[test]
    fn resuming_a_part_resolved_file_resolves_the_conflicts_without_markers() {
        let mut m = two_conflicts();
        let git = m.render(MarkerStyle::Diff3, 9, &git_labels());
        // the first Conflict resolved to both sides; the second still between markers
        let file = git.replacen("<<<<<<<<< HEAD\nB1\n||||||||| 1a2b3c4\nb\n=========\nB2\n>>>>>>>>> feature\n", "B1\nB2\n", 1);
        assert_ne!(file, git);
        assert_eq!(m.resume(&git, &file, MarkerStyle::Diff3, 9, &labels()), 1);
        assert_eq!(m.unresolved(), 1);
        assert_eq!(m.choice(0).custom, Some(vec!["B1".to_string(), "B2".to_string()]));
        assert!(m.choice(0).outside && m.choice(0).resolved);
        assert!(!m.choice(2).resolved);
        assert_eq!(m.render(MarkerStyle::Diff3, 9, &git_labels()), file);
        assert!(!m.clone().undo(), "resuming isn't an undo step");
    }

    #[test]
    fn resuming_an_externally_edited_file_keeps_every_edit_and_start_over_undoes_in_one_step() {
        let mut m = two_conflicts();
        let git = m.render(MarkerStyle::Merge, 7, &git_labels());
        let file = git
            .replacen("c\n", "c\nnew\n", 1) // an insertion between blocks
            .replacen("F\n", "F!\n", 1) // the applied change from Theirs, edited
            .replacen("h\n", "H\n", 1) // a line no version changed
            .replacen("<<<<<<< HEAD\nJ1\n=======\nJ2\n>>>>>>> feature\n", "J\n", 1);
        assert_eq!(m.resume(&git, &file, MarkerStyle::Merge, 7, &labels()), 4);
        assert_eq!(m.blocks().len(), 5);
        assert_eq!(m.unresolved(), 1);
        assert_eq!(m.render(MarkerStyle::Merge, 7, &git_labels()), file);
        let resumed = m.lines().to_vec();

        assert!(m.start_over());
        assert_eq!((m.blocks().len(), m.unresolved(), m.edited_outside()), (3, 2, 0));
        assert_eq!(m.render(MarkerStyle::Merge, 7, &git_labels()), git);
        assert!(!m.start_over());
        assert!(m.undo());
        assert_eq!(m.lines(), resumed);
        assert!(m.redo());
        assert_eq!(m.blocks().len(), 3);
    }

    #[test]
    fn an_edit_between_markers_stays_a_conflict_and_is_saved_as_it_stands() {
        let mut m = two_conflicts();
        let git = m.render(MarkerStyle::Merge, 7, &git_labels());
        let file = git.replacen("=======\nJ2\n", "=======\nJ2 edited\n", 1);
        assert_eq!(m.resume(&git, &file, MarkerStyle::Merge, 7, &labels()), 1);
        assert_eq!(m.unresolved(), 2);
        assert_eq!(m.render(MarkerStyle::Merge, 7, &git_labels()), file);
        // taking a version resolves it as usual
        let i = (0..m.blocks().len()).rev().find(|&i| m.state(i) == BlockState::Unresolved).unwrap();
        m.accept(i, Pick::Ours);
        assert_eq!(m.unresolved(), 1);
    }

    #[test]
    fn a_marker_shaped_line_of_text_doesn_t_keep_a_resolution_unresolved() {
        let mut m = two_conflicts();
        let git = m.render(MarkerStyle::Merge, 7, &git_labels());
        let file = git.replacen("<<<<<<< HEAD\nB1\n=======\nB2\n>>>>>>> feature\n", "Title\n=======\n", 1);
        assert_eq!(m.resume(&git, &file, MarkerStyle::Merge, 7, &labels()), 1);
        assert_eq!(m.unresolved(), 1);
        assert_eq!(m.render(MarkerStyle::Merge, 7, &git_labels()), file);
    }

    #[test]
    fn git_s_own_marker_labels_are_read_from_the_file() {
        let text = "a\n<<<<<<< HEAD\nc\n||||||| parent of 09c2c21 (feat change)\na\n=======\nb\n>>>>>>> 09c2c21 (feat change)\n";
        let l = marker_labels(text, 7).unwrap();
        assert_eq!((l.ours.as_str(), l.base.as_str(), l.theirs.as_str()), ("HEAD", "parent of 09c2c21 (feat change)", "09c2c21 (feat change)"));
        assert_eq!(marker_labels("<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> feature\r\n", 7).unwrap().theirs, "feature");
        assert!(marker_labels("no conflicts\n=======\n", 7).is_none());
    }

    #[test]
    fn where_git_merged_differently_and_the_user_didn_t_edit_lenz_s_merge_stays() {
        let mut m = two_conflicts();
        let git = m.render(MarkerStyle::Merge, 7, &git_labels()).replacen("d\n", "d-by-git\n", 1);
        let file = git.replacen("<<<<<<< HEAD\nB1\n=======\nB2\n>>>>>>> feature\n", "B\n", 1);
        assert_eq!(m.resume(&git, &file, MarkerStyle::Merge, 7, &labels()), 1);
        assert_eq!(m.lines()[..4], ["a", "B", "c", "d"]);
    }

    #[test]
    fn without_a_base_equal_runs_settle_and_keeping_the_base_drops_both() {
        let ours = lines("same\nmine\nend\n");
        let theirs = lines("same\nyours\nend\n");
        let b = lenz_engine::ChangeBlock { left: 1..2, right: 1..2, ignored: false, inline: None };
        let mut m = MergeResult::without_base(&ours, &theirs, &[b]);
        assert!(!m.has_base());
        assert_eq!(text(&m), "same\nend\n");
        assert_eq!(m.unresolved(), 1);
        m.keep_base(0);
        assert_eq!(text(&m), "same\nend\n");
        m.accept_both(0);
        assert_eq!(text(&m), "same\nmine\nyours\nend\n");
    }
}
