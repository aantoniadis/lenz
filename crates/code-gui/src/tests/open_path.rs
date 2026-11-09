use super::*;

#[gpui::test]
fn picker_failure_offers_editable_path_validation_cancel_and_open(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |v| v.session.state().current_source().is_some());
    let target = f.dir.path().join("with spaces é.txt");
    std::fs::write(&target, "target\n").unwrap();
    let mut visual = visual(cx, &f);
    f.view.update(&mut visual, |v, cx| {
        v.open_path_failed(code_core::open_path::Kind::File, cx)
    });
    visual.simulate_resize(size(px(400.), px(240.)));
    draw(&mut visual);
    assert!(f.view.read_with(&visual, |v, _| v.form_layout().is_some() && v.modal_layout().is_none()));
    assert!(f.view.read_with(&visual, |v, _| v.form_layout().unwrap().visible > 0));
    visual.simulate_keystrokes("ctrl-s x escape");
    assert!(f.view.read_with(&visual, |v, _| v.modal.is_none()));
    assert_eq!(
        f.view.read_with(&visual, |v, _| {
            let source = v.session.state().current_source().unwrap();
            source.copy_range(0..source.len_bytes()).unwrap()
        }),
        "original\n"
    );
    f.view.update(&mut visual, |v, cx| {
        v.open_path_failed(code_core::open_path::Kind::File, cx)
    });
    visual.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("/missing/path".into())));
    visual.simulate_keystrokes("ctrl-v enter");
    assert!(f.view.read_with(&visual, |v, _| matches!(
        &v.modal,
        Some(Modal::OpenPath { error: Some(_), .. })
    )));
    visual.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(target.display().to_string()))
    });
    visual.simulate_keystrokes("ctrl-a ctrl-v home end enter");
    settle(cx, &f, |v| {
        v.session
            .state()
            .active()
            .is_some_and(|tab| tab.source() == target)
    });
    assert!(f.view.read_with(cx, |v, _| v.modal.is_none()));
    assert_eq!(
        std::fs::read(f.dir.path().join("original.txt")).unwrap(),
        b"original\n"
    );
    assert_eq!(std::fs::read(target).unwrap(), b"target\n");
}
