//! Snippets: inserting a body as one transaction, Tab between its placeholders, and ending it safely.
//!
//! Every action runs through the shared Session command seam; these tests assert the exact text, the selections,
//! the live snippet state, the undo history and the untouched file.
use super::*;

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }

fn ranges(s: &Session) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<_> = s.state().active().unwrap().selections().map(|selection| selection.range()).collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    ranges
}

fn selected(s: &Session) -> Vec<String> {
    let doc = s.state().current_source().unwrap();
    ranges(s).into_iter().map(|range| doc.copy_range(range).unwrap_or_default()).collect()
}

fn hint(s: &Session) -> Option<String> { s.snippet_hint() }

/// Insert `body` where the caret is, through the command seam.
fn insert(s: &mut Session, body: &str) {
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    let at = s.state().selection().map_or(0, |selection| selection.range().start);
    s.dispatch(Command::InsertSnippet { tab, expected, range: at..at, body: body.into() });
}

fn tab(s: &mut Session, outdent: bool) { s.dispatch(Command::Indent { outdent }); }

fn undo(s: &mut Session) {
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
}

fn ready(d: &Path, name: &str, content: &str) -> (Session, PathBuf) {
    let dir = d.join("state");
    std::fs::create_dir_all(&dir).unwrap();
    let path = write(d, name, content.as_bytes());
    let mut s = session(&dir);
    open(&mut s, Command::OpenRetained(path.clone()), name);
    s.enable_text_input_for_test();
    (s, path)
}

#[test]
fn inserting_a_snippet_selects_its_first_placeholder_and_one_undo_removes_it() {
    let d = tempfile::tempdir().unwrap();
    let original = "fn main() {\n}\n";
    let (mut s, path) = ready(d.path(), "main.rs", original);
    s.dispatch(Command::Select(Selection { anchor: 14, head: 14 }));
    insert(&mut s, "fn ${2:name}(${1:x}) { $0 }");
    assert_eq!(text(&s), "fn main() {\n}\nfn name(x) {  }", "the body is inserted at the caret");
    assert_eq!(selected(&s), ["x"], "the first placeholder is selected");
    assert_eq!(hint(&s).as_deref(), Some("Snippet: placeholder 1 of 3 · Tab next · Esc done"));
    // Tab visits the numbered placeholders in order, then the final caret, and then ends the snippet.
    tab(&mut s, false);
    assert_eq!(selected(&s), ["name"]);
    assert_eq!(hint(&s).as_deref(), Some("Snippet: placeholder 2 of 3 · Tab next · Esc done"));
    tab(&mut s, false);
    assert_eq!(ranges(&s), vec![27..27], "the final caret, at $0");
    assert_eq!(hint(&s).as_deref(), Some("Snippet: placeholder 3 of 3 · Tab ends · Esc done"));
    tab(&mut s, false);
    assert_eq!(text(&s), "fn main() {\n}\nfn name(x) {  }", "the last Tab only ends the snippet");
    assert_eq!(hint(&s), None);
    // Tab indents again now (ticket 27's rule: the lines a selection touches, or the step at a caret).
    s.dispatch(Command::Select(Selection { anchor: 14, head: 14 }));
    tab(&mut s, false);
    assert_eq!(text(&s), "fn main() {\n}\n    fn name(x) {  }", "outside a snippet Tab indents the line by the file's step");
    undo(&mut s);
    undo(&mut s);
    assert_eq!(text(&s), original, "one Undo removes the whole insertion");
    assert_eq!(hint(&s), None);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "no file is written");
}

/// A body whose only stop is its final caret (a CSS property's `color: $0;`, a closing tag's `$0</div>`) leaves the
/// caret there, not after the inserted text, with nothing left to visit.
#[test]
fn a_body_with_only_a_final_caret_leaves_the_caret_there() {
    let d = tempfile::tempdir().unwrap();
    let original = ".a { }
";
    let (mut s, path) = ready(d.path(), "a.css", original);
    s.dispatch(Command::Select(Selection { anchor: 5, head: 5 }));
    insert(&mut s, "color: $0;");
    assert_eq!(text(&s), ".a { color: ;}
");
    assert_eq!(ranges(&s), vec![12..12], "the caret is where the value goes");
    assert_eq!(hint(&s), None, "nothing to visit");
    undo(&mut s);
    assert_eq!(text(&s), original);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "no file is written");
}

#[test]
fn typing_in_the_placeholder_stays_in_it_and_a_repeated_placeholder_is_typed_at_once() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, _path) = ready(d.path(), "sum.rs", "");
    insert(&mut s, "${1:sum} + $1 = $0");
    assert_eq!(text(&s), "sum + sum = ");
    assert_eq!(ranges(&s), vec![0..3, 6..9], "a stop selects every occurrence of a repeated placeholder");
    s.dispatch(Command::InsertText { text: "t".into(), kind: crate::text::EditKind::Typing });
    assert_eq!(text(&s), "t + t = ", "one typing transaction changes every occurrence");
    assert_eq!(ranges(&s), vec![1..1, 5..5], "the carets sit after each typed letter, and the stop with them");
    tab(&mut s, false);
    assert_eq!(ranges(&s), vec![8..8], "the final caret");
    undo(&mut s);
    assert_eq!(text(&s), "sum + sum = ", "an Undo takes back the typing step only");
    assert_eq!(hint(&s), None, "and never leaves placeholder positions behind");
    undo(&mut s);
    assert_eq!(text(&s), "", "the whole insertion is one history step");
}

#[test]
fn inserted_lines_take_the_project_step_and_the_lines_own_ending() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".editorconfig"), "root = true\n[*]\nindent_style = space\nindent_size = 2\n").unwrap();
    let path = write(&root, "main.rs", b"fn main() {\r\n}\r\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(path.clone()), "main.rs");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 13, head: 13 }));
    insert(&mut s, "if (x) {\r\n\tbody();\r\n\tinner();\r\n}$0");
    assert_eq!(text(&s), "fn main() {\r\nif (x) {\r\n  body();\r\n  inner();\r\n}}\r\n",
        "the body's tabs become the project's two spaces and the file's CRLF stays");
    assert_eq!(ranges(&s), vec![47..47], "the final caret");
}

#[test]
fn escape_and_a_tab_change_end_the_snippet_and_tab_indents_again() {
    let d = tempfile::tempdir().unwrap();
    let (mut s, _path) = ready(d.path(), "esc.rs", "");
    insert(&mut s, "call(${1:a})");
    assert_eq!(ranges(&s), vec![5..6]);
    s.dispatch(Command::CollapseSelections);
    assert_eq!(ranges(&s), vec![5..6], "Escape keeps the caret where it is here");
    assert_eq!(hint(&s), None, "Escape ends the snippet");
    tab(&mut s, false);
    assert_eq!(text(&s), "    call(a)", "Tab indents the line instead of visiting a placeholder");
    // Moving the caret ends it: the same rule, so Tab indents where the caret is now.
    insert(&mut s, "call(${1:a})");
    assert_eq!(hint(&s).as_deref().map(|h| h.starts_with("Snippet: placeholder 1 of 2")), Some(true));
    s.dispatch(Command::MoveCaret(Motion::Right));
    assert_eq!(hint(&s), None, "a caret move ends the snippet");
    // A tab change ends it too: the snippet belongs to the view it was inserted in.
    let first = s.state().active().unwrap().id();
    let other = write(d.path(), "other.rs", b"other\n");
    open(&mut s, Command::OpenRetained(other.clone()), "other.rs");
    insert(&mut s, "x(${1:y})");
    assert!(hint(&s).is_some_and(|h| h.contains("placeholder 1 of 2")));
    s.dispatch(Command::ActivateTab(first));
    assert_eq!(hint(&s), None, "another tab has no live snippet");
    let second = s.state().tabs().iter().find(|t| t.source().ends_with("other.rs")).unwrap().id();
    s.dispatch(Command::ActivateTab(second));
    assert_eq!(hint(&s), None, "and returning to its tab does not revive it");
}

#[test]
fn a_refused_snippet_changes_nothing_and_says_why() {
    let d = tempfile::tempdir().unwrap();
    let original = "abc\n";
    let (mut s, path) = ready(d.path(), "refuse.rs", original);
    s.dispatch(Command::Select(Selection { anchor: 1, head: 2 }));
    insert(&mut s, "${1:outer ${2:inner}}");
    assert_eq!(text(&s), original, "a body this editor cannot expand inserts nothing at all");
    assert!(s.state().messages.iter().any(|m| m.contains("cannot contain another placeholder")), "{:?}", s.state().messages);
    // Several carets: two snippets could not share one set of placeholders, so it refuses rather than half-apply.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::AddCursorBelow);
    insert(&mut s, "fn $1");
    assert_eq!(text(&s), original);
    assert!(s.state().messages.iter().any(|m| m.contains("one caret")), "{:?}", s.state().messages);
    assert_eq!(hint(&s), None);
    // Read-only: the same refusal as typing.
    s.dispatch(Command::CollapseSelections);
    s.dispatch(Command::SetReadOnly(true));
    insert(&mut s, "fn $1");
    assert_eq!(text(&s), original);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}
