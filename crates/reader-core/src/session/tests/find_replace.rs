//! Replace within the current draft: the find options, the version the matches were found for, one undoable
//! action per replace, and the selections and results left behind.
use super::*;

fn found_matches(s: &Session) -> &[(usize, std::ops::Range<usize>)] {
    s.state().active().unwrap().find().unwrap().matches()
}

fn selection_range(s: &Session) -> std::ops::Range<usize> {
    let selection = s.state().selection().unwrap();
    selection.anchor.min(selection.head)..selection.anchor.max(selection.head)
}

#[test]
fn replacing_the_current_match_uses_the_find_options_and_undoes_once() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"Foo foo food\n");
    let other = write(d.path(), "b.txt", b"Foo foo\n");
    let mut s = session(d.path());
    open(&mut s, Command::Open(other.clone()), "b.txt");
    open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
    s.enable_text_input_for_test();
    caret(&mut s, 0);
    find(&mut s, "foo");
    assert_eq!(found_matches(&s), &[(0, 0..3), (0, 4..7), (0, 8..11)], "case is ignored by default");
    assert_eq!(selection_range(&s), 0..3, "the first match is selected");

    s.dispatch(Command::SetReplaceText("bar".into()));
    assert_eq!(s.disabled(CommandId::ReplaceMatch), None);
    assert_eq!(s.disabled(CommandId::ReplaceAll), None);
    s.dispatch(Command::ReplaceFind { all: false });
    assert_eq!(source_text(s.state().current_source().unwrap()), "bar foo food\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"Foo foo food\n", "the draft is not written");
    assert_eq!(std::fs::read(&other).unwrap(), b"Foo foo\n", "no other file is changed");

    // The results follow the replacement: two matches remain, and the next one is selected.
    settle(&mut s, found);
    assert_eq!(found_matches(&s), &[(0, 4..7), (0, 8..11)]);
    assert_eq!(s.state().active().unwrap().find().unwrap().current(), Some(0));
    assert_eq!(selection_range(&s), 4..7, "replace advances to the next match");

    s.dispatch(Command::ReplaceFind { all: false });
    settle(&mut s, found);
    assert_eq!(source_text(s.state().current_source().unwrap()), "bar bar food\n");
    assert_eq!(found_matches(&s), &[(0, 8..11)]);

    // Each explicit replace is one undo step, back to the exact original bytes on disk.
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "bar foo food\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "Foo foo food\n");
    assert!(!active_view(&s).can_undo(), "no earlier step is left behind");
    assert_eq!(std::fs::read(&path).unwrap(), b"Foo foo food\n");
    assert_eq!(std::fs::read(&other).unwrap(), b"Foo foo\n");
}

#[test]
fn replace_all_replaces_every_listed_match_as_one_action() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"one foo\ntwo foo\nfoo three\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
    s.enable_text_input_for_test();
    find(&mut s, "foo");
    assert_eq!(found_matches(&s).len(), 3);
    s.dispatch(Command::SetReplaceText("multi\nline".into()));
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "one multi\nline\ntwo multi\nline\nmulti\nline three\n");
    assert!(s.state().notices().iter().any(|n| n == "Replaced 3 matches"), "{:?}", s.state().notices());
    assert_eq!(std::fs::read(&path).unwrap(), b"one foo\ntwo foo\nfoo three\n", "replace never writes the file");
    // The results follow the new text: nothing is left to replace.
    settle(&mut s, found);
    assert!(found_matches(&s).is_empty());
    assert_eq!(s.disabled(CommandId::ReplaceAll), Some("No matches to replace"));
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().current_source().unwrap()), "one foo\ntwo foo\nfoo three\n", "one action, one undo");
    settle(&mut s, found);
    assert_eq!(found_matches(&s).len(), 3, "the results follow the undone text too");
}

#[test]
fn a_capped_search_refuses_replace_all_and_still_replaces_the_current_match() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", "x ".repeat(crate::find::MATCH_LIMIT + 5).as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "a.txt");
    s.enable_text_input_for_test();
    find(&mut s, "x");
    let find = s.state().active().unwrap().find().unwrap();
    assert_eq!((find.matches().len(), find.capped()), (crate::find::MATCH_LIMIT, true));
    s.dispatch(Command::SetReplaceText("y".into()));
    assert_eq!(s.disabled(CommandId::ReplaceAll), Some("Only the first 10,000 matches are listed; narrow the search to replace all"));
    assert_eq!(s.disabled(CommandId::ReplaceMatch), None);
    let before = source_text(s.state().current_source().unwrap());
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), before, "a capped replace all changes nothing");
    s.dispatch(Command::ReplaceFind { all: false });
    let after = source_text(s.state().current_source().unwrap());
    assert_eq!(after.len(), before.len());
    assert_eq!(after.matches('y').count(), 1, "only the current match was replaced");
}

#[test]
fn invalid_pending_and_missing_finds_say_why_nothing_was_replaced() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"foo bar\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::SetReplaceText("qux".into()));
    assert_eq!(s.disabled(CommandId::ReplaceMatch), Some("No find is open"));
    s.dispatch(Command::ReplaceFind { all: false });
    assert!(s.state().notices().last().is_some_and(|n| n == "Replace: No find is open"), "{:?}", s.state().notices());

    // A search still running has no matches to act on yet.
    s.dispatch(Command::Find("foo".into()));
    assert_eq!(s.disabled(CommandId::ReplaceMatch), Some("Wait for the search to finish"));
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "foo bar\n");

    // An invalid expression keeps its reason: nothing is replaced and no stale match is reused.
    settle(&mut s, found);
    s.dispatch(Command::ToggleFindOption(FindOption::Regex));
    s.dispatch(Command::Find("(".into()));
    settle(&mut s, found);
    assert_eq!(s.disabled(CommandId::ReplaceMatch), Some("The search pattern is invalid"));
    s.dispatch(Command::ReplaceFind { all: false });
    assert_eq!(source_text(s.state().current_source().unwrap()), "foo bar\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"foo bar\n");

    // A valid query with no match says so.
    s.dispatch(Command::ToggleFindOption(FindOption::Regex));
    s.dispatch(Command::Find("absent".into()));
    settle(&mut s, found);
    assert_eq!(s.disabled(CommandId::ReplaceAll), Some("No matches to replace"));
}

#[test]
fn an_intervening_edit_rejects_the_captured_matches() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"foo one\nfoo two\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "a.txt");
    s.enable_text_input_for_test();
    find(&mut s, "foo");
    assert_eq!(found_matches(&s).len(), 2);
    s.dispatch(Command::SetReplaceText("bar".into()));
    // Another edit lands before Replace: the matches are for the version before it.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::InsertText { text: "x".into(), kind: crate::text::EditKind::Typing });
    assert_eq!(s.disabled(CommandId::ReplaceAll), Some("Wait for the search to finish"), "the edit's rerun supersedes the old matches");
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "xfoo one\nfoo two\n", "no stale match was replaced");
    assert_eq!(std::fs::read(&path).unwrap(), b"foo one\nfoo two\n");
    // Once the rerun arrives the new matches can be replaced.
    settle(&mut s, found);
    assert_eq!(s.disabled(CommandId::ReplaceAll), None);
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "xbar one\nbar two\n");
}

#[test]
fn read_only_saved_views_and_previews_refuse_replace() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.md", b"# foo\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "a.md");
    s.enable_text_input_for_test();
    find(&mut s, "foo");
    s.dispatch(Command::SetReplaceText("bar".into()));
    s.dispatch(Command::SetReadOnly(true));
    assert_eq!(s.disabled(CommandId::ReplaceMatch), Some("Read-only mode is on"));
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "# foo\n");
    s.dispatch(Command::SetReadOnly(false));
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| rendered(s).is_some());
    assert_eq!(s.disabled(CommandId::ReplaceAll), Some("Replace works on the source; Ctrl+Shift+V shows it"));
}

#[test]
fn replace_changes_only_the_focused_draft() {
    let d = tempfile::tempdir().unwrap();
    let a = write(d.path(), "a.txt", b"foo\n");
    let b = write(d.path(), "b.txt", b"foo\n");
    let mut s = session(&d.path().join("first"));
    s.enable_text_input_for_test();
    open(&mut s, Command::OpenRetained(a.clone()), "a.txt");
    let a_tab = s.state().active().unwrap().id();
    let moved_a = |s: &Session| s.state().current_source().is_some_and(|d| d.source().ends_with("a.txt")) && found(s);
    find(&mut s, "foo");
    s.dispatch(Command::SetReplaceText("bar".into()));
    let mut other = session(&d.path().join("second"));
    open(&mut other, Command::OpenRetained(a.clone()), "a.txt");
    // The other tab has no find: the active tab alone decides what is replaced.
    open(&mut s, Command::OpenRetained(b.clone()), "b.txt");
    s.dispatch(Command::ReplaceFind { all: true });
    assert!(s.state().notices().last().is_some_and(|n| n == "Replace: No find is open"), "{:?}", s.state().notices());
    assert_eq!(source_text(s.state().current_source().unwrap()), "foo\n", "the focused tab is unchanged");
    s.dispatch(Command::ActivateTab(a_tab));
    settle(&mut s, moved_a);
    assert_eq!(s.disabled(CommandId::ReplaceAll), None, "its own find is still open");
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "bar\n");
    assert_eq!(std::fs::read(&a).unwrap(), b"foo\n", "another file and the disk are untouched");
    assert_eq!(std::fs::read(&b).unwrap(), b"foo\n");
    assert_eq!(source_text(other.state().current_source().unwrap()), "foo\n", "another session keeps its own draft");
}

#[test]
fn replace_keeps_several_cursors_coherent() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"foo\nfoo\nfoo\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "a.txt");
    s.enable_text_input_for_test();
    find(&mut s, "foo");
    s.dispatch(Command::SetReplaceText("bar".into()));
    // A second caret below the selected match: the next match is the primary one after the replace.
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(s.state().active().unwrap().selections().count(), 2);
    s.dispatch(Command::ReplaceFind { all: false });
    assert_eq!(source_text(s.state().current_source().unwrap()), "bar\nfoo\nfoo\n");
    settle(&mut s, found);
    let selections: Vec<_> = s.state().active().unwrap().selections().collect();
    assert_eq!(selections, [Selection { anchor: 8, head: 11 }], "the next match is the only selection, as a find step leaves it");
    // Replace All carries every caret through the one transaction: the kept match is replaced too.
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::AddCursorBelow);
    assert_eq!(s.state().active().unwrap().selections().count(), 2);
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "bar\nbar\nbar\n");
    let mut heads: Vec<_> = s.state().active().unwrap().selections().map(|s| s.head).collect();
    heads.sort_unstable();
    assert_eq!(heads, [0, 7], "a caret on a replaced match follows its replacement; one before it stays");
}

#[test]
fn replacing_a_match_with_its_own_text_steps_on_without_an_undo_step() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"foo foo\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "a.txt");
    s.enable_text_input_for_test();
    find(&mut s, "foo");
    s.dispatch(Command::SetReplaceText("foo".into()));
    s.dispatch(Command::ReplaceFind { all: false });
    assert_eq!(source_text(s.state().current_source().unwrap()), "foo foo\n");
    assert_eq!(s.state().active().unwrap().find().unwrap().current(), Some(1), "the next match is current");
    assert_eq!(selection_range(&s), 4..7);
    assert!(!active_view(&s).can_undo(), "nothing was recorded to undo");
    s.dispatch(Command::Undo { tab: s.state().active().unwrap().id() });
    assert_eq!(source_text(s.state().current_source().unwrap()), "foo foo\n");
}

#[test]
fn replace_honours_the_find_options() {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "a.txt", b"Foo foo Foo\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path), "a.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::ToggleFindOption(FindOption::Case));
    find(&mut s, "Foo");
    s.dispatch(Command::SetReplaceText("bar".into()));
    s.dispatch(Command::ReplaceFind { all: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), "bar foo bar\n");
}
