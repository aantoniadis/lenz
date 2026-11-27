use super::*;
use crate::actions::CatalogAction;
use crate::text::{EditKind, TextEdit};

fn ready(dir: &Path, text: &str) -> (Session, PathBuf) {
    let root = dir.join("project");
    std::fs::create_dir(&root).unwrap();
    write(&root, ".editorconfig", b"[*.json]\nindent_style = space\nindent_size = 2\n");
    let path = write(&root, "data.json", text.as_bytes());
    let mut s = session(dir);
    s.dispatch(Command::OpenRoot(root));
    settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(path.clone()), "data.json");
    s.enable_text_input_for_test();
    (s, path)
}

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }

fn run(s: &mut Session, id: CommandId) {
    let CatalogAction::Command(command) = s.action(id) else { panic!("{id:?} must route to the shared command") };
    s.dispatch(command);
}

fn replace(s: &mut Session, value: &str) {
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..text(s).len(), text: value.into() }], kind: EditKind::Replace });
}

#[test]
fn json_format_catalog_routes_offline_to_one_undo_step_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let original = "{\"z\":1e+999,\"z\":90071992547409931234567890,\"a\":[true,null]}\r\n";
    let formatted = "{\r\n  \"z\": 1e+999,\r\n  \"z\": 90071992547409931234567890,\r\n  \"a\": [\r\n    true,\r\n    null\r\n  ]\r\n}\r\n";
    let (mut s, path) = ready(dir.path(), original);
    assert_eq!(s.disabled(CommandId::FormatDocument), None);
    assert_eq!(s.disabled(CommandId::OrganizeImports), Some("JSON has no imports to organize"));
    run(&mut s, CommandId::FormatDocument);
    settle(&mut s, |s| s.state.formatting.is_none());
    assert_eq!(text(&s), formatted);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "format changes only the draft");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), original);
    assert!(!active_view(&s).can_undo(), "one formatting transaction");
    s.dispatch(Command::Redo { tab });
    assert_eq!(text(&s), formatted);
    run(&mut s, CommandId::FormatDocument);
    settle(&mut s, |s| s.state.formatting.is_none());
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), original, "formatting already formatted text adds no history");
}

#[test]
fn json_format_on_save_writes_formatted_bytes_and_invalid_json_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let original = "{}\r\n";
    let (mut s, path) = ready(dir.path(), original);
    run(&mut s, CommandId::FormatOnSave);
    replace(&mut s, "{\"a\":[1,2]}\r\n");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !active_view(s).dirty());
    let expected = "{\r\n  \"a\": [\r\n    1,\r\n    2\r\n  ]\r\n}\r\n";
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    replace(&mut s, "{\"a\":}\r\n");
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| s.state.formatting.is_none());
    assert_eq!(text(&s), "{\"a\":}\r\n");
    assert!(active_view(&s).dirty());
    assert_eq!(std::fs::read(path).unwrap(), expected.as_bytes());
    assert!(s.state.messages.iter().any(|m| m.contains("nothing was written")));
}

#[test]
fn json_format_cancellation_and_changed_drafts_reject_late_worker_results() {
    for cancel in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let original = "{\"a\":1}";
        let (mut s, path) = ready(dir.path(), original);
        run(&mut s, CommandId::FormatDocument);
        let result = loop {
            let next = s.completions().recv_blocking_timeout().unwrap();
            if matches!(next, Completion::ProjectFormatted { .. }) { break next; }
            s.dispatch(next);
        };
        if cancel { s.cancel_format_by_user(); }
        else { replace(&mut s, "{\"new\":2}"); }
        s.dispatch(result);
        assert_eq!(text(&s), if cancel { original } else { "{\"new\":2}" });
        assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
        assert!(s.state.formatting.is_none());
    }
}

#[test]
fn json_format_on_save_uses_the_target_tabs_indentation() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, path) = ready(dir.path(), "{}");
    run(&mut s, CommandId::FormatOnSave);
    replace(&mut s, "{\"a\":1}");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Split(Split::Columns));
    let other = write(path.parent().unwrap(), "other.txt", b"untouched");
    open(&mut s, Command::OpenRetained(other), "other.txt");
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| s.state().view(tab).is_some_and(|v| !v.dirty()));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "{\n  \"a\": 1\n}");
    assert_eq!(text(&s), "untouched");
}

fn ready_file(dir: &Path, name: &str, text: &str) -> (Session, PathBuf) {
    let root = dir.join("project");
    std::fs::create_dir_all(&root).unwrap();
    write(&root, ".editorconfig", b"[*]\nindent_style = space\nindent_size = 2\n");
    let path = write(&root, name, text.as_bytes());
    let mut s = session(dir);
    s.dispatch(Command::OpenRoot(root));
    settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(path.clone()), name);
    s.enable_text_input_for_test();
    (s, path)
}

#[test]
fn format_selection_expands_pasted_json_in_place_in_any_file_as_one_undo_step() {
    let dir = tempfile::tempdir().unwrap();
    let original = "# Notes\r\n  payload = {\"a\":[1,{\"b\":null}],\"c\":\"x\"}  \r\nend\r\n";
    let (mut s, path) = ready_file(dir.path(), "notes.md", original);
    assert_eq!(s.disabled(CommandId::FormatSelection), Some("Select the text to format first"));
    let start = original.find('{').unwrap();
    let end = original.rfind('}').unwrap() + 3; // the trailing blanks stay outside the formatted value
    s.dispatch(Command::Select(Selection { anchor: start, head: end }));
    assert_eq!(s.disabled(CommandId::FormatSelection), None);
    assert_eq!(s.disabled(CommandId::FormatDocument), None, "Format Document formats a JSON selection");
    // The keyboard route: Format Document (Ctrl+Alt+L) with JSON selected formats the selection alone.
    let CatalogAction::Command(command) = s.action(CommandId::FormatDocument) else { panic!("routes to the shared command") };
    assert_eq!(command, Command::Format(FormatKind::Selection));
    s.dispatch(command);
    settle(&mut s, |s| s.state.formatting.is_none());
    let expected = "# Notes\r\n  payload = {\r\n    \"a\": [\r\n      1,\r\n      {\r\n        \"b\": null\r\n      }\r\n    ],\r\n    \"c\": \"x\"\r\n  }  \r\nend\r\n";
    assert_eq!(text(&s), expected);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "formatting changes only the draft");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), original);
    assert!(!active_view(&s).can_undo(), "one formatting transaction");
}

#[test]
fn format_selection_refuses_invalid_and_unformattable_selections_without_an_edit() {
    let dir = tempfile::tempdir().unwrap();
    let original = "let a = {\"a\":};\nlet b = plain;\n";
    let (mut s, _) = ready_file(dir.path(), "notes.txt", original);
    let start = original.find('{').unwrap();
    s.dispatch(Command::Select(Selection { anchor: start, head: start + 7 }));
    // Not well-formed: Ctrl+Alt+L keeps formatting the document; the explicit command says why it cannot.
    assert_ne!(s.action(CommandId::FormatDocument), CatalogAction::Command(Command::Format(FormatKind::Selection)));
    run(&mut s, CommandId::FormatSelection);
    settle(&mut s, |s| s.state.formatting.is_none());
    assert_eq!(text(&s), original);
    assert!(s.state.messages.iter().any(|m| m.starts_with("Format Selection: JSON formatting:")), "{:?}", s.state.messages);
    let plain = original.find("plain").unwrap();
    s.dispatch(Command::Select(Selection { anchor: plain, head: plain + 5 }));
    assert_eq!(s.disabled(CommandId::FormatSelection), Some("Format Selection formats JSON or XML; select a value or element"));
    s.dispatch(Command::Select(Selection { anchor: 3, head: 4 }));
    assert_eq!(s.disabled(CommandId::FormatSelection), Some("The selection holds only blanks"));
}

#[test]
fn format_document_formats_a_plain_text_document_holding_json_but_save_never_guesses() {
    let dir = tempfile::tempdir().unwrap();
    let original = "{\"pasted\":[true,false]}";
    let (mut s, path) = ready_file(dir.path(), "paste.txt", original);
    assert_eq!(s.disabled(CommandId::FormatDocument), None);
    run(&mut s, CommandId::FormatDocument);
    settle(&mut s, |s| s.state.formatting.is_none());
    assert_eq!(text(&s), "{\n  \"pasted\": [\n    true,\n    false\n  ]\n}");
    run(&mut s, CommandId::FormatOnSave);
    replace(&mut s, "{not json");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !active_view(s).dirty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "{not json", "a plain-text file is saved as typed");
}

#[test]
fn xml_format_document_reindents_element_only_content_and_organize_has_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let original = "<root><a x=\"1\"/><b>text  kept</b></root>\n";
    let (mut s, path) = ready_file(dir.path(), "data.xml", original);
    assert_eq!(s.disabled(CommandId::OrganizeImports), Some("XML has no imports to organize"));
    assert_eq!(s.disabled(CommandId::FormatDocument), None);
    run(&mut s, CommandId::FormatDocument);
    settle(&mut s, |s| s.state.formatting.is_none());
    assert_eq!(text(&s), "<root>\n  <a x=\"1\"/>\n  <b>text  kept</b>\n</root>\n");
    replace(&mut s, "<root><a></root>\n");
    run(&mut s, CommandId::FormatDocument);
    settle(&mut s, |s| s.state.formatting.is_none());
    assert_eq!(text(&s), "<root><a></root>\n", "malformed XML stays unchanged");
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}
