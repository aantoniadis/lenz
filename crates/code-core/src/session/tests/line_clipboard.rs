use super::*;

fn ready(dir: &Path, text: &str) -> Session {
    let path = write(dir, "lines.txt", text.as_bytes());
    let mut s = session(dir);
    open(&mut s, Command::OpenRetained(path), "lines.txt");
    s.enable_text_input_for_test();
    s
}

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }
fn caret(s: &mut Session, at: usize) { s.dispatch(Command::Select(Selection { anchor: at, head: at })); }

#[test]
fn a_caret_cuts_and_copies_its_whole_line_and_pastes_it_back_as_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(dir.path(), "one\r\ntwo\r\nthree");
    assert_eq!(s.disabled(CommandId::Cut), None, "a caret alone can cut");
    caret(&mut s, 7); // in "two"
    let cut = s.dispatch(Command::Cut);
    assert_eq!(cut.copy.as_deref(), Some("two\r\n"));
    assert_eq!(text(&s), "one\r\nthree");
    // Pasted with a caret in the middle of "three": the line goes in above it, the caret stays in its text.
    caret(&mut s, 7);
    s.dispatch(Command::Paste("two\r\n".into()));
    assert_eq!(text(&s), "one\r\ntwo\r\nthree");
    assert_eq!(s.state().selection().map(|sel| sel.head), Some(12), "still after \"th\" in three");
    // Undo takes back the paste, then the cut: each one step.
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "one\r\ntwo\r\nthree");
    // The last line has no ending: it takes the line break before it, and pastes back with the file's ending.
    let end = text(&s).len();
    caret(&mut s, end);
    assert_eq!(s.dispatch(Command::Cut).copy.as_deref(), Some("three"));
    assert_eq!(text(&s), "one\r\ntwo");
    caret(&mut s, 0);
    s.dispatch(Command::Paste("three".into()));
    assert_eq!(text(&s), "three\r\none\r\ntwo");
    // Text copied from a selection pastes where the caret is, as always.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 2 }));
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("th"));
    caret(&mut s, 7);
    s.dispatch(Command::Paste("th".into()));
    assert_eq!(text(&s), "three\r\nthone\r\ntwo");
}

#[test]
fn several_carets_cut_and_copy_each_of_their_lines_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(dir.path(), "a\nb\nc\nd\n");
    caret(&mut s, 0);
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("a\nb\n"));
    let cut = s.dispatch(Command::Cut);
    assert_eq!(cut.copy.as_deref(), Some("a\nb\n"));
    assert_eq!(text(&s), "c\nd\n");
    caret(&mut s, 2);
    s.dispatch(Command::Paste("a\nb\n".into()));
    assert_eq!(text(&s), "c\na\nb\nd\n");
}

#[test]
fn a_terminal_paste_of_copied_lines_pastes_them_as_lines_whatever_its_line_breaks() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ready(dir.path(), "one\ntwo\n");
    caret(&mut s, 0);
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("one\n"));
    caret(&mut s, 5);
    // Terminals send a pasted line break as CR.
    s.dispatch(Command::InsertText { text: "one\r".into(), kind: crate::text::EditKind::Paste });
    assert_eq!(text(&s), "one\none\ntwo\n");
}
