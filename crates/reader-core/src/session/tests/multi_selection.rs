//! Several selections in one view: how they are created, kept disjoint, edited and undone.
use super::*;
use crate::text::EditKind;

fn carets(s: &Session) -> Vec<usize> {
    let mut heads: Vec<_> = s.state().active().unwrap().selections().filter(|s| s.anchor == s.head).map(|s| s.head).collect();
    heads.sort_unstable();
    heads
}

fn ranges(s: &Session) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<_> = s.state().active().unwrap().selections().map(|selection| selection.range()).collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    ranges
}

#[test]
fn adding_cursors_below_and_above_keeps_the_column_and_stops_at_the_ends() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "carets.txt", b"alpha\nbeta gamma\ndelta\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "carets.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    assert_eq!(s.disabled(CommandId::AddCursorBelow), None);
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [3, 9], "the next line's column 3");
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [3, 9, 20]);
    assert_eq!(s.state().selection(), Some(Selection { anchor: 20, head: 20 }), "the newest caret is the primary one");
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [3, 9, 20, 23], "the last, empty line takes the caret clamped to its start");
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [3, 9, 20, 23], "there is no line below the last");
    assert_eq!(s.disabled(CommandId::AddCursorBelow), Some("The caret is on the last line"));
    assert_eq!(s.disabled(CommandId::AddCursorAbove), None);
    s.dispatch(Command::AddCursorAbove);
    assert_eq!(carets(&s), [3, 9, 17, 20, 23], "above keeps the column, here line 2's start");
    // A selection keeps its anchor: the new cursor is a caret on the next line at the head's column.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(ranges(&s), [0..5, 11..11]);
    assert_eq!(carets(&s), [11]);
}

fn three_carets(s: &mut Session, path: &std::path::Path) {
    open(s, Command::OpenRetained(path.to_path_buf()), "carets.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    s.dispatch(Command::AddCursorBelow);
    s.dispatch(Command::AddCursorBelow);
}

#[test]
fn typing_at_every_selection_is_one_undoable_transaction_per_typing_group() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "carets.txt", b"one\ntwo\nthree\n");
    let mut s = session(d.path());
    three_carets(&mut s, &path);
    assert_eq!(carets(&s), [1, 5, 9]);
    for text in ["a", "b", "c"] {
        s.dispatch(Command::InsertText { text: text.into(), kind: EditKind::Typing });
    }
    assert_eq!(source_text(s.state().current_source().unwrap()), "oabcne\ntabcwo\ntabchree\n");
    assert_eq!(carets(&s), [4, 11, 18], "every caret follows its own insertion");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "one\ntwo\nthree\n", "one undo takes the whole typing group");
    assert_eq!(carets(&s), [1, 5, 9], "and every caret returns to where it was");
    assert!(!active_view(&s).can_undo(), "no earlier typing step is left behind");
    s.dispatch(Command::Redo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "oabcne\ntabcwo\ntabchree\n");
    assert_eq!(carets(&s), [4, 11, 18]);
    assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\nthree\n", "typing never writes the file");
}

#[test]
fn multiline_unicode_text_replaces_every_selection_and_undoes_once() {
    let d = tempfile::tempdir().unwrap();
    // Two selections of the same text, replaced at once by text with a non-BMP character and a newline.
    let path = write(d.path(), "unicode.txt", "aa λ 😀 aa\n".as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "unicode.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 0, head: 2 }));
    s.dispatch(Command::SelectNextOccurrence);
    assert_eq!(ranges(&s), [0..2, 11..13]);
    s.dispatch(Command::InsertText { text: "👋\nλ".into(), kind: EditKind::Paste });
    assert_eq!(source_text(s.state().current_source().unwrap()), "👋\nλ λ 😀 👋\nλ\n");
    assert_eq!(carets(&s), [7, 23], "each caret sits after its own replacement");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "aa λ 😀 aa\n", "both selections come back untouched");
    assert_eq!(carets(&s), [2, 13], "each caret rests after the text it restored");
    s.dispatch(Command::Redo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "👋\nλ λ 😀 👋\nλ\n");
}

#[test]
fn selections_in_different_contexts_change_together_and_undo_one_step_each() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "context.txt", b"    let a = 1;\n    let bb = 22;\n  let longer = 333;\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "context.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 12, head: 12 }));
    s.dispatch(Command::AddCursorBelow);
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [12, 27, 44], "the same column on three differently indented lines");
    s.dispatch(Command::InsertText { text: "X".into(), kind: EditKind::Typing });
    assert_eq!(source_text(s.state().current_source().unwrap()), "    let a = X1;\n    let bb =X 22;\n  let longerX = 333;\n");
    s.dispatch(Command::InsertText { text: "\n".into(), kind: EditKind::Typing });
    assert_eq!(source_text(s.state().current_source().unwrap()), "    let a = X\n1;\n    let bb =X\n 22;\n  let longerX\n = 333;\n");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "    let a = 1;\n    let bb = 22;\n  let longer = 333;\n", "one undo takes the whole typing group at every caret");
    assert!(!active_view(&s).can_undo());
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "    let a = 1;\n    let bb = 22;\n  let longer = 333;\n", "there is nothing earlier to undo");
}

#[test]
fn deleting_at_several_carets_removes_one_character_each_and_undoes_together() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "carets.txt", b"alpha\r\nbeta\r\ngamma\r\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "carets.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 2, head: 2 }));
    s.dispatch(Command::AddCursorBelow);
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [2, 9, 15]);
    s.dispatch(Command::Delete { backward: false });
    assert_eq!(source_text(s.state().current_source().unwrap()), "alha\r\nbea\r\ngama\r\n");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "alpha\r\nbeta\r\ngamma\r\n");
    // A caret at each line's end removes its CRLF as one character, as a single caret does.
    s.dispatch(Command::Select(Selection { anchor: 5, head: 5 }));
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [5, 11]);
    s.dispatch(Command::Delete { backward: false });
    assert_eq!(source_text(s.state().current_source().unwrap()), "alphabetagamma\r\n", "each CRLF pair goes as one character");
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "alpha\r\nbeta\r\ngamma\r\n");
    assert_eq!(std::fs::read(path).unwrap(), b"alpha\r\nbeta\r\ngamma\r\n");
}

#[test]
fn cut_and_copy_take_every_selection_and_paste_fills_them() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "carets.txt", b"ab\nab\ncd\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "carets.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 0, head: 2 }));
    s.dispatch(Command::SelectNextOccurrence);
    assert_eq!(ranges(&s), [0..2, 3..5]);
    let effect = s.dispatch(Command::Copy);
    assert_eq!(effect.copy.as_deref(), Some("ab\nab"), "each selection's text, one per line");
    let effect = s.dispatch(Command::Cut);
    assert_eq!(effect.copy.as_deref(), Some("ab\nab"));
    assert_eq!(source_text(s.state().current_source().unwrap()), "\n\ncd\n");
    assert_eq!(carets(&s), [0, 1], "each caret stays where its selection was");
    s.dispatch(Command::Paste("λ".into()));
    assert_eq!(source_text(s.state().current_source().unwrap()), "λ\nλ\ncd\n", "the text goes at every selection");
    assert_eq!(carets(&s), [2, 5]);
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "\n\ncd\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "ab\nab\ncd\n");
}

#[test]
fn select_next_occurrence_adds_disjoint_matches_and_stops_when_none_is_left() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "occurrences.txt", b"aaa aa aaa\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "occurrences.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    assert_eq!(s.disabled(CommandId::SelectNextOccurrence), None);
    s.dispatch(Command::SelectNextOccurrence);
    assert_eq!(ranges(&s), vec![0..3], "a caret first selects the word under it");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 2 }));
    s.dispatch(Command::SelectNextOccurrence);
    assert_eq!(ranges(&s), [0..2, 4..6], "the match overlapping the selection at 1..3 is skipped");
    s.dispatch(Command::SelectNextOccurrence);
    assert_eq!(ranges(&s), [0..2, 4..6, 7..9]);
    let before = ranges(&s);
    s.dispatch(Command::SelectNextOccurrence);
    assert_eq!(ranges(&s), before, "every occurrence is already selected");
    assert!(s.state.messages.iter().any(|m| m == "No further occurrence of “aa”"));
    assert!(s.disabled(CommandId::CollapseSelections).is_none());
    s.dispatch(Command::CollapseSelections);
    assert_eq!(ranges(&s), vec![7..9], "the primary selection is the one kept");
    assert_eq!(s.disabled(CommandId::CollapseSelections), Some("One selection is active"));
}

#[test]
fn clicking_or_moving_the_caret_leaves_one_selection() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "carets.txt", b"alpha\nbeta\ngamma\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "carets.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    s.dispatch(Command::AddCursorBelow);
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [1, 7, 12]);
    s.dispatch(Command::MoveCaret(Motion::Right));
    assert_eq!(carets(&s), [13], "an ordinary move goes back to one caret, from the primary one");
    s.dispatch(Command::AddCursorBelow);
    s.dispatch(Command::ClickSelect { at: 3, clicks: 1 });
    assert_eq!(carets(&s), [3], "a click places one caret");
    s.dispatch(Command::AddCursorBelow);
    s.dispatch(Command::SelectAll);
    assert_eq!(ranges(&s), vec![0..17], "Select All is still one selection");
}

#[test]
fn the_multi_cursor_chords_come_from_the_keymap_preset() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "carets.txt", b"alpha\nbeta\ngamma\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "carets.txt");
    s.enable_text_input_for_test();
    let press = |s: &Session, text: &str| {
        let chord = crate::keymap::Chord::parse(text).unwrap();
        s.key_command(&crate::keymap::KeyPress::new(&chord.key, chord.ctrl, chord.shift, chord.alt).unwrap())
    };
    // The IntelliJ preset the adapters start with runs the commands and names them in the menus.
    assert_eq!(press(&s, "Alt+Shift+Down"), Some(CommandId::AddCursorBelow));
    assert_eq!(press(&s, "Alt+Shift+Up"), Some(CommandId::AddCursorAbove));
    assert_eq!(press(&s, "Alt+J"), Some(CommandId::SelectNextOccurrence));
    assert_eq!(s.key_hint(CommandId::AddCursorBelow).as_deref(), Some("Alt+Shift+Down"));
    assert_eq!(s.key_hint(CommandId::CollapseSelections).as_deref(), Some("Esc"), "Escape is a fixed editing key");
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    let Some(CommandId::AddCursorBelow) = press(&s, "Alt+Shift+Down") else { panic!("the keymap chord resolves") };
    let crate::actions::CatalogAction::Command(add) = s.action(CommandId::AddCursorBelow) else { panic!("a command") };
    s.dispatch(add);
    assert_eq!(carets(&s), [1, 7], "the resolved chord adds the cursor the menu names");
    // The VS Code preset keeps both adapters' arrows and adds VS Code's own; the menu hint follows the preset.
    s.dispatch(Command::SetKeymapPreset(crate::keymap::KeymapPreset::VsCode));
    assert_eq!(s.key_hint(CommandId::AddCursorBelow).as_deref(), Some("Ctrl+Alt+Down"));
    assert_eq!(press(&s, "Ctrl+Alt+Down"), Some(CommandId::AddCursorBelow));
    assert_eq!(press(&s, "Alt+Shift+Down"), Some(CommandId::AddCursorBelow), "the arrows both adapters deliver still work");
    assert_eq!(press(&s, "Alt+J"), Some(CommandId::SelectNextOccurrence));
    // A rebinding moves the command and its menu line: the preset chord no longer runs it.
    s.dispatch(Command::SetKeybinding { command: CommandId::AddCursorBelow, chord: "Ctrl+Alt+J".into() });
    assert_eq!(press(&s, "Alt+Shift+Down"), None);
    assert_eq!(press(&s, "Ctrl+Alt+J"), Some(CommandId::AddCursorBelow));
    assert_eq!(s.key_hint(CommandId::AddCursorBelow).as_deref(), Some("Ctrl+Alt+J"));
}

#[test]
fn adding_cursors_above_the_first_line_is_unavailable() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "top.txt", b"one\ntwo\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "top.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    assert_eq!(s.disabled(CommandId::AddCursorAbove), Some("The caret is on the first line"));
    s.dispatch(Command::AddCursorAbove);
    assert_eq!(carets(&s), [1]);
    assert_eq!(s.disabled(CommandId::AddCursorBelow), None);
}

#[test]
fn multiple_cursors_follow_read_only_state_and_explain_it() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "readonly.txt", b"alpha\nbeta\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "readonly.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    s.dispatch(Command::SetReadOnly(true));
    for id in [CommandId::AddCursorBelow, CommandId::AddCursorAbove, CommandId::SelectNextOccurrence, CommandId::CollapseSelections] {
        assert!(s.disabled(id).is_some(), "{id:?} is unavailable in read-only mode");
    }
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [1]);
    s.dispatch(Command::InsertText { text: "x".into(), kind: EditKind::Typing });
    assert_eq!(source_text(s.state().current_source().unwrap()), "alpha\nbeta\n");
    s.dispatch(Command::SetReadOnly(false));
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(carets(&s), [1, 7], "editing is available again with both cursors intact");
    s.dispatch(Command::InsertText { text: "x".into(), kind: EditKind::Typing });
    assert_eq!(source_text(s.state().current_source().unwrap()), "axlpha\nbxeta\n");
    assert_eq!(carets(&s), [2, 9], "each caret follows its own insertion");
    assert_eq!(std::fs::read(path).unwrap(), b"alpha\nbeta\n", "the draft is never written by typing");
}

#[test]
fn an_empty_session_explains_why_selections_cannot_be_added() {
    let d = tempfile::tempdir().unwrap();
    let s = session(d.path());
    for id in [CommandId::AddCursorBelow, CommandId::AddCursorAbove, CommandId::SelectNextOccurrence, CommandId::CollapseSelections] {
        assert_eq!(s.disabled(id), Some("No loaded file is open"), "{id:?}");
    }
}
