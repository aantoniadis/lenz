use super::*;
use crate::text::TextEdit;
use crate::actions::CatalogAction;

const DATABASE: &str = "default: &default\n  adapter: postgresql\n  pool: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n\ndevelopment:\n  <<: *default\n<% if ENV[\"CI\"] %>\n  host: <%= ENV[\"DB_HOST\"] %>\n<% end %>\n";

#[test]
fn yaml_read_through_erb_has_no_syntax_problems_and_its_server_reads_the_masked_copy() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("app");
    std::fs::create_dir_all(root.join("config")).unwrap();
    let path = write(&root, "config/database.yml", DATABASE.as_bytes());
    let broken = write(&root, "config/broken.yml", b"a: [1, 2\nb: c\n");
    let mut s = session(dir.path());
    s.dispatch(Command::OpenRoot(root));
    settle(&mut s, scanned);
    // A YAML error elsewhere is still found: the check works, and ERB is no error.
    open(&mut s, Command::OpenRetained(broken), "broken.yml");
    let problems = |s: &Session| s.state().active().and_then(|t| s.state().diagnostics(t.id())).map(|d| d.items.len());
    settle(&mut s, |s| problems(s).is_some_and(|n| n > 0));
    open(&mut s, Command::OpenRetained(path.clone()), "database.yml");
    settle(&mut s, |s| problems(s) == Some(0));
    let doc = crate::document::Document::decode(path.clone(), 1, DATABASE.as_bytes());
    let wire = doc.wire_text();
    assert_eq!(wire.len(), DATABASE.len(), "every offset unchanged");
    assert!(!wire.contains("<%") && wire.contains("\n#"), "tags masked: {wire}");
    // A server edit reaching a tag is refused; one beside it is not.
    let at = DATABASE.find("<%=").unwrap();
    assert!(doc.erb_tag_refusal(&[TextEdit { range: at + 2..at + 5, text: "x".into() }]).is_some());
    assert!(doc.erb_tag_refusal(&[TextEdit { range: at..at, text: "x".into() }]).is_none(), "an insertion at the tag's edge");
    assert!(doc.erb_tag_refusal(&[TextEdit { range: 0..7, text: "base".into() }]).is_none());
    // Format on Save leaves such a file as typed.
    s.enable_text_input_for_test();
    let CatalogAction::Command(command) = s.action(CommandId::FormatOnSave) else { panic!("format on save toggles") };
    s.dispatch(command);
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: 0..0, text: "# edited\n".into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !active_view(s).dirty());
    assert_eq!(std::fs::read_to_string(path).unwrap(), format!("# edited\n{DATABASE}"));
}
