use super::*;

#[gpui::test]
fn server_install_menu_opens_preview_and_keeps_document_input_modal(cx: &mut TestAppContext) {
    let f = fixture_providers(cx, Some(("script.sh", b"echo unchanged\n")), &[("bash-language-server", Path::new(""), Some("/nonexistent"), None)]);
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(350.0)));
    v.simulate_keystrokes("f2");
    let at = f.view.read_with(&v, |r, _| r.server_rows().iter().position(|(_, id)| *id == Some(CommandId::InstallServer)).unwrap());
    for _ in 0..at { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::StatusDetails { title: "Install language server", .. }))));
    let text = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>().join(""));
    assert!(text.contains("bash-language-server@5.8.1") && text.contains("Node.js"), "{text}");
    v.simulate_keystrokes("s ctrl-a ctrl-s f5");
    assert!(f.view.read_with(&v, |r, _| !r.session().state().server_install().unwrap().running));
    for (w,h) in [(170.0,90.0),(1.0,1.0),(700.0,350.0)] { v.simulate_resize(size(px(w),px(h))); v.simulate_keystrokes("end up"); draw(&mut v); }
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session().state().server_install().is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("script.sh")).unwrap(), b"echo unchanged\n");
}

#[cfg(unix)]
#[gpui::test]
fn server_install_start_progress_and_cancel_use_real_owned_processes(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let tools=tempfile::tempdir().unwrap();
    for (name,script) in [("node","#!/bin/sh\nif [ \"$1\" = --version ]; then echo v24.0.0; else exec \"$@\"; fi\n"),("npm","#!/bin/sh\necho FIXTURE_INSTALL_PROGRESS\nexec /bin/sleep 30\n")] {
        let path=tools.path().join(name);std::fs::write(&path,script).unwrap();std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let search=format!("{}:/usr/bin:/bin",tools.path().display());
    let f=fixture_providers(cx,Some(("script.sh",b"echo unchanged\n")),&[("bash-language-server",Path::new(""),Some(&search),None)]);
    settle(cx,&f,|r|r.session().state().current_source().is_some());
    let providers=f.dir.path().join("config/lenz-code/providers.json");let before=std::fs::read(&providers).unwrap();
    let mut v=visual(cx,&f);v.simulate_resize(size(px(900.0),px(500.0)));v.simulate_keystrokes("f2");
    let at=f.view.read_with(&v,|r,_|r.server_rows().iter().position(|(_,id)|*id==Some(CommandId::InstallServer)).unwrap());
    for _ in 0..at {v.simulate_keystrokes("down");}v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v,|r,_|!r.session().state().server_install().unwrap().running));
    v.simulate_keystrokes("s");assert!(f.view.read_with(&v,|r,_|r.session().state().server_install().unwrap().running));
    settle(cx,&f,|r|r.session().state().server_install().unwrap().output.contains("FIXTURE_INSTALL_PROGRESS"));
    draw(&mut v);
    assert!(f.view.read_with(&v,|r,_|r.details_labels(Details::Install).contains(&CommandId::CancelServerInstall.label())));
    assert!(f.view.read_with(&v,|r,_|r.modal_rows().into_iter().map(|r|r.0).collect::<String>().contains("FIXTURE_INSTALL_PROGRESS")));
    v.simulate_keystrokes("d");
    assert!(f.view.read_with(&v,|r,_|matches!(r.modal,Some(Modal::StatusDetails {title:"Server details",..}))));
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v,|r,_|matches!(r.modal,Some(Modal::StatusDetails {kind:Details::Install,..}))));
    v.simulate_keystrokes("ctrl-s c");
    settle(cx,&f,|r|r.session().state().server_install().unwrap().finished);
    assert!(f.view.read_with(&v,|r,_|r.session().state().server_install().unwrap().message.contains("cancelled")));
    assert_eq!(std::fs::read(providers).unwrap(),before);
    v.simulate_keystrokes("escape");assert!(f.view.read_with(&v,|r,_|r.modal.is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("script.sh")).unwrap(),b"echo unchanged\n");
}
