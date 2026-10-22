//! Read-only caret and selection on one Side (gpui-free). Positions are raw lines and byte
//! columns, so text inside Collapsed Regions is part of a selection and Filler Rows never are.

use std::ops::Range;

use crate::view::Side;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    /// 0-based raw line.
    pub line: u32,
    /// Byte offset within the line, on a char boundary.
    pub col: usize,
}

impl Pos {
    pub fn new(line: u32, col: usize) -> Self {
        Pos { line, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    Left,
    Right,
    /// To this line (the view picks it, stepping over Collapsed Regions), keeping the column.
    ToLine(u32),
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
}

#[derive(Clone,Copy)]
pub enum CaretCommand {Left,Right,Up,Down,PageUp,PageDown,Home(bool),End(bool)}
pub fn move_caret(mut selection:Selection,lines:&[String],view:&mut crate::view::View,command:CaretCommand,extend:bool)->Selection{
    let page=(view.viewport_rows()-2.0).max(1.0)as i32;
    let motion=match command {CaretCommand::Left=>Motion::Left,CaretCommand::Right=>Motion::Right,CaretCommand::Up=>Motion::ToLine(view.step_line(selection.side,selection.head.line,-1)),CaretCommand::Down=>Motion::ToLine(view.step_line(selection.side,selection.head.line,1)),CaretCommand::PageUp=>Motion::ToLine(view.step_line(selection.side,selection.head.line,-page)),CaretCommand::PageDown=>Motion::ToLine(view.step_line(selection.side,selection.head.line,page)),CaretCommand::Home(doc)=>if doc{Motion::DocStart}else{Motion::LineStart},CaretCommand::End(doc)=>if doc{Motion::DocEnd}else{Motion::LineEnd}};
    selection.apply(lines,motion,extend);view.reveal(selection.side,selection.head.line);selection
}

/// A selection on one Side; `anchor == head` is a bare caret.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub side: Side,
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub fn caret(side: Side, at: Pos) -> Self {
        Selection { side, anchor: at, head: at }
    }

    pub fn all(side: Side, lines: &[String]) -> Self {
        let end = last(lines);
        Selection { side, anchor: Pos::new(0, 0), head: end }
    }

    /// The whole word (or run of non-word characters) around `at`.
    pub fn word(side: Side, lines: &[String], at: Pos) -> Self {
        let line = line_text(lines, at.line);
        let r = word_at(line, at.col);
        Selection { side, anchor: Pos::new(at.line, r.start), head: Pos::new(at.line, r.end) }
    }

    /// The whole line, through the start of the next one.
    pub fn line(side: Side, lines: &[String], line: u32) -> Self {
        let next = if (line as usize) + 1 < lines.len() {
            Pos::new(line + 1, 0)
        } else {
            Pos::new(line, line_text(lines, line).len())
        };
        Selection { side, anchor: Pos::new(line, 0), head: next }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Start and end, in file order.
    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    /// The selected text, lines joined with `\n`.
    pub fn text(&self, lines: &[String]) -> String {
        let (a, b) = self.ordered();
        if a.line == b.line {
            return line_text(lines, a.line).get(a.col..b.col).unwrap_or("").to_string();
        }
        let mut out = line_text(lines, a.line).get(a.col..).unwrap_or("").to_string();
        for l in a.line + 1..b.line {
            out.push('\n');
            out.push_str(line_text(lines, l));
        }
        out.push('\n');
        out.push_str(line_text(lines, b.line).get(..b.col).unwrap_or(""));
        out
    }

    /// Selected bytes of `line` (of length `len`), if any.
    pub fn on_line(&self, line: u32, len: usize) -> Option<Range<usize>> {
        let (a, b) = self.ordered();
        if self.is_empty() || line < a.line || line > b.line {
            return None;
        }
        let start = if line == a.line { a.col } else { 0 };
        let end = if line == b.line { b.col } else { len };
        (start < end).then_some(start..end)
    }

    /// Move the head; `extend` keeps the anchor (shift), otherwise the selection collapses to a caret.
    pub fn apply(&mut self, lines: &[String], motion: Motion, extend: bool) {
        let h = self.head;
        let text = line_text(lines, h.line);
        let head = match motion {
            // without shift, ←/→ on a selection lands on its edge
            Motion::Left if !extend && !self.is_empty() => self.ordered().0,
            Motion::Right if !extend && !self.is_empty() => self.ordered().1,
            Motion::Left => match text[..h.col].chars().next_back() {
                Some(c) => Pos::new(h.line, h.col - c.len_utf8()),
                None if h.line > 0 => Pos::new(h.line - 1, line_text(lines, h.line - 1).len()),
                None => h,
            },
            Motion::Right => match text[h.col..].chars().next() {
                Some(c) => Pos::new(h.line, h.col + c.len_utf8()),
                None if (h.line as usize) + 1 < lines.len() => Pos::new(h.line + 1, 0),
                None => h,
            },
            Motion::ToLine(line) => {
                // keep the column in characters
                let chars = text[..h.col].chars().count();
                let target = line_text(lines, line);
                Pos::new(line, target.char_indices().nth(chars).map_or(target.len(), |(i, _)| i))
            }
            Motion::LineStart => {
                // first press goes to the indent, a second to column 0
                let indent = text.len() - text.trim_start().len();
                Pos::new(h.line, if h.col == indent { 0 } else { indent })
            }
            Motion::LineEnd => Pos::new(h.line, text.len()),
            Motion::DocStart => Pos::new(0, 0),
            Motion::DocEnd => last(lines),
        };
        self.head = head;
        if !extend {
            self.anchor = head;
        }
    }
}

fn line_text(lines: &[String], line: u32) -> &str {
    lines.get(line as usize).map_or("", String::as_str)
}

fn last(lines: &[String]) -> Pos {
    let n = lines.len().saturating_sub(1) as u32;
    Pos::new(n, line_text(lines, n).len())
}

pub use lenz_content::select::word_at;

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn copying_joins_lines_with_newlines_and_keeps_partial_ends() {
        let l = lines(&["fn main() {", "    let x = 1;", "}"]);
        let s = Selection { side: Side::Left, anchor: Pos::new(0, 3), head: Pos::new(2, 1) };
        assert_eq!(s.text(&l), "main() {\n    let x = 1;\n}");
        // a backwards selection copies the same text
        let s = Selection { side: Side::Left, anchor: Pos::new(2, 1), head: Pos::new(0, 3) };
        assert_eq!(s.text(&l), "main() {\n    let x = 1;\n}");
        let s = Selection { side: Side::Left, anchor: Pos::new(1, 4), head: Pos::new(1, 7) };
        assert_eq!(s.text(&l), "let");
    }

    #[test]
    fn whole_line_and_select_all() {
        let l = lines(&["a", "bb", "ccc"]);
        assert_eq!(Selection::line(Side::Right, &l, 1).text(&l), "bb\n");
        assert_eq!(Selection::line(Side::Right, &l, 2).text(&l), "ccc");
        assert_eq!(Selection::all(Side::Right, &l).text(&l), "a\nbb\nccc");
    }

    #[test]
    fn arrows_move_by_character_across_line_ends_and_shift_extends() {
        let l = lines(&["ab", "é"]);
        let mut s = Selection::caret(Side::Left, Pos::new(0, 2));
        s.apply(&l, Motion::Right, true);
        assert_eq!(s.head, Pos::new(1, 0));
        s.apply(&l, Motion::Right, true);
        assert_eq!(s.head, Pos::new(1, 2)); // é is two bytes
        assert_eq!(s.text(&l), "\né");
        s.apply(&l, Motion::Left, false);
        assert!(s.is_empty());
        assert_eq!(s.head, Pos::new(0, 2), "← on a selection lands on its start");
    }

    #[test]
    fn vertical_moves_keep_the_character_column_and_home_toggles_indent() {
        let l = lines(&["    let x = 1;", "ab", "αβγδεζ"]);
        let mut s = Selection::caret(Side::Left, Pos::new(0, 5));
        s.apply(&l, Motion::ToLine(1), false);
        assert_eq!(s.head, Pos::new(1, 2));
        s.apply(&l, Motion::ToLine(2), false);
        assert_eq!(s.head, Pos::new(2, 4), "two Greek letters, four bytes");
        s.apply(&l, Motion::ToLine(0), false);
        s.apply(&l, Motion::LineStart, false);
        assert_eq!(s.head, Pos::new(0, 4));
        s.apply(&l, Motion::LineStart, false);
        assert_eq!(s.head, Pos::new(0, 0));
        s.apply(&l, Motion::DocEnd, true);
        assert_eq!(s.text(&l), "    let x = 1;\nab\nαβγδεζ");
    }

    #[test]
    fn per_line_highlight_ranges() {
        let s = Selection { side: Side::Left, anchor: Pos::new(1, 2), head: Pos::new(3, 1) };
        assert_eq!(s.on_line(0, 5), None);
        assert_eq!(s.on_line(1, 5), Some(2..5));
        assert_eq!(s.on_line(2, 7), Some(0..7));
        assert_eq!(s.on_line(3, 4), Some(0..1));
        assert_eq!(Selection::caret(Side::Left, Pos::new(1, 1)).on_line(1, 5), None);
    }
}
