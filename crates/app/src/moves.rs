//! Moving changes in 2-way diffs (gpui-free): which Sides may be written, the text a move (≫ / ≪) makes,
//! and per-file undo. The UI writes the result through [`crate::write`].

use std::{fs, hash::{DefaultHasher, Hasher}, ops::Range, path::Path};

use crate::{changeset::Content, view::Side};

/// Why a Side can't take a move, or `Ok` when it is a real file Lenz may write: a file on disk outside
/// `temp` (the system temp directory, where git hands over revisions) once symlinks are resolved, so
/// `git difftool -d`'s links into the working tree count. A symlink anywhere else (a working-tree link,
/// mode 120000) stays read-only, as does a temp link to a link, so a move never writes through a link
/// to somewhere outside the repository.
pub fn editable(content: &Content, temp: &Path) -> Result<(), &'static str> {
    let path = match content {
        Content::File(path) => path,
        Content::Blob(_) => return Err("a git revision"),
        Content::Empty => return Err("absent"),
        Content::Symlink(_) => return Err("a symlink"),
        Content::Submodule(_) => return Err("a submodule"),
        Content::Archived { .. } => return Err("inside an archive"),
    };
    let temp = fs::canonicalize(temp).unwrap_or_else(|_| temp.to_path_buf());
    let in_temp = |p: &Path| fs::canonicalize(p).is_ok_and(|p| p.starts_with(&temp));
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink()) {
        let dir = path.parent().unwrap_or(Path::new("."));
        let target = fs::read_link(path).map(|t| dir.join(t));
        let to_link = target.is_ok_and(|t| fs::symlink_metadata(t).is_ok_and(|m| m.is_symlink()));
        if !in_temp(dir) || to_link {
            return Err("a symlink");
        }
    }
    match fs::canonicalize(path) {
        Err(_) => Err("missing on disk"),
        Ok(_) if in_temp(path) => Err("a temporary copy"),
        Ok(_) => Ok(()),
    }
}

/// `target` with its lines `into` replaced by `source`'s lines `from` — one Change Block's two ranges.
/// Lines are the text split at `\n`, as the diff counts them (a final newline makes an empty last line),
/// so blocks at the file end carry the final newline with them. Copied lines lose a `\r` before their
/// `\n`: the writer gives them the target's own line ending.
pub fn moved_text(target: &str, source: &str, into: Range<u32>, from: Range<u32>) -> String {
    let (t, s): (Vec<&str>, Vec<&str>) = (target.split('\n').collect(), source.split('\n').collect());
    let last = s.len() - 1;
    let copied = (from.start as usize..from.end as usize).map(|i| if i < last { s[i].strip_suffix('\r').unwrap_or(s[i]) } else { s[i] });
    let lines: Vec<&str> = t[..into.start as usize].iter().copied().chain(copied).chain(t[into.end as usize..].iter().copied()).collect();
    lines.join("\n")
}

/// One Comparison's moves for the session. Each inverse patch keeps only the replaced text.
#[derive(Debug, Default, Clone)]
pub struct Edits {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

#[derive(Debug, Clone)]
struct Edit {
    side: Side,
    replace: Range<usize>,
    text: String,
    /// A patch's offsets belong to this exact version, even if another Comparison edited the file.
    expected: u64,
}

pub(crate) fn fingerprint(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    h.write(text.as_bytes());
    h.finish()
}

impl Edit {
    /// The patch that turns `after` back into `before`, on UTF-8 boundaries.
    fn between(side: Side, before: &str, after: &str) -> Self {
        let mut start = before.bytes().zip(after.bytes()).take_while(|(a, b)| a == b).count();
        while !before.is_char_boundary(start) || !after.is_char_boundary(start) { start -= 1; }
        let mut suffix = before[start..].bytes().rev().zip(after[start..].bytes().rev()).take_while(|(a, b)| a == b).count();
        while !before.is_char_boundary(before.len() - suffix) || !after.is_char_boundary(after.len() - suffix) { suffix -= 1; }
        Edit {
            side, replace: start..after.len() - suffix,
            text: before[start..before.len() - suffix].to_string(), expected: fingerprint(after),
        }
    }
}

impl Edits {
    pub(crate) fn record_detail(&mut self, side: Side, before: &str, after: &str) -> (Range<usize>, String) {
        let edit = Edit::between(side, before, after);
        let detail = (edit.replace.clone(), edit.text.clone());
        self.undo.push(edit); self.redo.clear(); detail
    }

    /// A move changed `side`, which held `before`; nothing is left to redo.
    pub fn record(&mut self, side: Side, before: &str, after: &str) {
        self.undo.push(Edit::between(side, before, after));
        self.redo.clear();
    }

    /// What undo (or redo) would write next: the Side and its text.
    pub fn next(&self, redo: bool, sides: [&str; 2]) -> Option<(Side, String)> {
        let e = if redo { self.redo.last()? } else { self.undo.last()? };
        let current = sides[e.side as usize];
        if fingerprint(current) != e.expected { return None; }
        let mut text = String::with_capacity(current.len() - e.replace.len() + e.text.len());
        text.push_str(current.get(..e.replace.start)?);
        text.push_str(&e.text);
        text.push_str(current.get(e.replace.end..)?);
        Some((e.side, text))
    }

    /// That step was written; `replaced` is the text it overwrote, for the opposite direction.
    pub fn stepped(&mut self, redo: bool, replaced: &str, written: &str) {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        if let Some(e) = from.pop() {
            // The writer may normalize line endings: reverse what it actually wrote.
            to.push(Edit::between(e.side, replaced, written));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::{Snapshot, write_text};
    use lenz_engine::{Options, compare_lines, split_lines};
    use std::os::unix::fs::symlink;

    /// ≫ for every Change Block between `left` and `right`, one at a time: each makes that block vanish.
    fn each_move(left: &str, right: &str) -> Vec<String> {
        let blocks = compare_lines(&split_lines(left), &split_lines(right), &Options::default());
        blocks.iter().map(|b| moved_text(right, left, b.right.clone(), b.left.clone())).collect()
    }

    #[test]
    fn moving_a_block_copies_its_lines_over_the_other_side() {
        // modified, inserted (on the right) and deleted (from the right) blocks
        let left = "a\nb\nc\nd\ne\n";
        let right = "a\nB\nc\nnew\nd\n";
        let blocks = compare_lines(&split_lines(left), &split_lines(right), &Options::default());
        assert_eq!(blocks.len(), 3);
        let right_after: Vec<String> = blocks.iter().map(|b| moved_text(right, left, b.right.clone(), b.left.clone())).collect();
        assert_eq!(right_after, ["a\nb\nc\nnew\nd\n", "a\nB\nc\nd\n", "a\nB\nc\nnew\nd\ne\n"]);
        // ≪ the other way round
        let left_after: Vec<String> = blocks.iter().map(|b| moved_text(left, right, b.left.clone(), b.right.clone())).collect();
        assert_eq!(left_after, ["a\nB\nc\nd\ne\n", "a\nb\nc\nnew\nd\ne\n", "a\nb\nc\nd\n"]);
    }

    #[test]
    fn blocks_at_the_file_end_carry_the_final_newline() {
        // a missing final newline added or removed, and lines appended after a last line without one
        assert_eq!(each_move("a\nb", "a\nb\n"), ["a\nb"]);
        assert_eq!(each_move("a\nb\n", "a\nb"), ["a\nb\n"]);
        assert_eq!(each_move("a\nb\nc\n", "a"), ["a\nb\nc\n"]);
        assert_eq!(each_move("a", "a\nb\nc\n"), ["a"]);
        assert_eq!(each_move("x\n", ""), ["x\n"]);
        assert_eq!(each_move("", "x\n"), [""]);
    }

    #[test]
    fn copied_lines_take_the_target_line_ending_and_exact_whitespace() {
        // CRLF source, LF target: the copied lines arrive with `\n`
        assert_eq!(moved_text("a\nX\nc\n", "a\r\n  b \r\nc\r\n", 1..2, 1..2), "a\n  b \nc\n");
        // the target's own lines keep theirs
        assert_eq!(moved_text("a\r\nX\r\nc\r\n", "a\nb\nc\n", 1..2, 1..2), "a\r\nb\nc\r\n");
        // under Ignore the block is found ignoring whitespace, but its exact text is copied
        let ignore = Options { whitespace: lenz_engine::WhitespaceMode::Ignore, ..Options::default() };
        let (left, right) = ("\tf(a, b)  \nx\n", "f( a,c )\nx\n");
        let b = &compare_lines(&split_lines(left), &split_lines(right), &ignore)[0];
        assert_eq!(moved_text(right, left, b.right.clone(), b.left.clone()), left);
    }

    #[test]
    fn a_move_written_keeps_crlf_and_bom() {
        let dir = tempfile::tempdir().unwrap();
        let (l, r) = (dir.path().join("l.txt"), dir.path().join("r.txt"));
        fs::write(&l, "one\ntwo\nthree\n").unwrap();
        fs::write(&r, b"\xEF\xBB\xBFone\r\n2\r\nthree\r\n").unwrap();
        let (snap, decoded) = Snapshot::read(&r, None).unwrap();
        let left = fs::read_to_string(&l).unwrap();
        let blocks = compare_lines(&split_lines(&left), &split_lines(&decoded.text), &Options::default());
        let b = &blocks[0];
        write_text(&snap, &moved_text(&decoded.text, &left, b.right.clone(), b.left.clone())).unwrap();
        assert_eq!(fs::read(&r).unwrap(), b"\xEF\xBB\xBFone\r\ntwo\r\nthree\r\n");
        // ≪ into the LF file from the CRLF one: LF stays
        let (snap, decoded) = Snapshot::read(&l, None).unwrap();
        let right = Snapshot::read(&r, None).unwrap().1.text;
        let text = moved_text(&decoded.text, &right, 0..1, 0..1);
        assert_eq!(text, "one\ntwo\nthree\n");
        write_text(&snap, &text).unwrap();
        assert_eq!(fs::read(&l).unwrap(), b"one\ntwo\nthree\n");
    }

    #[test]
    fn only_real_files_outside_the_temp_directory_are_editable() {
        let dir = tempfile::tempdir().unwrap();
        let (temp, work) = (dir.path().join("tmp"), dir.path().join("work"));
        fs::create_dir_all(&temp).unwrap();
        fs::create_dir_all(&work).unwrap();
        let real = work.join("a.txt");
        fs::write(&real, "a\n").unwrap();
        assert_eq!(editable(&Content::File(real.clone()), &temp), Ok(()));
        // a revision git copied into the temp directory
        fs::write(temp.join("left_a.txt"), "a\n").unwrap();
        assert_eq!(editable(&Content::File(temp.join("left_a.txt")), &temp), Err("a temporary copy"));
        // `git difftool -d`: a link in the temp directory to the working-tree file
        symlink(&real, temp.join("right_a.txt")).unwrap();
        assert_eq!(editable(&Content::File(temp.join("right_a.txt")), &temp), Ok(()));
        // …but not to a working-tree link
        symlink(&real, work.join("link.txt")).unwrap();
        symlink(work.join("link.txt"), temp.join("right_link.txt")).unwrap();
        assert_eq!(editable(&Content::File(temp.join("right_link.txt")), &temp), Err("a symlink"));
        // revision, index and PR Sides, absent Sides, working-tree links as git lists them
        assert_eq!(editable(&Content::Blob("HEAD:a.txt".into()), &temp), Err("a git revision"));
        assert_eq!(editable(&Content::Empty, &temp), Err("absent"));
        assert_eq!(editable(&Content::Symlink(work.join("link.txt")), &temp), Err("a symlink"));
        assert_eq!(editable(&Content::File(work.join("gone.txt")), &temp), Err("missing on disk"));
    }

    #[test]
    fn a_symlinked_working_tree_file_is_read_only() {
        use crate::git::testing::{git, repo, write};
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "s\n").unwrap();
        let dir = repo();
        write(dir.path(), "a.txt", "a\n");
        symlink(outside.path().join("secret.txt"), dir.path().join("link.txt")).unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "c"]);
        write(dir.path(), "a.txt", "b\n");
        // the repository itself sits in the temp directory here; pretend the system's is elsewhere
        let temp = dir.path().join("no-such-temp");
        let set = crate::changeset::ChangeSet::load(
            &crate::changeset::Spec::Git { cached: false, revs: vec![], paths: vec![], untracked: false },
            dir.path(),
        )
        .unwrap();
        let a = set.comparisons.iter().find(|c| c.path == "a.txt").unwrap();
        assert_eq!(editable(&a.right, &temp), Ok(()));
        assert_eq!(editable(&a.left, &temp), Err("a git revision"));
        // a symlink listed as a file (an unmerged or untracked link) is still a symlink
        assert_eq!(editable(&Content::File(dir.path().join("link.txt")), &temp), Err("a symlink"));
    }

    #[test]
    fn undo_and_redo_step_through_the_texts_a_file_held() {
        let mut e = Edits::default();
        let mut text = "v1".to_string();
        for next in ["v2", "v3"] {
            e.record(Side::Right, &text, next);
            text = next.into();
        }
        // undo twice: back to v1
        for want in ["v2", "v1"] {
            let (side, before) = e.next(false, ["", &text]).unwrap();
            assert_eq!((side, before.as_str()), (Side::Right, want));
            e.stepped(false, &text, &before);
            text = before;
        }
        assert!(e.next(false, ["", &text]).is_none());
        // redo once, then a new move drops the rest of the redo
        let (_, after) = e.next(true, ["", &text]).unwrap();
        assert_eq!(after, "v2");
        e.stepped(true, &text, &after);
        text = after;
        assert_eq!(e.next(true, ["", &text]).map(|s| s.1), Some("v3".into()));
        e.record(Side::Left, &text, "other");
        assert!(e.next(true, ["", &text]).is_none());
        assert_eq!(e.next(false, ["other", &text]), Some((Side::Left, "v2".to_string())));
    }

    #[test]
    fn small_changes_do_not_keep_a_full_file_per_undo_step() {
        let mut edits = Edits::default();
        let prefix = "unchanged line\n".repeat(100_000);
        for i in 0..10 {
            edits.record(Side::Right, &format!("{prefix}value {i}\n"), &format!("{prefix}value {}\n", i + 1));
        }
        let retained: usize = edits.undo.iter().map(|e| e.text.capacity()).sum();
        assert!(retained < 1024, "ten one-character edits retained {retained} bytes");
        assert_eq!(retained, 10);
        let mut text = format!("{prefix}value 10\n");
        for i in (0..10).rev() {
            let (_, previous) = edits.next(false, ["", &text]).unwrap();
            assert_eq!(previous, format!("{prefix}value {i}\n"));
            edits.stepped(false, &text, &previous);
            text = previous;
        }
        assert!(edits.redo.iter().map(|e| e.text.capacity()).sum::<usize>() < 1024);
        for i in 1..=10 {
            let (_, next) = edits.next(true, ["", &text]).unwrap();
            assert_eq!(next, format!("{prefix}value {i}\n"));
            edits.stepped(true, &text, &next);
            text = next;
        }
    }

    #[test]
    fn patches_round_trip_utf8_insertions_deletions_and_line_endings() {
        let fragments = ["", "a", "é", "ê", "ĩ", "🙂", "😄", "\n", "\r\n", "\u{feff}"];
        for before in fragments {
            for after in fragments {
                for (prefix, suffix) in [("", ""), ("α\r\n", "\nω")] {
                    let before = format!("{prefix}{before}{suffix}");
                    let after = format!("{prefix}{after}{suffix}");
                    for side in [Side::Left, Side::Right] {
                        let mut edits = Edits::default();
                        edits.record(side, &before, &after);
                        let mut texts = ["untouched".to_string(), "untouched".to_string()];
                        texts[side as usize] = after.clone();
                        for redo in [false, true, false, true] {
                            let (got, next) = edits.next(redo, [&texts[0], &texts[1]]).unwrap();
                            assert_eq!(got, side);
                            assert_eq!(next, if redo { &after } else { &before }.as_str());
                            edits.stepped(redo, &texts[side as usize], &next);
                            texts[side as usize] = next;
                            assert_eq!(texts[1 - side as usize], "untouched");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn patches_refuse_a_different_file_version_without_advancing_history() {
        let mut edits = Edits::default();
        edits.record(Side::Right, "before\nold\n", "before\nnew\n");
        assert!(edits.next(false, ["", "changed elsewhere\nnew\n"]).is_none());
        assert_eq!(edits.next(false, ["", "before\nnew\n"]).unwrap().1, "before\nold\n");
    }

    #[test]
    fn reversing_a_normalized_write_uses_the_actual_written_text() {
        let mut edits = Edits::default();
        edits.record(Side::Right, "old\nsame\r\n", "new\r\nsame\r\n");
        let proposed = edits.next(false, ["", "new\r\nsame\r\n"]).unwrap().1;
        let written = crate::write::with_crlf(&proposed);
        edits.stepped(false, "new\r\nsame\r\n", &written);
        assert_eq!(edits.next(true, ["", &written]).unwrap().1, "new\r\nsame\r\n");
    }
}
