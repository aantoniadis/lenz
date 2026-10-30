//! Optional modal editing; adapters forward keys only when the source editor has focus.
use super::*;
use crate::text::{EditKind, SnapshotKey, TextEdit, TextSnapshot};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VimMode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,
}

#[derive(Default)]
pub(super) struct Vim {
    mode: VimMode,
    count: usize,
    operator: Option<(char, usize)>,
    prefix: Option<char>,
    anchor: usize,
    caret: usize,
    pub(super) context: Option<(TabId, SnapshotKey, Option<Selection>)>,
    register: String,
    linewise: bool,
    command: String,
}
impl Vim {
    fn clear(&mut self) {
        self.count = 0;
        self.operator = None;
        self.prefix = None;
    }
    pub(super) fn normal(&mut self) {
        self.mode = VimMode::Normal;
        self.clear();
        self.command.clear();
    }
    fn count(&mut self) -> usize {
        std::mem::take(&mut self.count).max(1)
    }
}
impl State {
    pub fn relative_line_numbers(&self) -> bool {
        self.settings.relative_line_numbers
    }
    pub fn vim_enabled(&self) -> bool {
        self.settings.vim_enabled
    }
    pub fn vim_mode(&self) -> VimMode {
        self.vim.mode
    }
    /// Source-line numbering only; layout retains absolute identities for marks and hit testing.
    pub fn line_number(&self, tab: TabId, line: usize) -> usize {
        let Some(view) = self.view(tab) else { return line.saturating_add(1) };
        let caret = self
            .relative_number_origin(tab)
            .unwrap_or_else(|| view.tab().selection().map_or(view.tab().top_line(), |s| view.snapshot().line_of(s.head)));
        if self.relative_line_numbers() && line != caret { line.abs_diff(caret) } else { line.saturating_add(1) }
    }
    pub fn relative_number_origin(&self, tab: TabId) -> Option<usize> {
        if !self.relative_line_numbers() {
            return None;
        }
        let view = self.view(tab)?;
        let visual = self.vim_enabled()
            && self.active == Some(tab)
            && matches!(self.vim.mode, VimMode::Visual | VimMode::VisualLine)
            && self.vim.context == Some((tab, view.snapshot().key(), view.tab().selection()));
        Some(view.snapshot().line_of(if visual {
            self.vim.caret
        } else {
            view.tab().selection().map_or_else(|| view.snapshot().reader_line_range(view.tab().top_line()).start, |s| s.head)
        }))
    }
    pub fn vim_status(&self) -> Option<String> {
        if !self.vim_enabled() {
            return None;
        }
        let v = &self.vim;
        if v.mode == VimMode::Command {
            return Some(v.command.clone());
        }
        let mode = match v.mode {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
            VimMode::Visual => "VISUAL",
            VimMode::VisualLine => "VISUAL LINE",
            VimMode::Command => "COMMAND",
        };
        let mut status = mode.to_owned();
        if let Some((op, count)) = v.operator {
            status.push_str(&format!(" {count}{op}"));
        }
        if v.count > 0 {
            status.push_str(&format!(" {}", v.count));
        }
        if let Some(prefix) = v.prefix {
            status.push(prefix);
        }
        Some(status)
    }
}
fn character(key: &keymap::KeyPress) -> Option<char> {
    let mut chars = key.key.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if !key.shift {
        return Some(c);
    }
    if c.is_ascii_lowercase() {
        return Some(c.to_ascii_uppercase());
    }
    Some(match c {
        '4' => '$',
        '6' => '^',
        ';' => ':',
        '5' => '%',
        '/' => '?',
        _ => c,
    })
}
fn class(c: Option<char>) -> u8 {
    match c {
        None => 0,
        Some(c) if c.is_whitespace() => 0,
        Some(c) if c.is_alphanumeric() || c == '_' => 1,
        Some(_) => 2,
    }
}
fn at(doc: &TextSnapshot, pos: usize) -> Option<char> {
    doc.range(pos..doc.next_char_boundary(pos))?.chars().next()
}
fn normal_position(doc: &TextSnapshot, pos: usize) -> usize {
    let line = doc.reader_line_range(doc.line_of(pos));
    let end = if line.is_empty() { line.start } else { doc.previous_char_boundary(line.end) };
    doc.floor_char_boundary(pos).clamp(line.start, end)
}
fn first_nonblank(doc: &TextSnapshot, line: usize) -> usize {
    let r = doc.reader_line_range(line);
    let mut p = r.start;
    while p < r.end && at(doc, p).is_some_and(|c| c == ' ' || c == '\t') {
        p = doc.next_char_boundary(p);
    }
    p
}
fn word(doc: &TextSnapshot, mut p: usize, motion: char, count: usize) -> usize {
    for _ in 0..count {
        if motion == 'b' {
            if p == 0 {
                break;
            }
            p = doc.previous_char_boundary(p);
            while p > 0 && class(at(doc, p)) == 0 {
                p = doc.previous_char_boundary(p);
            }
            let kind = class(at(doc, p));
            while p > 0 {
                let prev = doc.previous_char_boundary(p);
                if class(at(doc, prev)) != kind {
                    break;
                }
                p = prev;
            }
        } else if motion == 'w' {
            let kind = class(at(doc, p));
            while p < doc.len_bytes() && class(at(doc, p)) == kind {
                p = doc.next_char_boundary(p);
            }
            while p < doc.len_bytes() && class(at(doc, p)) == 0 {
                p = doc.next_char_boundary(p);
            }
        } else {
            if p < doc.len_bytes() {
                p = doc.next_char_boundary(p);
            }
            while p < doc.len_bytes() && class(at(doc, p)) == 0 {
                p = doc.next_char_boundary(p);
            }
            let kind = class(at(doc, p));
            while p < doc.len_bytes() {
                let next = doc.next_char_boundary(p);
                if class(at(doc, next)) != kind {
                    break;
                }
                p = next;
            }
        }
    }
    p
}
impl Session {
    pub fn vim_insert_allowed(&self) -> bool {
        !self.state.vim_enabled() || self.state.vim_mode() == VimMode::Insert
    }
    pub fn vim_handles(&self, key: &keymap::KeyPress) -> bool {
        if !self.state.vim_enabled() || !self.state.active().is_some_and(|t| shows_text(&self.state, t) && t.csv.is_none()) {
            return false;
        }
        if key.alt {
            return false;
        }
        if key.ctrl {
            return key.key == "[" || self.state.vim_mode() != VimMode::Insert && matches!(key.key.as_str(), "r" | "f" | "b");
        }
        if key.key == "escape" || key.key == "esc" {
            return true;
        }
        self.state.vim_mode() != VimMode::Insert
            && (character(key).is_some()
                || matches!(
                    key.key.as_str(),
                    "enter" | "backspace" | "delete" | "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown" | "tab"
                ))
    }
    pub(super) fn set_vim_enabled(&mut self, on: bool) -> Effect {
        self.state.settings.vim_enabled = on;
        self.state.vim.normal();
        self.state.vim.context = None;
        self.cancel_completion();
        self.save_settings();
        if on {
            self.vim_place_current();
        }
        Effect { repaint: true, ..Effect::default() }
    }
    fn vim_place_current(&mut self) {
        let Some(view) = self.state.active().and_then(|t| self.state.view(t.id())) else { return };
        let head = normal_position(
            view.snapshot(),
            view.tab().selection().map_or_else(|| view.snapshot().reader_line_range(view.tab().top_line()).start, |s| s.head),
        );
        self.command(Command::Select(Selection { anchor: head, head }));
    }
    fn vim_context(&self) -> Option<(TabId, SnapshotKey, Option<Selection>)> {
        let t = self.state.active()?;
        Some((t.id(), self.state.view(t.id())?.snapshot().key(), t.selection()))
    }
    pub(super) fn vim_key(&mut self, key: keymap::KeyPress) -> Effect {
        if !self.vim_handles(&key) {
            return Effect::default();
        }
        let now = self.vim_context();
        if self.state.vim.context.is_some() && self.state.vim.context != now {
            let tab_changed = self.state.vim.context.map(|x| x.0) != now.map(|x| x.0);
            if tab_changed || self.state.vim.mode != VimMode::Insert {
                self.state.vim.normal();
            } else {
                self.state.vim.clear();
            }
        }
        let mut effect = self.vim_key_inner(key);
        self.state.vim.context = self.vim_context();
        effect.repaint = true;
        effect
    }
    fn vim_key_inner(&mut self, key: keymap::KeyPress) -> Effect {
        if matches!(key.key.as_str(), "escape" | "esc") || key.ctrl && key.key == "[" {
            let insert = self.state.vim.mode == VimMode::Insert;
            self.state.vim.normal();
            self.cancel_completion();
            self.cancel_signature();
            if insert {
                if let Some(view) = self.state.active().and_then(|t| self.state.view(t.id())) {
                    let head = view.tab().selection().map_or(0, |s| s.head);
                    let start = view.snapshot().reader_line_range(view.snapshot().line_of(head)).start;
                    let p = if head > start { view.snapshot().previous_char_boundary(head) } else { head };
                    self.command(Command::Select(Selection { anchor: p, head: p }));
                }
            } else {
                self.vim_place_current();
            }
            return Effect::default();
        }
        if self.state.vim.mode == VimMode::Command {
            match key.key.as_str() {
                "enter" => return self.vim_command(),
                "backspace" => {
                    self.state.vim.command.pop();
                    if self.state.vim.command.is_empty() {
                        self.state.vim.normal();
                    }
                }
                _ => {
                    if let Some(c) = character(&key).filter(|c| !c.is_control())
                        && self.state.vim.command.len() < 1024
                    {
                        self.state.vim.command.push(c);
                    }
                }
            }
            return Effect::default();
        }
        let Some(view) = self.state.active().and_then(|t| self.state.view(t.id())) else { return Effect::default() };
        let (tab, doc) = (view.tab().id(), view.snapshot().clone());
        let visible = view.tab().visible_for(&doc);
        let newline = view.input_newline();
        let visual = matches!(self.state.vim.mode, VimMode::Visual | VimMode::VisualLine);
        let p = if visual {
            self.state.vim.caret
        } else {
            normal_position(&doc, view.tab().selection().map_or_else(|| doc.reader_line_range(view.tab().top_line()).start, |s| s.head))
        };
        if key.ctrl {
            if key.key == "r" {
                self.state.vim.clear();
                let e = self.command(Command::Redo { tab });
                self.vim_place_current();
                return e;
            }
            return self.command(Command::ScrollRows(if key.key == "f" {
                self.state.text_size[self.state.focused as usize].1.max(1) as isize
            } else {
                -(self.state.text_size[self.state.focused as usize].1.max(1) as isize)
            }));
        }
        let c = match key.key.as_str() {
            "left" => 'h',
            "right" => 'l',
            "up" => 'k',
            "down" => 'j',
            "home" => '0',
            "end" => '$',
            _ => character(&key).unwrap_or('\0'),
        };
        if self.state.vim.prefix != Some('r') && c.is_ascii_digit() && (c != '0' || self.state.vim.count > 0) {
            self.state.vim.count = (self.state.vim.count.saturating_mul(10) + c.to_digit(10).unwrap_or(0) as usize).min(9999);
            return Effect::default();
        }
        let explicit_count = self.state.vim.count > 0;
        let count = self.state.vim.count();
        if let Some(prefix) = self.state.vim.prefix.take() {
            match prefix {
                'r' => {
                    if c != '\0' && !c.is_control() {
                        let mut end = p;
                        let line = doc.reader_line_range(doc.line_of(p));
                        for _ in 0..count {
                            if end >= line.end {
                                break;
                            }
                            end = doc.next_char_boundary(end);
                        }
                        if end > p {
                            let n = doc.range(p..end).map_or(0, |t| t.chars().count());
                            return self.vim_replace(tab, &doc, p..end, c.to_string().repeat(n), p, false);
                        }
                    }
                    self.state.vim.clear();
                    return Effect::default();
                }
                'g' if c == 'g' => {
                    let line = if explicit_count { count - 1 } else { self.state.vim.operator.map_or(0, |(_, n)| n.saturating_sub(1)) };
                    let to = first_nonblank(&doc, line.min(doc.line_count() - 1));
                    return self.vim_motion(tab, &doc, p, to, false, true);
                }
                'i' if c == 'w' => {
                    let r = doc.word_at(p);
                    return self.vim_operator(tab, &doc, r, false);
                }
                _ => {
                    self.state.vim.clear();
                    return Effect::default();
                }
            }
        }
        if self.state.vim.operator.is_some() && c == 'i' {
            self.state.vim.prefix = Some('i');
            return Effect::default();
        }
        if let Some((op, n)) = self.state.vim.operator
            && c == op
        {
            return self.vim_lines(tab, &doc, p, n.saturating_mul(count));
        }
        let count = count.saturating_mul(self.state.vim.operator.map_or(1, |(_, n)| n)).min(9999);
        let range = doc.reader_line_range(doc.line_of(p));
        let to = match c {
            'h' => {
                let mut to = p;
                for _ in 0..count {
                    if to <= range.start {
                        break;
                    }
                    to = doc.previous_char_boundary(to);
                }
                Some((to, false, false))
            }
            'l' => {
                let mut to = p;
                for _ in 0..count {
                    if to >= range.end {
                        break;
                    }
                    to = doc.next_char_boundary(to);
                }
                Some((to, false, false))
            }
            'j' | 'k' => {
                let line = visible.step(doc.line_of(p), if c == 'j' { count as isize } else { -(count as isize) });
                let r = doc.reader_line_range(line);
                Some((doc.floor_char_boundary(r.start + (p - range.start).min(r.len())), false, true))
            }
            'w' | 'b' | 'e' => {
                let motion =
                    if c == 'w' && self.state.vim.operator.is_some_and(|(o, _)| o == 'c') && class(at(&doc, p)) > 0 { 'e' } else { c };
                let to = if c == 'w' && motion == 'e' {
                    let end = doc.word_at(p).end;
                    let end = doc.previous_char_boundary(end);
                    if count > 1 { word(&doc, end, 'e', count - 1) } else { end }
                } else {
                    word(&doc, p, motion, count)
                };
                Some((to, motion == 'e', false))
            }
            '0' => Some((range.start, false, false)),
            '^' => Some((first_nonblank(&doc, doc.line_of(p)), false, false)),
            '$' => Some((doc.reader_line_range((doc.line_of(p) + count - 1).min(doc.line_count() - 1)).end, false, false)),
            'G' => Some((
                first_nonblank(&doc, if explicit_count { count.saturating_sub(1).min(doc.line_count() - 1) } else { doc.line_count() - 1 }),
                false,
                true,
            )),
            _ => None,
        };
        if let Some((to, inclusive, linewise)) = to {
            return self.vim_motion(tab, &doc, p, to, inclusive, linewise);
        }
        match c {
            'g' => {
                self.state.vim.prefix = Some('g');
                if explicit_count {
                    self.state.vim.count = count;
                }
            }
            'i' | 'a' | 'I' | 'A' => {
                if visual {
                    self.state.vim.normal();
                    return Effect::default();
                }
                let to = match c {
                    'a' => doc.next_char_boundary(p).min(range.end),
                    'I' => first_nonblank(&doc, doc.line_of(p)),
                    'A' => range.end,
                    _ => p,
                };
                self.state.vim.clear();
                self.state.vim.mode = VimMode::Insert;
                self.command(Command::Select(Selection { anchor: to, head: to }));
            }
            'v' | 'V' => {
                if visual {
                    self.state.vim.normal();
                    self.vim_place_current();
                } else {
                    self.state.vim.mode = if c == 'v' { VimMode::Visual } else { VimMode::VisualLine };
                    self.state.vim.anchor = p;
                    self.vim_select(&doc, p);
                }
            }
            'd' | 'c' | 'y' => {
                if visual {
                    self.state.vim.operator = Some((c, 1));
                    let range = self.state.selection().map(|s| s.range()).unwrap_or(p..p);
                    return self.vim_operator(tab, &doc, range, self.state.vim.mode == VimMode::VisualLine);
                } else {
                    self.state.vim.operator = Some((c, count));
                }
            }
            'D' | 'C' => {
                self.state.vim.operator = Some((if c == 'D' { 'd' } else { 'c' }, 1));
                return self.vim_operator(tab, &doc, p..range.end, false);
            }
            'Y' => {
                self.state.vim.operator = Some(('y', 1));
                return self.vim_lines(tab, &doc, p, count);
            }
            'x' | 's' => {
                let mut end = p;
                for _ in 0..count {
                    if end >= range.end {
                        break;
                    }
                    end = doc.next_char_boundary(end);
                }
                self.state.vim.operator = Some((if c == 'x' { 'd' } else { 'c' }, 1));
                return self.vim_operator(tab, &doc, p..end, false);
            }
            'r' => {
                self.state.vim.prefix = Some('r');
                self.state.vim.count = count;
            }
            'p' | 'P' => return self.vim_paste(tab, &doc, p, c == 'P', count),
            'o' | 'O' => {
                let first = first_nonblank(&doc, doc.line_of(p));
                let indent = doc.range(range.start..first).unwrap_or_default().to_string();
                let (pos, text, caret) = if c == 'o' {
                    (range.end, format!("{newline}{indent}"), range.end + newline.len() + indent.len())
                } else {
                    (range.start, format!("{indent}{newline}"), range.start + indent.len())
                };
                return self.vim_replace(tab, &doc, pos..pos, text, caret, true);
            }
            'u' => {
                self.state.vim.normal();
                let e = self.command(Command::Undo { tab });
                self.vim_place_current();
                return e;
            }
            ':' | '/' => {
                self.state.vim.clear();
                self.state.vim.mode = VimMode::Command;
                self.state.vim.command = c.to_string();
            }
            'n' | 'N' => return self.command(Command::FindStep { forward: c == 'n' }),
            _ => self.state.vim.clear(),
        }
        Effect::default()
    }
    fn vim_select(&mut self, doc: &TextSnapshot, to: usize) {
        let to = normal_position(doc, to);
        self.state.vim.caret = to;
        let anchor = self.state.vim.anchor;
        let range = if self.state.vim.mode == VimMode::VisualLine {
            let first = doc.line_of(anchor).min(doc.line_of(to));
            let last = doc.line_of(anchor).max(doc.line_of(to));
            doc.reader_line_range(first).start..doc.line_with_ending(last).map_or(doc.len_bytes(), |r| r.end)
        } else {
            anchor.min(to)..doc.next_char_boundary(anchor.max(to)).min(doc.reader_line_range(doc.line_of(anchor.max(to))).end)
        };
        self.command(Command::Select(if to < anchor {
            Selection { anchor: range.end, head: range.start }
        } else {
            Selection { anchor: range.start, head: range.end }
        }));
    }
    fn vim_motion(&mut self, tab: TabId, doc: &TextSnapshot, from: usize, to: usize, inclusive: bool, linewise: bool) -> Effect {
        if self.state.vim.operator.is_some() {
            if linewise {
                let first = doc.line_of(from).min(doc.line_of(to));
                let last = doc.line_of(from).max(doc.line_of(to));
                return self.vim_operator(
                    tab,
                    doc,
                    doc.reader_line_range(first).start..doc.line_with_ending(last).map_or(doc.len_bytes(), |r| r.end),
                    true,
                );
            }
            let end = if inclusive { doc.next_char_boundary(from.max(to)) } else { from.max(to) };
            return self.vim_operator(tab, doc, from.min(to)..end.min(doc.len_bytes()), false);
        }
        if matches!(self.state.vim.mode, VimMode::Visual | VimMode::VisualLine) {
            self.vim_select(doc, to);
        } else {
            let p = normal_position(doc, to);
            self.command(Command::Select(Selection { anchor: p, head: p }));
        }
        self.state.vim.clear();
        Effect::default()
    }
    fn vim_lines(&mut self, tab: TabId, doc: &TextSnapshot, p: usize, count: usize) -> Effect {
        let first = doc.line_of(p);
        let last = first.saturating_add(count - 1).min(doc.line_count() - 1);
        self.vim_operator(tab, doc, doc.reader_line_range(first).start..doc.line_with_ending(last).map_or(doc.len_bytes(), |r| r.end), true)
    }
    fn vim_operator(&mut self, tab: TabId, doc: &TextSnapshot, mut range: Range<usize>, linewise: bool) -> Effect {
        let Some((op, _)) = self.state.vim.operator.take() else { return Effect::default() };
        if op != 'y' && self.text_action_disabled(CommandId::Paste).is_some() {
            self.state.vim.normal();
            return Effect::default();
        }
        let Some(text) = doc.copy_range(range.clone()) else { return Effect::default() };
        self.state.vim.register =
            if linewise && text.is_empty() { self.state.view(tab).map_or("\n", |v| v.input_newline()).to_owned() } else { text };
        self.state.vim.linewise = linewise;
        self.state.vim.normal();
        if op == 'y' {
            let p = normal_position(doc, range.start);
            self.command(Command::Select(Selection { anchor: p, head: p }));
            return Effect::default();
        }
        if linewise && op == 'c' {
            range.end = doc.reader_line_range(doc.line_of(range.end.saturating_sub(1))).end;
        } else if linewise
            && range.end == doc.len_bytes()
            && range.start > 0
            && (range.is_empty() || !doc.range(range.end.saturating_sub(1)..range.end).is_some_and(|s| s == "\n"))
        {
            range.start = doc.reader_line_range(doc.line_of(range.start) - 1).end;
        }
        let p = range.start;
        self.vim_replace(tab, doc, range, String::new(), p, op == 'c')
    }
    fn vim_replace(&mut self, tab: TabId, doc: &TextSnapshot, range: Range<usize>, text: String, caret: usize, insert: bool) -> Effect {
        if self.text_action_disabled(CommandId::Paste).is_some() {
            self.state.vim.normal();
            return Effect::default();
        }
        self.vim_group_edit = insert;
        self.state.vim.mode = if insert { VimMode::Insert } else { VimMode::Normal };
        let effect = self.command(Command::Edit {
            tab,
            expected: doc.key(),
            edits: vec![TextEdit { range, text: text.into() }],
            kind: EditKind::Replace,
        });
        self.state.vim.clear();
        self.state.vim.mode = if insert { VimMode::Insert } else { VimMode::Normal };
        if let Some(now) = self.state.view(tab) {
            let p = if insert { now.snapshot().floor_char_boundary(caret) } else { normal_position(now.snapshot(), caret) };
            self.command(Command::Select(Selection { anchor: p, head: p }));
        }
        effect
    }
    fn vim_paste(&mut self, tab: TabId, doc: &TextSnapshot, p: usize, before: bool, count: usize) -> Effect {
        if self.state.vim.register.is_empty() {
            self.state.vim.normal();
            return Effect::default();
        }
        let Some(_size) = self.state.vim.register.len().checked_mul(count).filter(|&n| n <= 16 << 20) else {
            self.state.messages.push("Vim paste is larger than 16 MiB".into());
            return Effect::default();
        };
        let text = self.state.vim.register.repeat(count);
        let line = doc.reader_line_range(doc.line_of(p));
        let (pos, text) = if self.state.vim.linewise {
            let newline = self.state.view(tab).map_or("\n", |v| v.input_newline());
            if before {
                (line.start, if text.ends_with('\n') { text } else { format!("{text}{newline}") })
            } else {
                let end = doc.line_with_ending(doc.line_of(p)).map_or(line.end, |r| r.end);
                if end > line.end {
                    (end, if text.ends_with('\n') { text } else { format!("{text}{newline}") })
                } else {
                    (end, format!("{newline}{}", text.strip_suffix("\r\n").or_else(|| text.strip_suffix('\n')).unwrap_or(&text)))
                }
            }
        } else {
            (if before { p } else { doc.next_char_boundary(p).min(line.end) }, text)
        };
        self.vim_replace(tab, doc, pos..pos, text, pos, false)
    }
    fn vim_command(&mut self) -> Effect {
        let command = std::mem::take(&mut self.state.vim.command);
        self.state.vim.normal();
        if let Some(query) = command.strip_prefix('/') {
            return self.command(Command::Find(query.to_owned()));
        }
        let Some(tab) = self.state.active().map(|t| t.id()) else { return Effect::default() };
        let command = command.trim_start_matches(':').trim();
        match command {
            "w" | "write" => self.command(Command::Save { tab }),
            "q" | "quit" => self.command(Command::CloseTab(tab)),
            "wq" | "x" | "q!" => {
                let effect = self.command(Command::CloseTab(tab));
                if let Some(prompt) = self.close_prompt() {
                    let choice = if command == "q!" { crate::close::Choice::Discard } else { crate::close::Choice::Save };
                    return self.command(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice });
                }
                effect
            }
            "set relativenumber" | "set rnu" => self.command(Command::SetRelativeLineNumbers(true)),
            "set norelativenumber" | "set nornu" => self.command(Command::SetRelativeLineNumbers(false)),
            number if number.parse::<usize>().is_ok() => {
                let line = number.parse::<u32>().unwrap_or(u32::MAX).max(1);
                self.command(Command::GoToLine(lenz_content::goto::LineColumn { line, column: Some(1) }))
            }
            _ => {
                self.state.messages.push(format!("Unsupported Vim command: {command}"));
                Effect::default()
            }
        }
    }
}
