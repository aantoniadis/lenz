//! Find and replace in the terminal: the prompt's two fields, the keys that replace, and what is drawn.
use super::*;

fn open(dir: &Path, text: &str) -> (App, std::path::PathBuf) {
    let path = dir.join("a.txt");
    std::fs::write(&path, text).unwrap();
    let mut a = App::with_text_input(config(dir), Some(path.clone()), true);
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 24);
    (a, path)
}

fn text(a: &App) -> String {
    let source = a.session.state().current_source().unwrap();
    source.copy_range(0..source.len_bytes()).unwrap()
}

fn settled(a: &App) -> bool {
    a.session.state().active().and_then(|t| t.find()).is_some_and(|f| !f.pending())
}

fn type_text(a: &mut App, text: &str) {
    for c in text.chars() {
        key(a, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

fn find_prompt(a: &mut App, query: &str) {
    key(a, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert!(a.modal.as_ref().is_some_and(|m| m.kind == Prompt::Find), "Ctrl+F opens the find prompt");
    type_text(a, query);
    settle(a, settled);
}

#[test]
fn the_find_prompt_replaces_the_current_match_and_every_listed_match() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, path) = open(dir.path(), "Foo foo\nfood foo\n");
    find_prompt(&mut a, "foo");
    assert_eq!(a.session.state().selection().map(|s| s.range()), Some(0..3), "the first match is selected");
    // Tab gives the second field the keys; typing there is the replacement, not a new query.
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert!(a.modal.as_ref().is_some_and(|m| m.secondary_focus), "Tab moves to the replacement field");
    type_text(&mut a, "bar");
    assert_eq!(a.session.replace_text(), "bar");
    assert_eq!(a.session.state().active().unwrap().find().unwrap().query(), "foo");
    // Shift-Tab goes back to the query; Tab returns to the replacement, as in the GUI.
    key(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert!(a.modal.as_ref().is_some_and(|m| !m.secondary_focus));
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert!(a.modal.as_ref().is_some_and(|m| m.secondary_focus));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, settled);
    assert_eq!(text(&a), "bar foo\nfood foo\n");
    assert!(a.modal.as_ref().is_some_and(|m| m.kind == Prompt::Find), "the prompt stays open for the next replace");
    assert_eq!(a.session.state().active().unwrap().find().unwrap().current(), Some(0), "the next match is current");
    // Alt+A replaces every remaining match from the same prompt.
    key(&mut a, KeyCode::Char('a'), KeyModifiers::ALT);
    settle(&mut a, settled);
    assert_eq!(text(&a), "bar bar\nbard bar\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"Foo foo\nfood foo\n", "replace never writes the file");
    // Each explicit replace is one undo step once the prompt is closed.
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none() && a.session.state().active().unwrap().find().is_none());
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "bar foo\nfood foo\n", "one Undo takes the whole Replace All");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "Foo foo\nfood foo\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"Foo foo\nfood foo\n");
}

#[test]
fn an_empty_replacement_deletes_the_current_match_and_an_invalid_pattern_replaces_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, path) = open(dir.path(), "one\ntwo\nthree\n");
    find_prompt(&mut a, "two");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    // A match never spans a line break, so the empty replacement leaves that line's ending behind.
    assert_eq!(text(&a), "one\n\nthree\n", "an empty replacement deletes the match");
    assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\nthree\n");

    // A bad expression says so; nothing is replaced and the prompt keeps its text.
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('f'), KeyModifiers::CONTROL);
    type_text(&mut a, "(");
    key(&mut a, KeyCode::Char('r'), KeyModifiers::ALT);
    settle(&mut a, settled);
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    type_text(&mut a, "x");
    let before = text(&a);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(text(&a), before);
    assert!(a.session.state().notices().iter().any(|n| n == "Replace: The search pattern is invalid"), "{:?}", a.session.state().notices());
    draw(&mut a, 80, 24);
    let screen = draw_screen(&mut a, 80, 24);
    assert!(screen.contains("unclosed group"), "the prompt shows why the pattern cannot match");
}

#[test]
fn read_only_mode_refuses_replace_and_the_prompt_shows_the_counter_and_the_replace_field() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _path) = open(dir.path(), "foo foo\n");
    find_prompt(&mut a, "foo");
    draw(&mut a, 80, 24);
    let screen = draw_screen(&mut a, 80, 24);
    assert!(screen.contains("Replace"), "the replacement field is drawn: {screen:?}");
    assert!(screen.contains("1/2"), "the counter follows the match: {screen:?}");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    type_text(&mut a, "bar");
    a.session.dispatch(Command::SetReadOnly(true));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(text(&a), "foo foo\n");
    assert!(a.session.state().notices().iter().any(|n| n == "Replace: Read-only mode is on"), "{:?}", a.session.state().notices());
    // Modal input never reaches the document: Esc closes find, then typing edits the text.
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    a.session.dispatch(Command::SetReadOnly(false));
    type_text(&mut a, "x");
    assert_eq!(text(&a), "x foo\n", "typing replaced the still-selected match in the document");
}
