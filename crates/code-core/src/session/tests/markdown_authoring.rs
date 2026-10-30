//! Markdown authoring through the Session command seam: list continuation and link completion as undoable edits
//! with exact bytes on Save, a preview beside the source that follows the draft and drops obsolete renders, and
//! preview positions and links that lead to the current source.
use super::*;
use crate::actions::CatalogAction;
use crate::text::EditKind;

fn text(s: &Session) -> String { source_text(s.state().current_source().unwrap()) }

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

/// Enter as the adapters send it: the file's own line ending.
fn enter(s: &mut Session) {
    let newline = s.state().active().and_then(|t| s.state().view(t.id())).map_or("\n", |v| v.input_newline());
    s.dispatch(Command::InsertText { text: newline.into(), kind: EditKind::Typing });
}

fn head(s: &Session) -> usize { s.state().selection().unwrap().head }

fn run(s: &mut Session, id: CommandId) {
    let CatalogAction::Command(command) = s.action(id) else { panic!("{id:?} is not a command: {:?}", s.action(id)) };
    s.dispatch(command);
}

fn labels(s: &Session) -> Vec<String> { s.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect()).unwrap_or_default() }

fn choose(s: &mut Session, label: &str) {
    let row = s.state().completion().unwrap().rows().position(|r| r.label == label).unwrap_or_else(|| panic!("{label} in {:?}", labels(s)));
    s.dispatch(Command::SelectCompletion(row));
    s.dispatch(Command::AcceptCompletion);
}

/// The preview pane 1 shows (beside the source in pane 0).
fn beside(s: &Session) -> Option<Arc<crate::markdown::Preview>> {
    let tab = s.state().shown(1)?;
    match s.state().view(tab.id())?.rendering()? {
        Rendering::Preview(p) => Some(p.clone()),
        _ => None,
    }
}

/// Enter after a bullet, a number and a task continues the list; Enter on the empty item ends it; a new number
/// moves the following items up; each is one Undo step, and Save writes exactly the draft (CRLF kept).
#[test]
fn enter_continues_and_ends_markdown_lists_as_single_undo_steps() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(&root).unwrap();
    let original = "# List\r\n\r\n1. one\r\n2. two\r\n3. three\r\n\r\n- [x] done\r\n";
    let path = root.join("list.md");
    std::fs::write(&path, original).unwrap();
    let mut s = ready(d.path(), &root, &path);
    let tab = s.state().active().unwrap().id();
    // Enter after "1. one": a new "2. " and the following items renumbered, in one step.
    caret(&mut s, original.find("one").unwrap() + 3);
    enter(&mut s);
    assert_eq!(text(&s), "# List\r\n\r\n1. one\r\n2. \r\n3. two\r\n4. three\r\n\r\n- [x] done\r\n");
    assert_eq!(head(&s), "# List\r\n\r\n1. one\r\n2. ".len());
    typed(&mut s, "half");
    enter(&mut s);
    assert_eq!(text(&s), "# List\r\n\r\n1. one\r\n2. half\r\n3. \r\n4. two\r\n5. three\r\n\r\n- [x] done\r\n");
    // Enter on the empty item ends the list: its marker goes, and the numbers stay.
    enter(&mut s);
    assert_eq!(text(&s), "# List\r\n\r\n1. one\r\n2. half\r\n\r\n4. two\r\n5. three\r\n\r\n- [x] done\r\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "# List\r\n\r\n1. one\r\n2. half\r\n3. \r\n4. two\r\n5. three\r\n\r\n- [x] done\r\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "# List\r\n\r\n1. one\r\n2. half\r\n3. two\r\n4. three\r\n\r\n- [x] done\r\n", "the continuation and its renumbering are one step");
    // A task continues as an open task.
    let end = text(&s).len() - 2;
    caret(&mut s, end);
    enter(&mut s);
    typed(&mut s, "next");
    assert!(text(&s).ends_with("- [x] done\r\n- [ ] next\r\n"), "{:?}", text(&s));
    // Outside a list Enter stays a plain newline (and one typing step with what follows).
    caret(&mut s, "# List".len());
    enter(&mut s);
    assert!(text(&s).starts_with("# List\r\n\r\n\r\n1. one"));
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().tab_dirty(tab));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text(&s));
    assert_eq!(text(&s), "# List\r\n\r\n\r\n1. one\r\n2. half\r\n3. two\r\n4. three\r\n\r\n- [x] done\r\n- [ ] next\r\n");
}

/// Several carets continue their own items in one step (no renumbering then); a selection, read-only mode, a
/// code fence and other languages keep Enter's ordinary behaviour.
#[test]
fn list_continuation_respects_carets_selections_code_read_only_and_language() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("notes");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("a.md");
    std::fs::write(&path, "- a\n- b\n```\n- c\n```\n").unwrap();
    let txt = root.join("a.txt");
    std::fs::write(&txt, "- a\n").unwrap();
    let mut s = ready(d.path(), &root, &path);
    let tab = s.state().active().unwrap().id();
    caret(&mut s, 3);
    s.dispatch(Command::AddCursorBelow);
    enter(&mut s);
    assert_eq!(text(&s), "- a\n- \n- b\n- \n```\n- c\n```\n");
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "- a\n- b\n```\n- c\n```\n");
    s.dispatch(Command::CollapseSelections);
    // Inside the fence: a plain newline.
    caret(&mut s, "- a\n- b\n```\n- c".len());
    enter(&mut s);
    assert_eq!(text(&s), "- a\n- b\n```\n- c\n\n```\n");
    s.dispatch(Command::Undo { tab });
    // A selection is replaced by the newline.
    s.dispatch(Command::Select(Selection { anchor: 2, head: 3 }));
    enter(&mut s);
    assert_eq!(text(&s), "- \n\n- b\n```\n- c\n```\n");
    s.dispatch(Command::Undo { tab });
    // Read-only: nothing.
    run(&mut s, CommandId::ReadOnlyMode);
    caret(&mut s, 3);
    enter(&mut s);
    assert_eq!(text(&s), "- a\n- b\n```\n- c\n```\n");
    run(&mut s, CommandId::ReadOnlyMode);
    // Plain text has no lists.
    s.dispatch(Command::OpenRetained(txt.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == txt));
    caret(&mut s, 3);
    enter(&mut s);
    assert_eq!(text(&s), "- a\n\n");
}

/// `](` suggests the files and folders of the folder typed so far; a folder goes on into it; `#` after a file
/// suggests its headings (an open draft's unsaved ones), and `#` alone this draft's own unsaved headings. Each
/// acceptance is one Undo step; nothing is written until Save.
#[test]
fn link_destinations_complete_files_folders_and_headings_from_drafts() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("site");
    std::fs::create_dir_all(root.join("docs/img")).unwrap();
    std::fs::write(root.join("docs/guide.md"), "# Guide\n\n## Install it\n\n```\n# not a heading\n```\n\n## Install it\n").unwrap();
    std::fs::write(root.join("docs/img/logo.png"), b"png").unwrap();
    std::fs::write(root.join("docs/my notes.md"), "# Notes\n").unwrap();
    let path = root.join("docs/index.md");
    std::fs::write(&path, "# Index\n\n").unwrap();
    let mut s = ready(d.path(), &root, &path);
    let tab = s.state().active().unwrap().id();
    caret(&mut s, "# Index\n\n".len());
    typed(&mut s, "See [the guide](");
    assert_eq!(labels(&s), ["img/", "guide.md", "index.md", "my notes.md"]);
    typed(&mut s, "gu");
    assert_eq!(labels(&s), ["guide.md"]);
    s.dispatch(Command::CloseCompletion);
    for _ in 0..2 { s.dispatch(Command::Delete { backward: true }); }
    // A folder goes on into it.
    s.dispatch(Command::Complete);
    choose(&mut s, "img/");
    assert_eq!(text(&s), "# Index\n\nSee [the guide](img/");
    assert_eq!(labels(&s), ["logo.png"]);
    s.dispatch(Command::Undo { tab });
    assert_eq!(text(&s), "# Index\n\nSee [the guide](", "acceptance is one step");
    // A name needing escapes is inserted escaped.
    s.dispatch(Command::Complete);
    choose(&mut s, "my notes.md");
    assert_eq!(text(&s), "# Index\n\nSee [the guide](my%20notes.md");
    s.dispatch(Command::Undo { tab });
    // `#` after a file: its headings, with GitHub's anchors (a repeat numbered, code skipped), off the UI thread.
    typed(&mut s, "guide.md#");
    settle(&mut s, |s| s.state().completion().is_some());
    assert_eq!(labels(&s), ["guide", "install-it", "install-it-1"]);
    assert_eq!(s.state().completion().unwrap().rows().nth(1).unwrap().detail.as_deref(), Some("## Install it"));
    choose(&mut s, "install-it");
    typed(&mut s, ")");
    assert_eq!(text(&s), "# Index\n\nSee [the guide](guide.md#install-it)");
    // `#` alone: this draft's headings, the unsaved ones included.
    typed(&mut s, "\n\n## Draft Only Heading\n\n[back](#");
    settle(&mut s, |s| s.state().completion().is_some());
    assert_eq!(labels(&s), ["index", "draft-only-heading"]);
    typed(&mut s, "dr");
    assert_eq!(labels(&s), ["draft-only-heading"]);
    s.dispatch(Command::AcceptCompletion);
    assert!(text(&s).ends_with("[back](#draft-only-heading"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Index\n\n", "nothing is written before Save");
}

/// Another file's draft lends its unsaved headings; an answer for a caret or version that moved on is dropped, and
/// Escape keeps a late answer from opening the popup.
#[test]
fn link_heading_answers_for_an_old_caret_version_or_a_dismissed_popup_are_dropped() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("site");
    std::fs::create_dir_all(&root).unwrap();
    let other = root.join("other.md");
    std::fs::write(&other, "# Saved\n").unwrap();
    let path = root.join("a.md");
    std::fs::write(&path, "").unwrap();
    let mut s = ready(d.path(), &root, &other);
    caret(&mut s, 8);
    typed(&mut s, "\n## Unsaved\n");
    s.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == path));
    typed(&mut s, "[o](other.md#");
    settle(&mut s, |s| s.state().completion().is_some());
    assert_eq!(labels(&s), ["saved", "unsaved"]);
    // Escape, then the answer to the question it closed arrives: no popup.
    s.dispatch(Command::CloseCompletion);
    s.dispatch(Command::Complete);
    s.dispatch(Command::CloseCompletion);
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < until { if let Ok(c) = rx.try_recv() { s.dispatch(c); } }
    assert!(s.state().completion().is_none(), "a dismissed popup is not reopened by its late answer");
    // An answer for a request superseded by typing is dropped too.
    let request = s.link_seq.load(Ordering::Relaxed);
    s.dispatch(Command::Complete);
    typed(&mut s, "s");
    let stale = vec![crate::markdown_authoring::Heading { level: 1, text: "Stale".into(), anchor: "stale".into(), line: 0 }];
    s.dispatch(Completion::LinkHeadings { request, headings: Ok(stale) });
    assert!(!labels(&s).contains(&"stale".to_owned()));
}

/// Open Preview to the Side: the source keeps focus, the preview beside follows each edit (the last preview stays
/// shown while the next is made, never Pending), and a render of an obsolete version is rejected.
#[test]
fn a_preview_beside_the_source_follows_the_draft_and_rejects_obsolete_renders() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("site");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("a.md");
    std::fs::write(&path, "# Title\n\nFirst.\n").unwrap();
    let mut s = ready(d.path(), &root, &path);
    assert_eq!(s.disabled(CommandId::ShowInSource), Some("Only a preview is shown in its source"));
    assert_eq!(s.disabled(CommandId::PreviewBeside), None);
    run(&mut s, CommandId::PreviewBeside);
    assert_eq!(s.state().split(), Some(Split::Columns));
    assert_eq!(s.state().focused_pane(), 0);
    assert!(s.state().active().unwrap().markdown_source(), "the source keeps focus");
    settle(&mut s, |s| beside(s).is_some());
    assert_eq!(s.disabled(CommandId::PreviewBeside), Some("The other pane shows its preview already"));
    let first = beside(&s).unwrap();
    assert!(first.doc.text.contains("First."));
    // Typing in the source: the old preview stays until the new version's lands.
    let end = text(&s).len();
    caret(&mut s, end);
    typed(&mut s, "\nSecond *line*.\n");
    let preview_tab = s.state().shown(1).unwrap().id();
    assert!(matches!(s.state().view(preview_tab).unwrap().rendering(), Some(Rendering::Preview(_))), "never Pending while typing");
    settle(&mut s, |s| beside(s).is_some_and(|p| p.doc.text.contains("Second line.")));
    let current = beside(&s).unwrap();
    assert!(s.state().view(preview_tab).unwrap().preview_current());
    // A render of an older version arriving late is rejected.
    let stale = crate::markdown::render("# Stale\n".into(), || true).unwrap();
    s.dispatch(Completion::Rendered { version: first.version, preview: Ok(stale) });
    assert!(Arc::ptr_eq(&beside(&s).unwrap(), &current));
    // Undo is followed too.
    let tab = s.state().active().unwrap().id();
    for _ in 0..4 { if text(&s) != "# Title\n\nFirst.\n" { s.dispatch(Command::Undo { tab }); } }
    assert_eq!(text(&s), "# Title\n\nFirst.\n");
    settle(&mut s, |s| beside(s).is_some_and(|p| !p.doc.text.contains("Second")));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Title\n\nFirst.\n");
}

/// A position chosen in the preview maps to the current source, also when the preview is one edit behind; a
/// `#heading` link goes to the draft's (unsaved) heading; a link to another file opens it at its heading; a web
/// link is never opened.
#[test]
fn preview_positions_and_links_lead_to_the_current_source() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("site");
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/other.md"), "# Other\n\ntext\n\n## Far Part\n").unwrap();
    let path = root.join("a.md");
    let original = "# Title\n\nSee [part](#later), [other](docs/other.md#far-part), [web](https://example.com).\n\nTarget word.\n\n## Later\n";
    std::fs::write(&path, original).unwrap();
    let mut s = ready(d.path(), &root, &path);
    run(&mut s, CommandId::PreviewBeside);
    settle(&mut s, |s| beside(s).is_some());
    let p = beside(&s).unwrap();
    let preview_tab = s.state().shown(1).unwrap().id();
    // Insert text above the target in the source; the preview has not caught up.
    caret(&mut s, "# Title\n".len());
    typed(&mut s, "\nNew line.\n");
    assert!(Arc::ptr_eq(&beside(&s).unwrap(), &p) && !s.state().view(preview_tab).unwrap().preview_current());
    // Choose "word" in the (old) preview and show it in the source: the caret lands on "word" of the current text.
    s.dispatch(Command::FocusPane(1));
    let rendered = p.doc.text.find("word").unwrap() + 1;
    s.dispatch(Command::PointPreview(PreviewPoint { tab: preview_tab, version: p.version, rendered }));
    assert_eq!(s.disabled(CommandId::FollowLink), Some("Click a link in the preview first"));
    run(&mut s, CommandId::ShowInSource);
    assert_eq!(s.state().focused_pane(), 0, "the source beside takes focus");
    assert_eq!(head(&s), text(&s).find("word").unwrap() + 1);
    // `#later` (Ctrl+click): the heading in the current draft.
    settle(&mut s, |s| s.state().view(preview_tab).is_some_and(|v| v.preview_current()));
    let p = beside(&s).unwrap();
    s.dispatch(Command::FocusPane(1));
    let rendered = p.doc.text.find("part").unwrap();
    s.dispatch(Command::ActivatePreview(PreviewPoint { tab: preview_tab, version: p.version, rendered }));
    settle(&mut s, |s| s.state().focused_pane() == 0);
    assert_eq!(head(&s), text(&s).find("## Later").unwrap());
    // A web link: refused, said so, nothing opened.
    s.dispatch(Command::FocusPane(1));
    let rendered = p.doc.text.find("web").unwrap();
    s.dispatch(Command::PointPreview(PreviewPoint { tab: preview_tab, version: p.version, rendered }));
    assert_eq!(s.disabled(CommandId::FollowLink), None);
    run(&mut s, CommandId::FollowLink);
    assert!(s.state.messages.iter().any(|m| m == "Lenz does not open web links: https://example.com"), "{:?}", s.state.messages);
    assert_eq!(s.state().active().unwrap().id(), preview_tab);
    // Another file, at its heading.
    let rendered = p.doc.text.find("other").unwrap();
    s.dispatch(Command::PointPreview(PreviewPoint { tab: preview_tab, version: p.version, rendered }));
    run(&mut s, CommandId::FollowLink);
    let other = root.join("docs/other.md");
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == other) && s.state().selection().is_some_and(|c| c.head > 0));
    assert_eq!(head(&s), "# Other\n\ntext\n\n".len());
    // The preview's context menu offers both; the files are unchanged.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

/// Show in Source without a chosen position uses the reading line and switches an unsplit pane to the source; the
/// commands explain themselves elsewhere.
#[test]
fn show_in_source_switches_a_lone_preview_and_commands_explain_themselves() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("site");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("a.md");
    std::fs::write(&path, "# One\n\npara\n\n## Two\n\nmore\n").unwrap();
    let txt = root.join("t.txt");
    std::fs::write(&txt, "x\n").unwrap();
    let mut s = ready(d.path(), &root, &path);
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| matches!(s.state().rendering(), Some(Rendering::Preview(_))));
    let menu = s.context_menu();
    assert!(menu.contains(&MenuEntry::Command(CommandId::ShowInSource)) && menu.contains(&MenuEntry::Command(CommandId::FollowLink)));
    assert_eq!(s.disabled(CommandId::PreviewBeside), Some("This pane shows the preview; Show in Source opens its source"));
    s.dispatch(Command::Scroll(4));
    run(&mut s, CommandId::ShowInSource);
    assert!(matches!(s.state().rendering(), Some(Rendering::Source)));
    assert_eq!(head(&s), "# One\n\npara\n\n".len());
    assert!(s.context_menu().contains(&MenuEntry::Command(CommandId::PreviewBeside)));
    s.dispatch(Command::OpenRetained(txt.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == txt));
    for id in [CommandId::PreviewBeside, CommandId::ShowInSource, CommandId::FollowLink] { assert_eq!(s.disabled(id), Some("Not a Markdown file")); }
}

/// A followed link whose target answers after focus moved to another file goes nowhere; one whose source took focus
/// still lands there.
#[test]
fn a_followed_link_answering_after_focus_moved_on_is_dropped() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("site");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("other.md"), "# Other\n\n## Part\n").unwrap();
    let path = root.join("a.md");
    std::fs::write(&path, "[o](other.md#part) [h](#here)\n\n## Here\n").unwrap();
    let txt = root.join("t.txt");
    std::fs::write(&txt, "x\n").unwrap();
    let mut s = ready(d.path(), &root, &path);
    run(&mut s, CommandId::PreviewBeside);
    settle(&mut s, |s| beside(s).is_some());
    let p = beside(&s).unwrap();
    let preview_tab = s.state().shown(1).unwrap().id();
    s.dispatch(Command::FocusPane(1));
    let rendered = p.doc.text.find('o').unwrap();
    s.dispatch(Command::ActivatePreview(PreviewPoint { tab: preview_tab, version: p.version, rendered }));
    // Another file is opened in the preview's pane before the heading is read: the link is not followed.
    s.dispatch(Command::OpenRetained(txt.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == txt));
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < until { if let Ok(c) = rx.try_recv() { s.dispatch(c); } }
    assert!(s.state().current_source().is_some_and(|d| d.source() == txt), "other.md was not opened over the newer choice");
    // `#here` answered after the source beside took focus: the caret lands in that source.
    s.dispatch(Command::Close);
    settle(&mut s, |s| s.state().shown(1).is_some_and(|t| t.id() == preview_tab) && beside(s).is_some());
    let p = beside(&s).unwrap();
    s.dispatch(Command::FocusPane(1));
    let rendered = p.doc.text.find('h').unwrap();
    s.dispatch(Command::ActivatePreview(PreviewPoint { tab: preview_tab, version: p.version, rendered }));
    s.dispatch(Command::FocusPane(0));
    settle(&mut s, |s| s.state().selection().is_some_and(|c| c.head == "[o](other.md#part) [h](#here)\n\n".len()));
    assert_eq!(s.state().focused_pane(), 0);
}

/// Found by this ticket (failing on the integration branch before it): a preview shown again for the version it was
/// last made for (its tab released while another file was shown in its pane) is made again, not taken by the
/// analysis workers for a duplicate request and left preparing forever.
#[test]
fn a_preview_shown_again_after_its_tab_was_released_is_made_again() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("site");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("a.md");
    std::fs::write(&path, "# A\n\ntext\n").unwrap();
    let txt = root.join("t.txt");
    std::fs::write(&txt, "x\n").unwrap();
    let mut s = ready(d.path(), &root, &path);
    s.dispatch(Command::Split(Split::Columns));
    s.dispatch(Command::ToggleMarkdownSource);
    settle(&mut s, |s| matches!(s.state().rendering(), Some(Rendering::Preview(_))));
    s.dispatch(Command::OpenRetained(txt.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == txt));
    s.dispatch(Command::Close);
    settle(&mut s, |s| matches!(s.state().rendering(), Some(Rendering::Preview(_))));
}
