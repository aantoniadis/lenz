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
