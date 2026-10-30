//! Environment files through the Session command seam: `#` comments, quotes that pair only where a value starts,
//! literal typing, paste and Undo, exact bytes on Save, and contents that stay out of language servers, the
//! declaration index, saved places, settings and the details text.
use super::*;
use crate::actions::CatalogAction;
use crate::text::EditKind;
use lenz_content::highlight::Language;

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }

fn ranges(s: &Session) -> Vec<std::ops::Range<usize>> {
    let mut ranges: Vec<_> = s.state().active().unwrap().selections().map(|selection| selection.range()).collect();
    ranges.sort_by_key(|r| (r.start, r.end));
    ranges
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

fn caret(s: &mut Session, at: usize) { s.dispatch(Command::Select(Selection { anchor: at, head: at })); }

fn typed(s: &mut Session, typed: &str) {
    for c in typed.chars() { s.dispatch(Command::InsertText { text: c.to_string(), kind: EditKind::Typing }); }
}

/// Undo (at most four steps: typing within half a second is one) until the draft reads `expected`.
fn undo_to(s: &mut Session, expected: &str) {
    let tab = s.state().active().unwrap().id();
    for _ in 0..4 { if text(s) != expected { s.dispatch(Command::Undo { tab }); } }
    assert_eq!(text(s), expected);
}

fn run(s: &mut Session, id: CommandId) {
    let CatalogAction::Command(command) = s.action(id) else { panic!("{id:?} is not a command: {:?}", s.action(id)) };
    s.dispatch(command);
}

/// Every file under `dir` (recursively) whose bytes contain `needle`.
fn holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() { stack.push(path); }
            else if std::fs::read(&path).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes())) { found.push(path); }
        }
    }
    found.sort();
    found
}

#[test]
fn environment_files_are_detected_by_name_and_comment_with_a_hash() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, ".env.local", b"KEY=1\n  # note\n");
    let mut s = ready(d.path(), &root, &path);
    assert_eq!(s.state().current_source().unwrap().language(), Some(Language::Env));
    // Toggle Comment uses `#` after the line's own indentation, and takes it back; one Undo step each.
    caret(&mut s, 0);
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "# KEY=1\n  # note\n");
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "KEY=1\n  # note\n");
    s.dispatch(Command::Select(Selection { anchor: 0, head: text(&s).len() }));
    s.dispatch(Command::ToggleComment);
    assert_eq!(text(&s), "# KEY=1\n  # note\n", "a partly commented run is completed");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "KEY=1\n  # note\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"KEY=1\n  # note\n", "no file is written");
}

#[test]
fn quotes_pair_only_where_a_value_starts_and_brackets_stay_literal() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let path = write(&root, ".env", b"export KEY=\nNEXT=ab\n");
    let mut s = ready(d.path(), &root, &path);
    // Right after `=`, with nothing after it, a quote opens its pair; its closer is then stepped over.
    caret(&mut s, "export KEY=".len());
    typed(&mut s, "\"");
    assert_eq!(text(&s), "export KEY=\"\"\nNEXT=ab\n");
    assert_eq!(ranges(&s), vec![12..12], "the caret sits between the quotes");
    typed(&mut s, "a\"");
    assert_eq!(text(&s), "export KEY=\"a\"\nNEXT=ab\n", "the typed closer steps over the one already there");
    assert_eq!(ranges(&s), vec![14..14]);
    undo_to(&mut s, "export KEY=\nNEXT=ab\n");
    // Inside a value nothing pairs: a quote, brackets, braces and backticks are data.
    let at = "export KEY=\nNEXT=a".len();
    caret(&mut s, at);
    typed(&mut s, "'${HOME}$(id)`[x]`\"");
    assert_eq!(text(&s), "export KEY=\nNEXT=a'${HOME}$(id)`[x]`\"b\n", "every character typed once, none added");
    // A selection is wrapped in the quote typed, and stays selected.
    undo_to(&mut s, "export KEY=\nNEXT=ab\n");
    let value = "export KEY=\nNEXT=".len();
    s.dispatch(Command::Select(Selection { anchor: value, head: value + 2 }));
    typed(&mut s, "'");
    assert_eq!(text(&s), "export KEY=\nNEXT='ab'\n");
    assert_eq!(ranges(&s), vec![value + 1..value + 3]);
    // An escaped quote at a double-quoted value's end is typed, not stepped over.
    let file = write(&root, "app.env", b"A=\"x\\\"\n");
    open(&mut s, Command::OpenRetained(file), "app.env");
    caret(&mut s, "A=\"x\\".len());
    typed(&mut s, "\"");
    assert_eq!(text(&s), "A=\"x\\\"\"\n");
    // A paste is literal.
    caret(&mut s, 0);
    s.dispatch(Command::InsertText { text: "\"".into(), kind: EditKind::Paste });
    assert_eq!(text(&s), "\"A=\"x\\\"\"\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"export KEY=\nNEXT=ab\n");
}

/// Values are data: typing, paste, selection replacement and Undo keep exactly the bytes given (Unicode, CRLF, LF,
/// `$VAR`, `$(…)`, backticks), nothing is run or expanded, and Save writes the draft's bytes even with the
/// project's format-on-save on.
#[test]
fn typing_paste_undo_and_save_keep_values_literal_with_unicode_and_line_endings() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    let original = "# 𝄞 config\r\nPASSWORD='p@$$w0rd'\r\nCMD=$(touch ran)\nTICK=`touch ticked`\r\nNO_EOL=é";
    let path = write(&root, ".env", original.as_bytes());
    let mut s = ready(d.path(), &root, &path);
    let tab = s.state().active().unwrap().id();
    run(&mut s, CommandId::FormatOnSave);
    assert!(s.state().project_preferences().is_some_and(|p| p.format_on_save));
    // A paste with its own CRLF and LF endings, Unicode and shell syntax goes in byte for byte.
    let pasted = "NEW=\"${HOME} \\n 😀\"\r\nRAW=$(rm -rf /tmp/x)\n";
    let at = original.find("CMD=").unwrap();
    caret(&mut s, at);
    s.dispatch(Command::Paste(pasted.into()));
    let pasted_text = format!("{}{pasted}{}", &original[..at], &original[at..]);
    assert_eq!(text(&s), pasted_text);
    // A selection replaced by typing, over a CRLF boundary.
    let from = pasted_text.find("p@$$").unwrap();
    let to = pasted_text.find("NEW=").unwrap();
    s.dispatch(Command::Select(Selection { anchor: from, head: to }));
    typed(&mut s, "λ'\r");
    let replaced = format!("{}λ'\r{}", &pasted_text[..from], &pasted_text[to..]);
    assert_eq!(text(&s), replaced, "a quote typed after text in a value is that one character");
    // Undo restores each step exactly.
    undo_to(&mut s, &pasted_text);
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), original, "the paste is one step");
    s.dispatch(Command::Redo { tab });
    assert_eq!(text(&s), pasted_text);
    // Save (format on save is on for the project) writes the draft's bytes, unformatted.
    assert_eq!(s.disabled(CommandId::FormatDocument), Some("Environment files go to no language server: their values stay private data"));
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&path).unwrap(), pasted_text.as_bytes());
    for ran in ["ran", "ticked", "x"] { assert!(!root.join(ran).exists() && !d.path().join(ran).exists(), "{ran}: nothing in a value is run"); }
}

/// A sentinel secret in an environment file stays out of every route a document normally takes: no language server
/// wants it, the declaration index skips it, bookmarks and recent places save no line text, settings and the
/// workspace checkpoint hold none, Server Details and the language actions' reasons name none. The only copy outside
/// the project is the unsaved draft's recovery record, in the private (0700) recovery folder as a 0600 file.
#[test]
fn an_environment_files_text_reaches_no_server_index_place_setting_or_details() {
    const SECRET: &str = "SENTINEL_hunter2_7f3a";
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    write(&root, "main.rs", b"fn main() {}\n");
    let path = write(&root, ".env", format!("USER=app\nDB_PASSWORD={SECRET}\n# old {SECRET}\n").as_bytes());
    // No provider serves environment files, so no server is started for one or sent its text.
    assert!(crate::semantics::provider::PROVIDERS.iter().all(|p| !p.languages.contains(&Language::Env)));
    assert!(crate::semantics::provider::for_language(Language::Env).is_none());
    assert!(!crate::index::DeclIndex::wants(".env") && !crate::index::DeclIndex::wants("config/app.env"), "the saved declaration index skips them");
    let mut s = ready(d.path(), &root, &path);
    let tab = s.state().active().unwrap().id();
    assert!(!s.state().semantics.wants_text(Some(Language::Env), false, Some(&root)));
    // A bookmark on the secret's line, a recent location, and an edit that moves the bookmark with its line.
    caret_at_line(&mut s, 1);
    s.dispatch(Command::ToggleBookmark);
    s.dispatch(Command::GoToLine(LineColumn { line: 3, column: None }));
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: format!("NEW={SECRET}\n").into() }], kind: EditKind::Replace });
    let marks = s.state().places().bookmarks();
    assert_eq!(marks.iter().map(|m| (m.line, &*m.anchor)).collect::<Vec<_>>(), [(2, "")], "the bookmark follows its line and keeps no text");
    s.dispatch(Command::Open(root.join("main.rs")));
    settle(&mut s, opened);
    assert!(s.state().places().recent_locations().iter().filter(|m| m.source.ends_with(".env")).all(|m| m.anchor.is_empty()));
    s.dispatch(Command::Open(path.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == path));
    assert!(s.state().semantics.desired().keys().all(|p| p != &path), "no server holds the file");
    // Details and reasons say what happens, without any of the text.
    let details = s.language_server_details();
    assert!(details.starts_with(crate::session::ENV_FILE_DETAILS), "{details}");
    assert!(!details.contains(SECRET) && !details.contains("DB_PASSWORD"), "{details}");
    for id in [CommandId::Definition, CommandId::Completion, CommandId::QuickFix, CommandId::FormatDocument, CommandId::FileSymbols] {
        assert!(s.disabled(id).is_some_and(|why| !why.contains(SECRET)), "{id:?}: {:?}", s.disabled(id));
    }
    assert!(s.state().notices().iter().chain(s.state().messages.iter()).all(|n| !n.contains(SECRET)));
    // Everything Lenz keeps outside the project: settings, places, the workspace checkpoint, caches, recovery.
    s.flush_recovery_for_test();
    s.dispatch(Command::SaveWorkspace);
    let _ = s.workspace_saved().recv_timeout(Duration::from_secs(5));
    drop(s);
    let outside: Vec<PathBuf> = holding(d.path(), SECRET).into_iter().filter(|p| !p.starts_with(&root)).collect();
    assert!(!outside.is_empty(), "the unsaved draft is recoverable");
    for file in &outside {
        assert!(file.components().any(|c| c.as_os_str() == "recovery"), "{} holds the secret outside the recovery folder", file.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o777, 0o600, "{}", file.display());
            assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert!(!file.to_string_lossy().contains(SECRET), "a record's name carries no text");
    }
    let places = places_file(d.path(), &root);
    assert!(std::fs::read_to_string(&places).is_ok_and(|p| p.contains("bookmarks")), "places were saved (without the text)");
    assert_eq!(std::fs::read(&path).unwrap(), format!("USER=app\nDB_PASSWORD={SECRET}\n# old {SECRET}\n").as_bytes());
}

/// A places file written before environment files kept no text may hold a line of one: it is dropped when read.
#[test]
fn saved_places_of_an_environment_file_drop_their_line_text_when_read() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir_all(&root).unwrap();
    write(&root, ".env", b"A=1\nTOKEN=old-secret\n");
    write(&root, "a.rs", b"x\nfn f() {}\n");
    let mut s = session(d.path());
    open_project(&mut s, &root, "a.rs");
    caret_at_line(&mut s, 1);
    s.dispatch(Command::ToggleBookmark);
    drop(s);
    let file = places_file(d.path(), &root);
    let mut saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved["bookmarks"][0]["anchor"], "fn f() {}");
    saved["bookmarks"][0]["path"] = ".env".into();
    saved["bookmarks"][0]["anchor"] = "TOKEN=old-secret".into();
    std::fs::write(&file, saved.to_string()).unwrap();
    let env = normalized(&root.join(".env"));
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, |s| !s.state().places().bookmarks().is_empty());
    assert_eq!(s.state().places().bookmarks().iter().map(|m| (m.source.to_path_buf(), m.line, m.anchor.to_string())).collect::<Vec<_>>(), [(env, 1, String::new())]);
    drop(s);
    assert!(!std::fs::read_to_string(&file).unwrap().contains("old-secret"), "the next save keeps no text");
}
