//! Editing a single-line field. Adapters translate keys and transport clipboard text; selection and edits live here.
use crate::line_edit::{self, Edit};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Edit { edit: Edit, extend: bool },
    SelectAll,
    Copy,
    Cut,
    Paste,
    Insert(String),
    Clear,
}

impl Command {
    /// Path/name prompts retain the first pasted line; query fields can keep line breaks as spaces.
    pub fn first_line(mut self) -> Self {
        if let Self::Insert(text) = &mut self
            && let Some(end) = text.find(['\r', '\n'])
        {
            text.truncate(end);
        }
        self
    }
}

/// Toolkit-independent keyboard input. `command` is macOS Cmd; `control` and `alt` retain their actual meanings.
#[derive(Default)]
pub struct Key<'a> {
    pub name: &'a str,
    pub text: Option<&'a str>,
    pub control: bool,
    pub command: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Key<'_> {
    pub fn command(&self) -> Option<Command> {
        let primary = self.control || self.command;
        if primary && !self.alt {
            match self.name {
                "a" if !self.shift => return Some(Command::SelectAll),
                "c" => return Some(Command::Copy),
                "x" => return Some(Command::Cut),
                "v" => return Some(Command::Paste),
                "u" if !self.shift => return Some(Command::Clear),
                _ => {}
            }
        }
        if self.name == "space" && !primary && !self.alt {
            return Some(Command::Insert(" ".into()));
        }
        let word = self.control || self.alt;
        let edit = match self.name {
            "enter" | "escape" | "tab" | "up" | "down" | "pageup" | "pagedown" => return None,
            "left" if self.command && !word => Edit::Start,
            "right" if self.command && !word => Edit::End,
            "left" if word => Edit::WordLeft,
            "right" if word => Edit::WordRight,
            "left" => Edit::Left,
            "right" => Edit::Right,
            "home" if !self.alt => Edit::Start,
            "end" if !self.alt => Edit::End,
            "backspace" if word || self.command => Edit::BackspaceWord,
            "backspace" => Edit::Backspace,
            "delete" if word || self.command => Edit::DeleteWord,
            "delete" => Edit::Delete,
            "w" | "h" if self.control && !self.alt && !self.shift => Edit::BackspaceWord,
            _ => {
                return self
                    .text
                    .or_else(|| (self.name.chars().count() == 1).then_some(self.name))
                    .filter(|s| !primary && !self.alt && !s.chars().any(char::is_control))
                    .map(|s| Command::Insert(s.into()));
            }
        };
        Some(Command::Edit {
            edit,
            extend: self.shift,
        })
    }
}

/// A field's byte caret and selection anchor. `None` starts at the end of a newly opened field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub caret: Option<usize>,
    pub anchor: Option<usize>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub changed: bool,
    pub copy: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub text: String,
    pub cursor: Cursor,
}

impl Cursor {
    pub fn at(caret: Option<usize>, selected: bool, text: &str) -> Self {
        if selected {
            Self {
                caret: Some(text.len()),
                anchor: Some(0),
            }
        } else {
            Self {
                caret,
                anchor: None,
            }
        }
    }
    pub fn caret(&self, text: &str) -> usize {
        line_edit::clamp(text, self.caret.unwrap_or(text.len()))
    }
    pub fn selection(&self, text: &str) -> Option<Range<usize>> {
        let caret = self.caret(text);
        let anchor = line_edit::clamp(text, self.anchor?);
        (anchor != caret).then_some(anchor.min(caret)..anchor.max(caret))
    }
    pub fn selected<'a>(&self, text: &'a str) -> Option<&'a str> {
        text.get(self.selection(text)?)
    }
    pub fn all_selected(&self, text: &str) -> bool {
        !text.is_empty() && self.selection(text) == Some(0..text.len())
    }
    pub fn visible<'a>(&self, text: &'a str, columns: usize) -> line_edit::Visible<'a> {
        line_edit::visible(text, self.caret(text), columns)
    }
    /// An owned paint snapshot, bounded by the viewport rather than the full query.
    pub fn snapshot(&self, text: &str, columns: usize) -> Snapshot {
        let shown = self.visible(text, columns);
        let start = self.caret(text).saturating_sub(shown.prefix.len());
        Snapshot {
            text: shown.text.into(),
            cursor: Cursor {
                caret: Some(shown.prefix.len()),
                anchor: self.anchor.map(|a| {
                    line_edit::clamp(text, a)
                        .saturating_sub(start)
                        .min(shown.text.len())
                }),
            },
        }
    }
    /// Apply one field command. Paste transport is resolved by the adapter into Insert, without reading host clipboard here.
    pub fn execute(&mut self, text: &mut String, command: Command) -> Outcome {
        let before = self.caret(text);
        let selection = self.selection(text);
        let mut outcome = Outcome::default();
        match command {
            Command::SelectAll => {
                self.anchor = Some(0);
                self.caret = Some(text.len());
            }
            Command::Copy => outcome.copy = self.selected(text).map(str::to_owned),
            Command::Cut => {
                outcome.copy = self.selected(text).map(str::to_owned);
                if let Some(range) = selection {
                    self.replace(text, range, "");
                    outcome.changed = true;
                }
            }
            Command::Insert(value) => {
                let value = crate::display::one_line(&value);
                if !value.is_empty() {
                    let range = selection.unwrap_or(before..before);
                    outcome.changed = text.get(range.clone()) != Some(value.as_str());
                    self.replace(text, range, &value);
                }
            }
            Command::Clear => {
                outcome.changed = !text.is_empty();
                text.clear();
                self.caret = Some(0);
                self.anchor = None;
            }
            Command::Edit { edit, extend } => {
                let motion = matches!(
                    edit,
                    Edit::Left
                        | Edit::Right
                        | Edit::WordLeft
                        | Edit::WordRight
                        | Edit::Start
                        | Edit::End
                );
                if motion {
                    let mut at = before;
                    if !extend && let Some(range) = selection {
                        at = match edit {
                            Edit::Left | Edit::WordLeft => range.start,
                            Edit::Right | Edit::WordRight => range.end,
                            _ => {
                                line_edit::apply(text, &mut at, edit);
                                at
                            }
                        };
                    } else {
                        line_edit::apply(text, &mut at, edit);
                    }
                    self.anchor = if extend {
                        Some(self.anchor.unwrap_or(before))
                    } else {
                        None
                    };
                    self.caret = Some(at);
                } else if let Some(range) = selection {
                    self.replace(text, range, "");
                    outcome.changed = true;
                } else {
                    let mut at = before;
                    outcome.changed = line_edit::apply(text, &mut at, edit);
                    self.caret = Some(at);
                    self.anchor = None;
                }
            }
            Command::Paste => {}
        }
        outcome
    }
    pub fn execute_limited(
        &mut self,
        text: &mut String,
        mut command: Command,
        maximum: usize,
    ) -> Outcome {
        if let Command::Insert(value) = &mut command {
            let retained = text
                .len()
                .saturating_sub(self.selection(text).map_or(0, |r| r.len()));
            let end = value.floor_char_boundary(maximum.saturating_sub(retained).min(value.len()));
            value.truncate(end);
        }
        self.execute(text, command)
    }
    fn replace(&mut self, text: &mut String, range: Range<usize>, value: &str) {
        let at = range.start;
        text.replace_range(range, value);
        self.caret = Some(at + value.len());
        self.anchor = None;
    }
}

/// Two fields of the active popup. Switching fields retains both carets; changing popup resets them.
#[derive(Default)]
pub struct Fields {
    key: Option<u64>,
    cursors: [Option<Cursor>; 2],
}
impl Fields {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn cursor(&self, key: u64, field: usize) -> Option<Cursor> {
        (self.key == Some(key))
            .then(|| self.cursors.get(field).copied().flatten())
            .flatten()
    }
    pub fn edit(
        &mut self,
        key: u64,
        field: usize,
        initial: Cursor,
        text: &mut String,
        command: Command,
    ) -> Outcome {
        if self.key != Some(key) {
            self.clear();
            self.key = Some(key);
        }
        let Some(slot) = self.cursors.get_mut(field) else {
            return Outcome::default();
        };
        slot.get_or_insert(initial).execute(text, command)
    }
    pub fn edit_limited(
        &mut self,
        key: u64,
        field: usize,
        initial: Cursor,
        text: &mut String,
        command: Command,
        maximum: usize,
    ) -> Outcome {
        if self.key != Some(key) {
            self.clear();
            self.key = Some(key);
        }
        let Some(slot) = self.cursors.get_mut(field) else {
            return Outcome::default();
        };
        slot.get_or_insert(initial)
            .execute_limited(text, command, maximum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn motion(edit: Edit, extend: bool) -> Command {
        Command::Edit { edit, extend }
    }
    #[test]
    fn selections_copy_cut_paste_and_motions_preserve_unicode_and_untouched_text() {
        let mut text = "first é👩🏽‍💻 last".to_string();
        let mut cursor = Cursor::default();
        cursor.execute(&mut text, motion(Edit::WordLeft, false));
        cursor.execute(&mut text, motion(Edit::WordLeft, true));
        assert_eq!(
            cursor.execute(&mut text, Command::Copy).copy.as_deref(),
            Some("👩🏽‍💻 ")
        );
        assert_eq!(
            cursor.execute(&mut text, Command::Cut).copy.as_deref(),
            Some("👩🏽‍💻 ")
        );
        assert_eq!(text, "first élast");
        cursor.execute(
            &mut text,
            Command::Insert("other\nignored".into()).first_line(),
        );
        assert_eq!(text, "first éotherlast");
        cursor.execute(&mut text, Command::SelectAll);
        cursor.execute(&mut text, motion(Edit::Left, false));
        assert_eq!(cursor.caret(&text), 0);
        assert!(cursor.selection(&text).is_none());
        cursor.execute(&mut text, motion(Edit::End, true));
        cursor.execute(&mut text, motion(Edit::BackspaceWord, false));
        assert!(text.is_empty());
        assert_eq!(cursor.caret(&text), 0);
    }
    #[test]
    fn range_selection_extends_by_whole_graphemes_and_survives_field_switches() {
        let (mut text, mut fields) = ("aé👩🏽‍💻b".into(), Fields::default());
        fields.edit(
            1,
            0,
            Cursor::default(),
            &mut text,
            motion(Edit::Left, false),
        );
        fields.edit(1, 0, Cursor::default(), &mut text, motion(Edit::Left, true));
        assert_eq!(fields.cursor(1, 0).unwrap().selected(&text), Some("👩🏽‍💻"));
        fields.edit(
            1,
            1,
            Cursor::default(),
            &mut "second".into(),
            Command::SelectAll,
        );
        assert_eq!(fields.cursor(1, 0).unwrap().selected(&text), Some("👩🏽‍💻"));
        fields.edit(2, 0, Cursor::default(), &mut text, Command::Copy);
        assert!(fields.cursor(2, 0).unwrap().selection(&text).is_none());
    }
    #[test]
    fn normalized_keys_cover_editor_motions_and_leave_popup_actions_to_adapters() {
        let cmd = Key {
            name: "left",
            command: true,
            shift: true,
            ..Default::default()
        }
        .command();
        assert_eq!(cmd, Some(motion(Edit::Start, true)));
        assert_eq!(
            Key {
                name: "right",
                control: true,
                shift: true,
                ..Default::default()
            }
            .command(),
            Some(motion(Edit::WordRight, true))
        );
        assert_eq!(
            Key {
                name: "a",
                control: true,
                ..Default::default()
            }
            .command(),
            Some(Command::SelectAll)
        );
        assert!(
            Key {
                name: "a",
                control: true,
                shift: true,
                ..Default::default()
            }
            .command()
            .is_none()
        );
        assert_eq!(
            Key {
                name: "space",
                ..Default::default()
            }
            .command(),
            Some(Command::Insert(" ".into()))
        );
        assert_eq!(
            Key {
                name: "λ",
                ..Default::default()
            }
            .command(),
            Some(Command::Insert("λ".into()))
        );
        for name in ["up", "down", "pageup", "pagedown", "enter", "escape", "tab"] {
            assert!(
                Key {
                    name,
                    ..Default::default()
                }
                .command()
                .is_none()
            );
        }
    }
    #[test]
    fn bounded_paste_replaces_selection_and_stale_cursors_and_views_are_utf8_safe() {
        let mut text = "é one".to_string();
        let mut cursor = Cursor::default();
        cursor.execute(&mut text, Command::SelectAll);
        cursor.execute_limited(&mut text, Command::Insert("👩🏽‍💻 tail".into()), 15);
        assert_eq!(text, "👩🏽‍💻");
        cursor.execute(&mut text, Command::SelectAll);
        cursor.execute(&mut text, Command::Paste);
        assert_eq!(cursor.selected(&text), Some("👩🏽‍💻"));
        text = "é".into();
        for columns in [0, 1, 2, 80] {
            let snapshot = cursor.snapshot(&text, columns);
            assert!(snapshot.cursor.caret(&snapshot.text) <= snapshot.text.len());
        }
        cursor.execute(&mut text, Command::SelectAll);
        cursor.execute(&mut text, Command::Insert("x\nnever inserted".into()));
        assert_eq!(text, "x never inserted");
    }
}
