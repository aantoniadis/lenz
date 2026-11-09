//! Replay archive journeys through the terminal's ordinary constructor, keys and drawing.
use super::*;
use code_core::archive::Location;

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 { crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1)); }
    }
    !crc
}

fn zip_record(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(0x0403_4b50u32.to_le_bytes());
    for word in [10u16, 0, 0, 0, 0x21] { out.extend(word.to_le_bytes()); }
    for word in [crc32(bytes), bytes.len() as u32, bytes.len() as u32] { out.extend(word.to_le_bytes()); }
    for word in [name.len() as u16, 0] { out.extend(word.to_le_bytes()); }
    out.extend(name.as_bytes()); out.extend(bytes); out
}

/// Small stored ZIP fixtures need no shell process or additional adapter dependency.
fn zip(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut out = Vec::new(); let mut central = Vec::new();
    for &(name, bytes, mode) in entries {
        let offset = out.len() as u32;
        out.extend(zip_record(name, bytes));
        central.extend(0x0201_4b50u32.to_le_bytes());
        for word in [0x0314u16, 10, 0, 0, 0, 0x21] { central.extend(word.to_le_bytes()); }
        for word in [crc32(bytes), bytes.len() as u32, bytes.len() as u32] { central.extend(word.to_le_bytes()); }
        for word in [name.len() as u16, 0, 0, 0, 0] { central.extend(word.to_le_bytes()); }
        central.extend((mode << 16).to_le_bytes()); central.extend(offset.to_le_bytes()); central.extend(name.as_bytes());
    }
    let offset = out.len() as u32; let size = central.len() as u32;
    out.extend(central); out.extend(0x0605_4b50u32.to_le_bytes());
    for word in [0u16, 0, entries.len() as u16, entries.len() as u16] { out.extend(word.to_le_bytes()); }
    out.extend(size.to_le_bytes()); out.extend(offset.to_le_bytes());
    let comment = b"TUI archive metadata";
    out.extend((comment.len() as u16).to_le_bytes()); out.extend(comment); out
}

fn tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for &(name, bytes) in entries {
        let mut header = [0u8; 512]; header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..108].copy_from_slice(b"0000640\0");
        header[108..116].copy_from_slice(b"0000123\0"); header[116..124].copy_from_slice(b"0000456\0");
        header[124..136].copy_from_slice(format!("{:011o}\0", bytes.len()).as_bytes());
        header[136..148].copy_from_slice(b"00000000001\0"); header[148..156].fill(b' ');
        header[156] = b'0'; header[257..263].copy_from_slice(b"ustar\0"); header[263..265].copy_from_slice(b"00");
        let checksum = header.iter().map(|b| u32::from(*b)).sum::<u32>();
        header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
        out.extend(header); out.extend(bytes); out.resize(out.len().next_multiple_of(512), 0);
    }
    out.resize(out.len() + 1024, 0); out
}

fn text(a: &App) -> String {
    let source = a.session.state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap()
}
fn selected(a: &App) -> String { a.session.state().tree_rows().nth(a.tree_at).unwrap().name.to_string() }
fn select(a: &mut App, name: &str) {
    let at = a.session.state().tree_rows().position(|row| row.name.as_ref() == name).unwrap_or_else(|| panic!("Missing archive row {name:?}"));
    assert_eq!(a.focus, Focus::Tree);
    while a.tree_at < at { key(a, KeyCode::Down, KeyModifiers::NONE); }
    while a.tree_at > at { key(a, KeyCode::Up, KeyModifiers::NONE); }
}
fn ready(a: &mut App, name: &str) {
    settle(a, |a| a.session.state().tree_rows().any(|row| row.name.as_ref() == name));
}
fn member(path: &Path, members: &[&str]) -> Vec<u8> {
    let (leaf, parents) = members.split_last().unwrap();
    let location = Location::new(path.to_owned(), parents.iter().map(|s| (*s).to_owned()).collect()).unwrap();
    let index = code_core::archive::open_index(&location, &|| false).unwrap();
    let entry = index.members().iter().find(|entry| entry.path.as_deref() == Some(*leaf)).unwrap();
    index.read_member(entry.id, &|| false).unwrap()
}

#[test]
fn archive_cli_member_edit_undo_save_keeps_siblings_and_metadata() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("editable.zip");
    let sibling = [0, 255, 1, 200, 42];
    let original = zip(&[("src/edit.txt", b"before\r\n", 0o100640), ("sibling.bin", &sibling, 0o100751)]);
    std::fs::write(&path, &original).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone())); a.enable_draft_input_for_test();
    ready(&mut a, "src"); assert_eq!(a.focus, Focus::Tree); assert!(a.session.state().tabs().is_empty());
    select(&mut a, "src"); key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    select(&mut a, "edit.txt"); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some()); draw(&mut a, 80, 18);
    assert_eq!(a.focus, Focus::Text); assert_eq!(text(&a), "before\r\n");
    let title = a.session.state().active().unwrap().title(); assert!(title.starts_with("edit.txt") && title.contains("editable.zip"));
    assert!(a.session.status_details().contains("editable.zip!/src/edit.txt"));
    assert!(!a.session.status_details().contains("lenz-archive://"));
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE); assert_eq!(text(&a), "xbefore\r\n");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL); assert_eq!(text(&a), "before\r\n");
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("after λ😀\r\n".into()));
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL); assert_eq!(text(&a), "before\r\n");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT); assert_eq!(text(&a), "after λ😀\r\n");
    assert_eq!(std::fs::read(&path).unwrap(), original, "editing and Undo never write the container");
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    settle(&mut a, |a| !a.session.state().tab_dirty(a.session.state().active().unwrap().id()));
    assert_eq!(member(&path, &["src/edit.txt"]), "after λ😀\r\n".as_bytes());
    assert_eq!(member(&path, &["sibling.bin"]), sibling);
    let saved = std::fs::read(&path).unwrap(); let untouched = zip_record("sibling.bin", &sibling);
    assert!(saved.windows(untouched.len()).any(|bytes| bytes == untouched), "untouched sibling record is byte-for-byte preserved");
    assert!(saved.ends_with(b"TUI archive metadata"));
}

#[test]
fn archive_open_prompt_nested_navigation_and_save_stay_in_the_container() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("nested.zip");
    let inner = zip(&[("leaf.txt", b"inner", 0o100644), ("keep.txt", b"keep inner", 0o100644)]);
    std::fs::write(&path, zip(&[("inner.zip", &inner, 0o100644), ("outer.txt", b"keep outer", 0o100644)])).unwrap();
    let mut a = App::new(config(dir.path()), None); a.enable_draft_input_for_test();
    key(&mut a, KeyCode::Char('o'), KeyModifiers::CONTROL); a.handle(Event::Paste(path.display().to_string()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE); ready(&mut a, "inner.zip");
    select(&mut a, "inner.zip"); key(&mut a, KeyCode::Right, KeyModifiers::NONE); ready(&mut a, "leaf.txt");
    select(&mut a, "leaf.txt"); key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(selected(&a), "inner.zip", "Left selects a virtual member's parent");
    select(&mut a, "leaf.txt"); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some()); draw(&mut a, 80, 18);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("changed inner".into()));
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    settle(&mut a, |a| !a.session.state().tab_dirty(a.session.state().active().unwrap().id()));
    assert_eq!(member(&path, &["inner.zip", "leaf.txt"]), b"changed inner");
    assert_eq!(member(&path, &["inner.zip", "keep.txt"]), b"keep inner");
    assert_eq!(member(&path, &["outer.txt"]), b"keep outer");
    assert!(!dir.path().join("leaf.txt").exists(), "no member is extracted into the working directory");
}

#[test]
fn archive_tar_member_saves_with_ordinary_editor_keys() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("editable.tar");
    let original = tar(&[("edit.txt", b"tar before\n"), ("keep.bin", &[0, 255, 42])]);
    std::fs::write(&path, &original).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone())); a.enable_draft_input_for_test();
    ready(&mut a, "edit.txt"); select(&mut a, "edit.txt"); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some()); draw(&mut a, 80, 18);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("tar after\n".into()));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    settle(&mut a, |a| !a.session.state().tab_dirty(a.session.state().active().unwrap().id()));
    assert_eq!(member(&path, &["edit.txt"]), b"tar after\n"); assert_eq!(member(&path, &["keep.bin"]), [0, 255, 42]);
    let saved = std::fs::read(&path).unwrap();
    assert_eq!(&saved[1024..2048], &original[1024..2048], "untouched TAR member header, ownership, mode, payload and padding remain exact");
}

#[test]
fn archive_compressed_formats_use_the_same_editor_save_journey() {
    use lenz_archive::Compression;
    for (extension, compression) in [("tar.gz", Compression::Gzip), ("tar.bz2", Compression::Bzip2), ("tar.xz", Compression::Xz), ("tar.zst", Compression::Zstd)] {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join(format!("editable.{extension}"));
        let original = lenz_archive::testing::tar(compression, &[("edit.txt", b"before"), ("keep.txt", b"keep")]);
        std::fs::write(&path, &original).unwrap();
        let mut a = App::new(config(dir.path()), Some(path.clone())); a.enable_draft_input_for_test();
        ready(&mut a, "edit.txt"); select(&mut a, "edit.txt"); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        settle(&mut a, |a| a.session.state().current_source().is_some()); draw(&mut a, 80, 18);
        key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("after".into()));
        assert_eq!(std::fs::read(&path).unwrap(), original);
        key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        settle(&mut a, |a| !a.session.state().tab_dirty(a.session.state().active().unwrap().id()));
        assert_eq!(member(&path, &["edit.txt"]), b"after", "{extension}"); assert_eq!(member(&path, &["keep.txt"]), b"keep", "{extension}");
    }
}

#[test]
fn archive_speed_search_enter_moves_keys_into_the_member_editor() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("search.zip");
    let original = zip(&[("edit.txt", b"search before", 0o100644), ("other.txt", b"keep", 0o100644)]);
    std::fs::write(&path, &original).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone())); a.enable_draft_input_for_test(); ready(&mut a, "edit.txt");
    for c in "edit".chars() { key(&mut a, KeyCode::Char(c), KeyModifiers::NONE); }
    assert_eq!(selected(&a), "edit.txt"); assert_eq!(a.focus, Focus::Tree);
    assert!(a.session.state().current_source().is_none(), "speed search selects without opening");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE); settle(&mut a, |a| a.session.state().current_source().is_some());
    assert_eq!(a.focus, Focus::Text); assert!(a.session.state().explorer_search().query().is_empty());
    draw(&mut a, 32, 8); assert!(a.layout.tree.is_empty(), "narrow terminal returns its width to the editor");
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("search after".into()));
    assert_eq!(text(&a), "search after"); assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn archive_breadcrumb_popup_opens_sibling_without_a_project() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("crumbs.zip");
    let original = zip(&[("a.txt", b"first", 0o100644), ("b.txt", b"second", 0o100644)]);
    std::fs::write(&path, &original).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone())); a.enable_draft_input_for_test();
    ready(&mut a, "a.txt"); select(&mut a, "a.txt"); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some()); draw(&mut a, 80, 18);
    a.run(CommandId::FocusBreadcrumbs); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Siblings { .. })));
    let at = a.choices().iter().position(|choice| choice.label == "b.txt").unwrap();
    while a.modal.as_ref().unwrap().at < at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    let screen = draw_screen(&mut a, 32, 8); assert!(!screen.contains("lenz-archive"));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|source| source.source_meta().archive().is_some_and(|location| location.member_name() == Some("b.txt"))));
    assert_eq!(text(&a), "second"); assert_eq!(a.focus, Focus::Text); assert!(a.modal.is_none());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn archive_recent_files_popup_searches_readable_names_and_reopens_the_member() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("recent.zip");
    std::fs::write(&path, zip(&[("edit.txt", b"recent text", 0o100644)])).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    ready(&mut a, "edit.txt"); select(&mut a, "edit.txt"); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.run(CommandId::RecentFiles); a.handle(Event::Paste("edit.txt".into())); navigation_ready(&mut a);
    // Navigation uses its own renderer over core rows; ordinary modal choices do not back this popup.
    let search = a.session.state().navigation().unwrap();
    assert_eq!(search.query, "edit.txt"); assert!(search.rows.iter().any(|row| row.label.contains("edit.txt")), "archive members are searchable by their readable name: {search:?}");
    assert!(search.rows.iter().all(|row| !row.label.contains("lenz-archive") && !row.detail.contains("lenz-archive")));
    let screen = draw_screen(&mut a, 32, 8); assert!(screen.contains("edit.txt")); assert!(!screen.contains("lenz-archive"));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE); assert!(a.modal.is_none());
    settle(&mut a, |a| a.session.state().current_source().is_some()); assert_eq!(text(&a), "recent text");
}

#[test]
fn archive_error_retry_empty_close_and_narrow_browser_have_a_keyboard_route() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("retry.zip");
    std::fs::write(&path, b"invalid").unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().tree_rows().any(|row| row.name.starts_with("Cannot open archive:")));
    let at = a.session.state().tree_rows().position(|row| row.name.starts_with("Cannot open archive:")).unwrap();
    while a.tree_at < at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    let screen = draw_screen(&mut a, 32, 8);
    assert!(screen.contains("Cannot open archive")); assert!(screen.contains("Enter retries")); assert_eq!(a.layout.tree.width, 32);
    let error = selected(&a);
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE); assert_eq!(a.focus, Focus::Tree, "a browser without an editor keeps useful focus");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE); assert_eq!(selected(&a), "retry.zip"); select(&mut a, &error);
    std::fs::write(&path, zip(&[])).unwrap(); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut a, "(Empty archive)"); assert_eq!(a.focus, Focus::Tree);
    assert!(draw_screen(&mut a, 32, 8).contains("Empty archive"));
    select(&mut a, "↑ Up / close archive"); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(!a.session.state().has_tree()); assert_eq!(a.focus, Focus::Text);
    assert!(draw_screen(&mut a, 32, 8).contains("Open File"));
}

#[test]
fn archive_loading_row_returns_selection_to_its_parent_after_listing() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("loading.zip");
    std::fs::write(&path, zip(&[("member.txt", b"archive member", 0o100644)])).unwrap();
    std::fs::write(dir.path().join("ordinary.txt"), b"project file").unwrap();
    let mut a = App::new(config(dir.path()), Some(dir.path().to_owned()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    a.dispatch(Command::Open(path));
    let archive_root = a.session.state().tree_rows().nth(a.tree_at).unwrap().path.clone();
    key(&mut a, KeyCode::Down, KeyModifiers::NONE); key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(selected(&a), "Loading archive…");
    ready(&mut a, "member.txt");
    assert_eq!(a.session.state().tree_rows().nth(a.tree_at).unwrap().path, archive_root, "a disappearing status row must not jump to an unrelated physical project file");
    assert_eq!(a.focus, Focus::Tree); assert!(a.session.state().current_source().is_none());
}

#[test]
fn archive_symlink_is_read_only_and_filesystem_actions_explain_why() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("links.zip");
    let original = zip(&[("link.txt", b"target.txt", 0o120777), ("target.txt", b"target", 0o100644)]);
    std::fs::write(&path, &original).unwrap();
    let mut a = App::new(config(dir.path()), Some(dir.path().to_owned())); a.enable_draft_input_for_test();
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| !p.refreshing()));
    a.dispatch(Command::Open(path.clone())); ready(&mut a, "link.txt");
    select(&mut a, "link.txt");
    for id in [CommandId::RenameExplorerEntry, CommandId::MoveExplorerEntry, CommandId::DeleteExplorerEntry] {
        a.remember_explorer(); assert!(a.session.disabled(id).is_some_and(|why| why.to_lowercase().contains("archive")), "{id:?} must explain why an archive member cannot be used as a disk path");
    }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE); settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 18); let before = text(&a);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("forbidden".into())); key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), before); assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(a.session.status_details().to_lowercase().contains("read-only"));
}
