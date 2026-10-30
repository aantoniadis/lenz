use super::*;
use crate::keymap::KeyPress;

fn vim_fixture(text: &str) -> (tempfile::TempDir, Session, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "vim.txt", text.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "vim.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    s.dispatch(Command::SetVimEnabled(true));
    (d, s, path)
}
fn keys(s: &mut Session, keys: &str) {
    for c in keys.chars() {
        let key = KeyPress { key: c.to_string(), ctrl: false, alt: false, shift: false };
        assert!(s.vim_handles(&key), "{c}");
        s.dispatch(Command::VimKey(key));
    }
}
fn esc(s: &mut Session) {
    s.dispatch(Command::VimKey(KeyPress::new("escape", false, false, false).unwrap()));
}
fn head(s: &Session) -> usize {
    s.state().selection().unwrap().head
}

#[test]
fn vim_modes_motions_and_insert_preserve_disk_and_undo() {
    let (_d, mut s, path) = vim_fixture("one two\r\nthree\r\n");
    keys(&mut s, "wi");
    assert_eq!(head(&s), 4);
    assert_eq!(s.state().vim_mode(), VimMode::Insert);
    s.dispatch(Command::InsertText { text: "λ".into(), kind: crate::text::EditKind::Typing });
    esc(&mut s);
    assert_eq!(s.state().vim_mode(), VimMode::Normal);
    assert_eq!(head(&s), 4);
    assert_eq!(source_text(s.state().current_source().unwrap()), "one λtwo\r\nthree\r\n");
    keys(&mut s, "u");
    assert_eq!(source_text(s.state().current_source().unwrap()), "one two\r\nthree\r\n");
    assert_eq!(std::fs::read(path).unwrap(), b"one two\r\nthree\r\n");
}
#[test]
fn vim_counts_operators_register_and_visual_are_undoable() {
    let (_d, mut s, _) = vim_fixture("α one\r\nβ two\r\nγ three");
    keys(&mut s, "2dd");
    assert_eq!(source_text(s.state().current_source().unwrap()), "γ three");
    keys(&mut s, "P");
    assert_eq!(source_text(s.state().current_source().unwrap()), "α one\r\nβ two\r\nγ three");
    keys(&mut s, "ggvld");
    assert_eq!(source_text(s.state().current_source().unwrap()), "one\r\nβ two\r\nγ three");
    keys(&mut s, "u");
    assert_eq!(source_text(s.state().current_source().unwrap()), "α one\r\nβ two\r\nγ three");
}
#[test]
fn relative_line_numbers_use_each_tabs_caret_and_persist() {
    let (d, mut s, path) = vim_fixture("a\nb\nc\nd\ne");
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
    s.dispatch(Command::SetRelativeLineNumbers(true));
    assert_eq!((0..5).map(|l| s.state().line_number(tab, l)).collect::<Vec<_>>(), vec![2, 1, 3, 1, 2]);
    s.dispatch(Command::Split(Split::Columns));
    let other = s.state().active().unwrap().id();
    s.dispatch(Command::Select(Selection { anchor: 8, head: 8 }));
    assert_eq!(s.state().line_number(tab, 0), 2);
    assert_eq!(s.state().line_number(other, 0), 4);
    let settings = s.settings_file.clone();
    settle(&mut s, |_| {
        std::fs::read(&settings)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| v["relative_line_numbers"] == true)
    });
    assert!(session(d.path()).state().relative_line_numbers());
    assert_eq!(std::fs::read(path).unwrap(), b"a\nb\nc\nd\ne");
}

#[test]
fn vim_replace_digit_and_counted_word_change_do_not_cross_unrelated_lines() {
    let (_d, mut s, _) = vim_fixture("a one two\r\nlast");
    keys(&mut s, "r2");
    assert_eq!(source_text(s.state().current_source().unwrap()), "2 one two\r\nlast");
    keys(&mut s, "u0cw");
    s.dispatch(Command::InsertText { text: "λ".into(), kind: crate::text::EditKind::Typing });
    esc(&mut s);
    assert_eq!(source_text(s.state().current_source().unwrap()), "λ one two\r\nlast");
}
#[test]
fn vim_last_line_delete_paste_empty_line_and_read_only_are_safe() {
    let (_d, mut s, path) = vim_fixture("α\r\nβ");
    keys(&mut s, "Gdd");
    assert_eq!(source_text(s.state().current_source().unwrap()), "α");
    keys(&mut s, "p");
    assert_eq!(source_text(s.state().current_source().unwrap()), "α\r\nβ");
    keys(&mut s, "u");
    s.dispatch(Command::SetReadOnly(true));
    keys(&mut s, "ddp");
    assert_eq!(source_text(s.state().current_source().unwrap()), "α");
    assert_eq!(std::fs::read(path).unwrap(), "α\r\nβ".as_bytes());
    let (_d, mut s, _) = vim_fixture("a\n");
    keys(&mut s, "Gdd");
    assert_eq!(source_text(s.state().current_source().unwrap()), "a");
    keys(&mut s, "p");
    assert_eq!(source_text(s.state().current_source().unwrap()), "a\n");
}
#[test]
fn vim_pending_operators_cannot_edit_a_different_tab() {
    let (d, mut s, path) = vim_fixture("first");
    keys(&mut s, "d");
    let second = write(d.path(), "second.txt", b"second");
    open(&mut s, Command::OpenRetained(second.clone()), "second.txt");
    keys(&mut s, "w");
    assert_eq!(source_text(s.state().current_source().unwrap()), "second");
    assert_eq!(s.state().vim_mode(), VimMode::Normal);
    assert_eq!(std::fs::read(path).unwrap(), b"first");
    assert_eq!(std::fs::read(second).unwrap(), b"second");
}
#[test]
fn vim_colon_save_close_and_dirty_quit_use_existing_safety_flow() {
    let (_d, mut s, path) = vim_fixture("saved\n");
    keys(&mut s, "i");
    s.dispatch(Command::InsertText { text: "draft ".into(), kind: crate::text::EditKind::Typing });
    esc(&mut s);
    keys(&mut s, ":q");
    s.dispatch(Command::VimKey(KeyPress::new("enter", false, false, false).unwrap()));
    assert!(s.close_prompt().is_some());
    assert_eq!(std::fs::read(&path).unwrap(), b"saved\n");
    let prompt = s.close_prompt().unwrap();
    s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Cancel });
    keys(&mut s, ":wq");
    s.dispatch(Command::VimKey(KeyPress::new("enter", false, false, false).unwrap()));
    settle(&mut s, |s| s.state().tabs().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), b"draft saved\n");
}

#[test]
fn vim_insert_change_and_open_line_each_undo_and_redo_as_one_group() {
    for command in ["cw", "o", "O"] {
        let (_d, mut s, _) = vim_fixture("α one\r\nlast");
        keys(&mut s, command);
        s.dispatch(Command::InsertText { text: "λ".into(), kind: crate::text::EditKind::Typing });
        s.dispatch(Command::Delete { backward: true });
        s.dispatch(Command::InsertText { text: "new".into(), kind: crate::text::EditKind::Paste });
        esc(&mut s);
        let changed = source_text(s.state().current_source().unwrap());
        keys(&mut s, "u");
        assert_eq!(source_text(s.state().current_source().unwrap()), "α one\r\nlast", "{command}");
        s.dispatch(Command::VimKey(KeyPress::new("r", true, false, false).unwrap()));
        assert_eq!(source_text(s.state().current_source().unwrap()), changed, "redo {command}");
    }
}

#[test]
fn vim_paused_typing_and_paste_group_but_save_and_tab_changes_end_it() {
    use std::sync::atomic::AtomicU64;
    let d = tempfile::tempdir().unwrap();
    let path = write(d.path(), "paused.txt", b"");
    let millis = Arc::new(AtomicU64::new(0));
    let timer = millis.clone();
    let now = std::time::Instant::now();
    let mut s = session_clock(d.path(), Arc::new(move || now + Duration::from_millis(timer.load(Ordering::Relaxed))));
    open(&mut s, Command::OpenRetained(path.clone()), "paused.txt");
    s.enable_text_input_for_test();
    s.dispatch(Command::SetVimEnabled(true));
    keys(&mut s, "i");
    s.dispatch(Command::InsertText { text: "α".into(), kind: crate::text::EditKind::Typing });
    millis.store(2000, Ordering::Relaxed);
    s.dispatch(Command::InsertText { text: "β".into(), kind: crate::text::EditKind::Typing });
    s.dispatch(Command::Paste("γ".into()));
    esc(&mut s);
    keys(&mut s, "u");
    assert_eq!(source_text(s.state().current_source().unwrap()), "");
    s.dispatch(Command::VimKey(KeyPress::new("r", true, false, false).unwrap()));
    assert_eq!(source_text(s.state().current_source().unwrap()), "αβγ");
    keys(&mut s, "A");
    s.dispatch(Command::InsertText { text: "saved".into(), kind: crate::text::EditKind::Typing });
    let tab = s.state().active().unwrap().id();
    s.dispatch(Command::Save { tab });
    settle(&mut s, |s| !s.state().view(tab).unwrap().dirty());
    let saved = source_text(s.state().current_source().unwrap());
    s.dispatch(Command::InsertText { text: "after".into(), kind: crate::text::EditKind::Typing });
    esc(&mut s);
    keys(&mut s, "u");
    assert_eq!(source_text(s.state().current_source().unwrap()), saved);
    keys(&mut s, "i");
    let next = write(d.path(), "next.txt", b"next");
    open(&mut s, Command::OpenRetained(next), "next.txt");
    assert_eq!(s.state().vim_mode(), VimMode::Normal);
    assert!(!s.vim_insert_allowed());
    keys(&mut s, "j");
    assert_eq!(source_text(s.state().current_source().unwrap()), "next");
}

#[test]
fn vim_insert_tab_and_enter_join_the_insert_undo_group() {
    let (_d, mut s, _)=vim_fixture("original\r\n");keys(&mut s,"i");
    s.dispatch(Command::InsertText{text:"a".into(),kind:crate::text::EditKind::Typing});
    s.dispatch(Command::Indent{outdent:false});
    s.dispatch(Command::InsertText{text:"\r\n".into(),kind:crate::text::EditKind::Typing});
    s.dispatch(Command::InsertText{text:"b".into(),kind:crate::text::EditKind::Typing});esc(&mut s);
    let changed=source_text(s.state().current_source().unwrap());keys(&mut s,"u");assert_eq!(source_text(s.state().current_source().unwrap()),"original\r\n");
    s.dispatch(Command::VimKey(KeyPress::new("r",true,false,false).unwrap()));assert_eq!(source_text(s.state().current_source().unwrap()),changed);
}

#[test]
fn vim_typing_while_save_is_pending_keeps_each_save_boundary() {
    let (_d,mut s,path)=vim_fixture("");keys(&mut s,"i");
    s.dispatch(Command::InsertText{text:"a".into(),kind:crate::text::EditKind::Typing});let tab=s.state().active().unwrap().id();
    s.dispatch(Command::Save{tab});
    s.dispatch(Command::InsertText{text:"b".into(),kind:crate::text::EditKind::Typing});
    settle(&mut s,|s|!s.state().view(tab).unwrap().save_status().busy());
    assert_eq!(std::fs::read(&path).unwrap(),b"a");assert_eq!(source_text(s.state().current_source().unwrap()),"ab");
    s.dispatch(Command::InsertText{text:"c".into(),kind:crate::text::EditKind::Typing});esc(&mut s);
    keys(&mut s,"u");assert_eq!(source_text(s.state().current_source().unwrap()),"ab");
    keys(&mut s,"u");assert_eq!(source_text(s.state().current_source().unwrap()),"a");
    assert_eq!(std::fs::read(path).unwrap(),b"a");
}

#[test]
fn vim_force_quit_with_spaces_discards_without_saving() {
    let (_d, mut s, path) = vim_fixture("saved\n");
    keys(&mut s, "i");
    s.dispatch(Command::InsertText { text: "draft ".into(), kind: crate::text::EditKind::Typing });
    esc(&mut s);
    keys(&mut s, ": q! ");
    s.dispatch(Command::VimKey(KeyPress::new("enter", false, false, false).unwrap()));
    settle(&mut s, |s| s.state().tabs().is_empty());
    assert_eq!(std::fs::read(path).unwrap(), b"saved\n");
}
