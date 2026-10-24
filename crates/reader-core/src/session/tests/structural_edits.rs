//! Structural edits: indentation, comments, moving and duplicating lines, and paired delimiters.
//!
//! Each command runs through the shared Session command seam; these tests assert the exact text, the resulting
//! selections, the undo history and the untouched file.
use super::*;
use crate::actions::CatalogAction;
use crate::text::EditKind;

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }

fn ranges(s: &Session) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<_> = s.state().active().unwrap().selections().map(|selection| selection.range()).collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    ranges
}

fn project_file(d: &Path, name: &str, editorconfig: &str, content: &str) -> (PathBuf, PathBuf) {
    let root = d.join("p");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".editorconfig"), editorconfig).unwrap();
    let path = write(&root, name, content.as_bytes());
    (root, path)
}

fn ready(d: &Path, root: &Path, path: &Path) -> Session {
    let mut s = session(d);
    s.dispatch(Command::OpenRoot(root.to_path_buf()));
    settle(&mut s, scanned);
    s.dispatch(Command::OpenRetained(path.to_path_buf()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == path));
    s.enable_text_input_for_test();
    s
}

#[test]
fn indent_and_outdent_follow_the_files_editorconfig_and_undo_once() {
    let d = tempfile::tempdir().unwrap();
    let (root, path) = project_file(d.path(), "main.rs", "root = true\n[*]\nindent_style = space\nindent_size = 2\n", "    let a = 1;\nnext\n");
    let mut s = ready(d.path(), &root, &path);
    // A partial selection indents the line it touches and keeps covering the same text.
    s.dispatch(Command::Select(Selection { anchor: 4, head: 9 }));
    assert_eq!(s.disabled(CommandId::IndentSelection), None);
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "      let a = 1;\nnext\n");
    assert_eq!(ranges(&s), vec![6..11], "the selection follows its text");
    // Outdent removes one width again, never more.
    s.dispatch(Command::Indent { outdent: true });
    assert_eq!(text(&s), "    let a = 1;\nnext\n");
    assert_eq!(ranges(&s), vec![4..9]);
    // One action is one undo step.
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "      let a = 1;\nnext\n");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "    let a = 1;\nnext\n", "Undo reverses the whole command");
    s.dispatch(Command::Redo { tab });
    assert_eq!(text(&s), "      let a = 1;\nnext\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"    let a = 1;\nnext\n", "no file is written");
}

#[test]
fn a_caret_indents_with_the_files_step_and_outdent_removes_one_step_at_most() {
    let d = tempfile::tempdir().unwrap();
    let (root, path) = project_file(d.path(), "go.mod", "root = true\n[*.mod]\nindent_style = tab\ntab_width = 8\n", "one\n  two\n");
    let mut s = ready(d.path(), &root, &path);
    // A caret with no selection inserts the file's step, not a bare tab the file would not use.
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "o\tne\n  two\n");
    assert_eq!(ranges(&s), vec![2..2], "the caret follows the step it inserted");
    // Outdent takes leading whitespace back, never past what is there and never from the middle of a line.
    s.dispatch(Command::Indent { outdent: true });
    assert_eq!(text(&s), "o\tne\n  two\n", "a line that starts with text has no step to remove");
    s.dispatch(Command::Select(Selection { anchor: 7, head: 7 }));
    s.dispatch(Command::Indent { outdent: true });
    assert_eq!(text(&s), "o\tne\ntwo\n", "two leading spaces go with the line's step");
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "o\tne\n\ttwo\n", "indenting that line uses the file's tab");
}

#[test]
fn toggling_a_comment_follows_the_languages_syntax_and_the_lines_indentation() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let rust = write(&root, "a.rs", b"fn f() {\n\n    let a = 1;\n}\n");
    let mut s = ready(d.path(), &root, &rust);
    // Lines 0–2 (the blank line stays blank), commented after each line's own indentation.
    s.dispatch(Command::Select(Selection { anchor: 8, head: 20 }));
    assert_eq!(ranges(&s), vec![8..20]);
    assert_eq!(s.disabled(CommandId::ToggleComment), None);
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "// fn f() {\n\n    // let a = 1;\n}\n");
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "fn f() {\n\n    let a = 1;\n}\n", "a second toggle takes the prefix back");
    // A partly commented run is completed, not removed: line 0 gains a prefix while line 3 takes the last one.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 3 }));
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "// fn f() {\n\n    let a = 1;\n}\n", "only line 0 had a prefix");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 28 }));
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "// fn f() {\n\n    // let a = 1;\n}\n", "the uncommented line is completed, the commented one kept");
    // One toggle is one undo step.
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "// fn f() {\n\n    let a = 1;\n}\n");
}

#[test]
fn markup_comments_wrap_the_selection_and_a_json_file_has_none() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let html = write(&root, "index.html", b"<p>x</p>\n");
    let mut s = ready(d.path(), &root, &html);
    s.dispatch(Command::Select(Selection { anchor: 0, head: 8 }));
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "<!-- <p>x</p> -->\n");
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "<p>x</p>\n", "the pair and its spaces go again");
    // A caret comments its whole line.
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "<!-- <p>x</p> -->\n");
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "<p>x</p>\n");

    let json = write(&root, "app.json", b"{\"a\": 1}\n");
    open(&mut s, Command::OpenRetained(json), "app.json");
    assert_eq!(s.disabled(CommandId::ToggleComment), Some("This file type has no comment syntax"));
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "{\"a\": 1}\n", "nothing changes where there is no comment syntax");
}

#[test]
fn moving_lines_keeps_mixed_endings_and_the_file_final_newline_in_place() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "mixed.rs", b"one\r\ntwo\nthree\r\nfour");
    let mut s = ready(d.path(), &root, &path);
    // The caret on line 2 moves that line over its neighbour; each line's own ending stays where it stands.
    s.dispatch(Command::Select(Selection { anchor: 13, head: 13 }));
    assert_eq!(s.disabled(CommandId::MoveLinesDown), None);
    s.dispatch(Command::MoveLines { down: true });
    assert_eq!(text(&s), "one\r\ntwo\nfour\r\nthree", "the endings stay at their positions");
    assert_eq!(ranges(&s), vec![19..19], "the caret follows the line it moved");
    s.dispatch(Command::MoveLines { down: false });
    assert_eq!(text(&s), "one\r\ntwo\nthree\r\nfour");
    // The last line cannot move down, nor the first up.
    s.dispatch(Command::Select(Selection { anchor: 22, head: 22 }));
    assert_eq!(s.disabled(CommandId::MoveLinesDown), Some("Already on the last line"));
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    assert_eq!(s.disabled(CommandId::MoveLinesUp), Some("Already on the first line"));
    // One move is one undo step.
    s.dispatch(Command::Select(Selection { anchor: 1, head: 1 }));
    s.dispatch(Command::MoveLines { down: true });
    assert_eq!(text(&s), "two\r\none\nthree\r\nfour", "each position keeps its own ending");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "one\r\ntwo\nthree\r\nfour");
    assert_eq!(std::fs::read(&path).unwrap(), b"one\r\ntwo\nthree\r\nfour", "no file is written");

    // A file that ends with a newline has no line below its last one to move past.
    let trailing = write(&root, "trailing.rs", b"a\nb\n");
    open(&mut s, Command::OpenRetained(trailing), "trailing.rs");
    s.dispatch(Command::Select(Selection { anchor: 2, head: 2 }));
    assert_eq!(s.disabled(CommandId::MoveLinesDown), Some("Already on the last line"));
    s.dispatch(Command::MoveLines { down: true });
    assert_eq!(text(&s), "a\nb\n");
}

#[test]
fn moving_a_multiline_selection_carries_all_its_lines_and_undoes_once() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "block.txt", b"a\nb\nc\nd\ne\n");
    let mut s = ready(d.path(), &root, &path);
    // Lines b and c move down together, and the selection keeps covering them.
    s.dispatch(Command::Select(Selection { anchor: 2, head: 6 }));
    assert_eq!(ranges(&s), vec![2..6]);
    s.dispatch(Command::MoveLines { down: true });
    assert_eq!(text(&s), "a\nd\nb\nc\ne\n");
    assert_eq!(ranges(&s), vec![4..8], "the selection still covers b and c");
    s.dispatch(Command::MoveLines { down: false });
    assert_eq!(text(&s), "a\nb\nc\nd\ne\n");
    assert_eq!(ranges(&s), vec![2..6]);
    // Two separate blocks move at once, each into the line next to it, as one transaction.
    let repeats = write(&root, "repeats.txt", b"x\na\nx\nb\n");
    open(&mut s, Command::OpenRetained(repeats), "repeats.txt");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 1 }));
    s.dispatch(Command::SelectNextOccurrence);
    assert_eq!(ranges(&s), vec![0..1, 4..5]);
    s.dispatch(Command::MoveLines { down: true });
    assert_eq!(text(&s), "a\nx\nb\nx\n");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "x\na\nx\nb\n", "one Undo reverses every moved block");
}

#[test]
fn the_project_indentation_choice_follows_the_file_and_applies_at_once() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "main.rs", b"fn main() {}\n");
    let mut s = ready(d.path(), &root, &path);
    // The language default for Rust is four spaces.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "    fn main() {}\n");
    s.dispatch(Command::Indent { outdent: true });
    // The project's own choice overrides the language default, and a change applies without reopening the file.
    let CatalogAction::Command(two) = s.action(CommandId::IndentTwoSpaces) else { panic!("a command") };
    s.dispatch(two);
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "  fn main() {}\n");
    // An EditorConfig nearer to the file wins over the project choice.
    std::fs::write(root.join(".editorconfig"), "root = true\n[*]\nindent_style = space\nindent_size = 3\n").unwrap();
    s.dispatch(Command::Indent { outdent: true });
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "   fn main() {}\n");
}

#[test]
fn duplicating_lines_copies_their_own_endings_and_keeps_the_caret_on_the_original() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "dup.rs", b"a\r\nb\r\nc\r\n");
    let mut s = ready(d.path(), &root, &path);
    s.dispatch(Command::Select(Selection { anchor: 3, head: 3 }));
    assert_eq!(s.disabled(CommandId::DuplicateLinesDown), None);
    s.dispatch(Command::DuplicateLines { down: true });
    assert_eq!(text(&s), "a\r\nb\r\nb\r\nc\r\n", "the copy keeps the CRLF line ending");
    assert_eq!(ranges(&s), vec![3..3], "the caret stays on the original line");
    s.dispatch(Command::DuplicateLines { down: false });
    assert_eq!(text(&s), "a\r\nb\r\nb\r\nb\r\nc\r\n", "duplicating up puts the copy above");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "a\r\nb\r\nb\r\nc\r\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "a\r\nb\r\nc\r\n", "each duplicate is one undo step");

    // A file whose last line has no newline keeps that state: the copy stands beside it.
    let no_newline = write(&root, "no-newline.rs", b"a\nb");
    open(&mut s, Command::OpenRetained(no_newline), "no-newline.rs");
    s.dispatch(Command::Select(Selection { anchor: 2, head: 2 }));
    s.dispatch(Command::DuplicateLines { down: true });
    assert_eq!(text(&s), "a\nb\nb");
    s.dispatch(Command::DuplicateLines { down: false });
    assert_eq!(text(&s), "a\nb\nb\nb");
    // A multiline selection is copied whole, with its endings.
    let block = write(&root, "block.rs", b"one\ntwo\r\nthree\nfour");
    open(&mut s, Command::OpenRetained(block), "block.rs");
    s.dispatch(Command::Select(Selection { anchor: 0, head: 7 }));
    s.dispatch(Command::DuplicateLines { down: true });
    assert_eq!(text(&s), "one\ntwo\r\none\ntwo\r\nthree\nfour");
    // An empty document has nothing to duplicate.
    let empty = write(&root, "empty.rs", b"");
    open(&mut s, Command::OpenRetained(empty), "empty.rs");
    assert_eq!(s.disabled(CommandId::DuplicateLinesDown), Some("Nothing to duplicate"));
    s.dispatch(Command::DuplicateLines { down: true });
    assert_eq!(text(&s), "");
}

#[test]
fn typed_delimiters_wrap_selections_and_open_pairs_only_where_the_language_has_room() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "pairs.rs", b"let a = x;\n");
    let mut s = ready(d.path(), &root, &path);
    // A selection is wrapped and stays selected.
    s.dispatch(Command::Select(Selection { anchor: 8, head: 9 }));
    s.dispatch(Command::InsertText { text: "(".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "let a = (x);\n");
    assert_eq!(ranges(&s), vec![9..10], "the wrapped text stays selected");
    // A caret gets the pair, with the caret between it.
    s.dispatch(Command::Select(Selection { anchor: 10, head: 10 }));
    s.dispatch(Command::InsertText { text: "{".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "let a = (x{});\n");
    assert_eq!(ranges(&s), vec![11..11]);
    // Typing the closer already in front of the caret steps over it instead of doubling it.
    s.dispatch(Command::InsertText { text: "}".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "let a = (x{});\n");
    assert_eq!(ranges(&s), vec![12..12], "the caret moved past the closer");
    // A quote pairs in front of a closer, and the one it made is stepped over in turn.
    s.dispatch(Command::InsertText { text: "\"".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "let a = (x{}\"\");\n");
    assert_eq!(ranges(&s), vec![13..13]);
    s.dispatch(Command::InsertText { text: "\"".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "let a = (x{}\"\");\n");
    assert_eq!(ranges(&s), vec![14..14]);
    // Each pair is one undo step.
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "let a = (x{});\n");
    // A paste is literal, never a pair.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::InsertText { text: "(".into(), kind: EditKind::Paste });
    assert_eq!(text(&s), "(let a = (x{});\n");

    // A quote after a word stays an apostrophe, and a pair in front of a word is not inserted.
    let words = write(&root, "words.rs", b"it\nx\n");
    open(&mut s, Command::OpenRetained(words), "words.rs");
    s.dispatch(Command::Select(Selection { anchor: 2, head: 2 }));
    s.dispatch(Command::InsertText { text: "'".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "it'\nx\n", "an apostrophe after a word is not a pair");
    s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
    s.dispatch(Command::InsertText { text: "(".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "it'\n(x\n", "no pair is opened in front of a word");
}

#[test]
fn prose_and_data_files_type_delimiters_literally() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "notes.md", b"text\n");
    let mut s = ready(d.path(), &root, &path);
    s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
    s.dispatch(Command::InsertText { text: "(".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "text(\n");
    let yaml = write(&root, "conf.yaml", b"a: b\n");
    open(&mut s, Command::OpenRetained(yaml), "conf.yaml");
    s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
    s.dispatch(Command::InsertText { text: "[".into(), kind: EditKind::Typing });
    assert_eq!(text(&s), "a: b[\n");
}

#[test]
fn structural_edits_refuse_gated_read_only_and_empty_sources_without_changing_anything() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "main.rs", b"let a = 1;\nlet b = 2;\n");
    // A session no adapter validated yet refuses every one of them and says why.
    let mut gated = session(d.path());
    gated.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut gated, scanned);
    gated.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut gated, |s| s.state().current_source().is_some());
    for id in [CommandId::IndentSelection, CommandId::OutdentSelection, CommandId::ToggleComment, CommandId::MoveLinesUp, CommandId::MoveLinesDown, CommandId::DuplicateLinesUp, CommandId::DuplicateLinesDown] {
        assert_eq!(gated.disabled(id), Some("Text input is gated on this unvalidated platform/backend"), "{id:?}");
    }
    // Every command leaves the text as it is while read-only, with the shared reason.
    let mut s = ready(d.path(), &root, &path);
    s.dispatch(Command::Select(Selection { anchor: 0, head: 9 }));
    let commands = [Command::Indent { outdent: false }, Command::Indent { outdent: true }, Command::ToggleComment,
        Command::MoveLines { down: true }, Command::MoveLines { down: false }, Command::DuplicateLines { down: true }, Command::DuplicateLines { down: false }];
    assert_eq!(s.disabled(CommandId::ToggleComment), None);
    s.dispatch(Command::SetReadOnly(true));
    for id in [CommandId::IndentSelection, CommandId::ToggleComment, CommandId::MoveLinesDown, CommandId::DuplicateLinesDown] {
        assert_eq!(s.disabled(id), Some("Read-only mode is on"), "{id:?}");
    }
    let before = text(&s);
    for command in commands {
        s.dispatch(command);
    }
    assert_eq!(text(&s), before, "a read-only draft never changes");
    assert_eq!(std::fs::read(&path).unwrap(), b"let a = 1;\nlet b = 2;\n", "and no file is written");
}

#[test]
fn an_empty_document_indents_but_has_nothing_to_comment_move_or_duplicate() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "empty.rs", b"");
    let mut s = ready(d.path(), &root, &path);
    assert_eq!(s.disabled(CommandId::ToggleComment), Some("There is no text to comment"));
    assert_eq!(s.disabled(CommandId::MoveLinesDown), Some("Already on the last line"));
    assert_eq!(s.disabled(CommandId::MoveLinesUp), Some("Already on the first line"));
    assert_eq!(s.disabled(CommandId::DuplicateLinesDown), Some("Nothing to duplicate"));
    for command in [Command::ToggleComment, Command::MoveLines { down: true }, Command::MoveLines { down: false }, Command::DuplicateLines { down: true }] {
        s.dispatch(command);
    }
    assert_eq!(text(&s), "");
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "    ", "indenting an empty document writes the file's step at the caret");
    s.dispatch(Command::Indent { outdent: true });
    assert_eq!(text(&s), "");
}

#[test]
fn multiline_unicode_selections_move_whole_lines_and_keep_their_columns() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "unicode.rs", "😀 one\nλ two\n三 three\n".as_bytes());
    let mut s = ready(d.path(), &root, &path);
    // From inside the first line to inside the second: both move down together over the third.
    s.dispatch(Command::Select(Selection { anchor: 5, head: 12 }));
    s.dispatch(Command::MoveLines { down: true });
    assert_eq!(text(&s), "三 three\n😀 one\nλ two\n");
    assert_eq!(ranges(&s), vec![15..22], "the selection keeps its columns on the same lines");
    s.dispatch(Command::Indent { outdent: false });
    assert_eq!(text(&s), "三 three\n    😀 one\n    λ two\n", "both lines take the step");
    s.dispatch(Command::DuplicateLines { down: true });
    assert_eq!(text(&s), "三 three\n    😀 one\n    λ two\n    😀 one\n    λ two\n");
    let tab = s.state().active().unwrap().id();
    for _ in 0..3 { s.dispatch(Command::Undo { tab }); }
    assert_eq!(text(&s), "😀 one\nλ two\n三 three\n", "three undos take all three actions back");
}

#[test]
fn a_structural_edit_makes_the_shown_draft_compare_again() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(d.path(), "marked.txt", b"one\ntwo\nthree\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "marked.txt");
    s.enable_text_input_for_test();
    let tab = s.state().active().unwrap().id();
    let settle_markers = |s: &mut Session| {
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while !s.state().markers(tab).is_some_and(|markers| !markers.pending() && markers.summary().is_some()) {
            assert!(std::time::Instant::now() < until, "markers did not settle");
            s.dispatch(Completion::MarkersDue);
            while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle_markers(&mut s);
    assert_eq!(s.state().markers(tab).and_then(crate::markers::Markers::summary).as_deref(), Some("vs last saved: no changes"));
    // Moving a line changes it against the last saved version, so the markers recompute for the new text.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::MoveLines { down: true });
    assert_eq!(text(&s), "two\none\nthree\n");
    settle_markers(&mut s);
    let markers = s.state().markers(tab).expect("markers");
    assert_eq!(markers.drawn_version(), s.state().current_source().map(|d| d.key().version), "the drawn blocks describe the moved text");
    assert_eq!(markers.summary().as_deref(), Some("vs last saved: 1 added, 1 deleted"), "the moved lines are compared as they are now");
    // Back to the saved text, the markers are clean again rather than keeping the moved lines marked.
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    settle_markers(&mut s);
    assert_eq!(s.state().markers(tab).and_then(crate::markers::Markers::summary).as_deref(), Some("vs last saved: no changes"));
}

#[test]
fn the_structural_edit_chords_come_from_the_keymap_preset_and_never_hide_a_command() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, "keys.rs", b"one\ntwo\n");
    let mut s = ready(d.path(), &root, &path);
    let press = |s: &Session, text: &str| {
        let chord = crate::keymap::Chord::parse(text).unwrap();
        s.key_command(&crate::keymap::KeyPress::new(&chord.key, chord.ctrl, chord.shift, chord.alt).unwrap())
    };
    // IntelliJ: its Move Statement arrows move lines, Alt+Shift+arrows duplicate them (IntelliJ's own Ctrl+D is
    // Compare Selected here), and Ctrl+/ comments.
    assert_eq!(press(&s, "Ctrl+Shift+Down"), Some(CommandId::MoveLinesDown));
    assert_eq!(press(&s, "Ctrl+Shift+Up"), Some(CommandId::MoveLinesUp));
    assert_eq!(press(&s, "Alt+Shift+Down"), Some(CommandId::DuplicateLinesDown));
    assert_eq!(press(&s, "Alt+Shift+Up"), Some(CommandId::DuplicateLinesUp));
    assert_eq!(press(&s, "Ctrl+/"), Some(CommandId::ToggleComment));
    assert_eq!(press(&s, "Alt+/"), Some(CommandId::ToggleComment), "the route terminals always deliver");
    assert_eq!(press(&s, "Ctrl+D"), Some(CommandId::CompareSelected), "Compare Selected keeps its chord");
    assert_eq!(s.key_hint(CommandId::IndentSelection).as_deref(), Some("Tab"), "indent is a fixed editing key");
    assert_eq!(s.key_hint(CommandId::OutdentSelection).as_deref(), Some("Shift+Tab"));
    // VS Code: its own line-move arrows; the rest is the same.
    s.dispatch(Command::SetKeymapPreset(crate::keymap::KeymapPreset::VsCode));
    assert_eq!(press(&s, "Alt+Down"), Some(CommandId::MoveLinesDown));
    assert_eq!(press(&s, "Alt+Up"), Some(CommandId::MoveLinesUp));
    assert_eq!(press(&s, "Alt+Shift+Down"), Some(CommandId::DuplicateLinesDown));
    assert_eq!(press(&s, "Ctrl+/"), Some(CommandId::ToggleComment));
    assert_eq!(s.key_hint(CommandId::MoveLinesDown).as_deref(), Some("Alt+Down"));
    // A key runs the command its menu row names, and the rows are in the Selection menu both adapters show.
    let Some(CommandId::ToggleComment) = press(&s, "Ctrl+/") else { panic!("the chord resolves") };
    let CatalogAction::Command(command) = s.action(CommandId::ToggleComment) else { panic!("a command") };
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(command);
    assert_eq!(text(&s), "// one\ntwo\n");
    for id in [CommandId::IndentSelection, CommandId::OutdentSelection, CommandId::ToggleComment, CommandId::MoveLinesUp, CommandId::MoveLinesDown, CommandId::DuplicateLinesUp, CommandId::DuplicateLinesDown] {
        assert_eq!(id.group(), crate::actions::CommandGroup::Selection, "{id:?}");
        assert!(s.actions().iter().any(|action| action.id == id && action.label == id.label()), "{id:?} is listed");
    }
}
