use super::*;

fn menu(v: &mut VisualTestContext, f: &Fixture, command: CommandId) {
    v.simulate_keystrokes("f10");
    let row = f.view.read_with(v, |view, _| view.modal_rows().iter().position(|r| r.0 == command.label()).unwrap());
    for _ in 0..row { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
}

#[gpui::test]
fn dirty_external_choices_use_real_menus_input_and_saved_receipts(cx: &mut TestAppContext) {
    for dark in [true, false] {
        let f = fixture(cx, Some(("external.txt", b"saved\r\n")));
        let path = f.dir.path().join("external.txt");
        settle(cx, &f, |view| view.session.state().current_source().is_some());
        let mut v = visual(cx, &f);
        f.view.update(&mut v, |view, cx| {
            view.enable_draft_input_for_test();
            view.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(if dark { "darcula" } else { "github-light" }).unwrap())), cx);
        });
        v.write_to_clipboard(ClipboardItem::new_string("draft λ\r\n".into()));
        v.simulate_keystrokes("ctrl-a ctrl-v");
        draw(&mut v);
        let key = f.view.read_with(&v, |view, _| view.session.state().current_source().unwrap().key());
        menu(&mut v, &f, CommandId::OpenInEditor);
        assert!(f.view.read_with(&v, |view, _| view.session.external_prompt().is_some()), "the real menu must enter the shared dirty-file decision");
        draw(&mut v);
        let labels = f.view.read_with(&v, |view, _| {
            let Some(Modal::StatusDetails {kind,..}) = &view.modal else { panic!("expected external decision") };
            view.details_labels(*kind)
        });
        assert_eq!(labels, ["Save and Open", "Open Saved Version", "Cancel"]);
        v.write_to_clipboard(ClipboardItem::new_string("must not paste".into()));
        v.simulate_keystrokes("ctrl-v ctrl-s ctrl-z x");
        for (w,h) in [(180.,90.), (1.,1.), (800.,500.)] {
            v.simulate_resize(size(px(w),px(h))); draw(&mut v);
            v.simulate_keystrokes("end up");
        }
        assert_eq!(f.view.read_with(&v, |view, _| view.session.state().current_source().unwrap().key()), key);
        assert!(f.view.read_with(&v, |view, _| view.launched.is_empty()));
        v.simulate_keystrokes("escape");
        assert!(f.view.read_with(&v, |view, _| view.session.external_prompt().is_none() && view.modal.is_none()));
        assert_eq!(std::fs::read(&path).unwrap(), b"saved\r\n");

        menu(&mut v, &f, CommandId::OpenWithDefaultApp); draw(&mut v);
        let saved_button = f.view.read_with(&v, |view, _| {
            let Some(Modal::StatusDetails {kind,..}) = &view.modal else { panic!("expected external decision") };
            view.modal_layout().unwrap().details_buttons(view.char_w, &view.details_labels(*kind))[1].center()
        });
        v.simulate_click(saved_button, Modifiers::default());
        assert_eq!(f.view.read_with(&v, |view, _| view.launched.len()), 1);
        let args=f.view.read_with(&v, |view, _| view.launched[0].args.clone());
        assert_eq!(args.last(),Some(&path.to_string_lossy().into_owned()));
        assert!(!args.iter().any(|arg|arg=="--goto" || arg.starts_with(&format!("{}:",path.display()))));
        assert!(f.view.read_with(&v, |view, _| view.session.state().view(view.session.state().active().unwrap().id()).unwrap().dirty()));
        assert_eq!(std::fs::read(&path).unwrap(), b"saved\r\n");

        menu(&mut v, &f, CommandId::OpenWithDefaultApp);
        v.simulate_keystrokes("s");
        settle(cx, &f, |view| view.launched.len() == 2);
        assert!(f.view.read_with(&v, |view, _| view.session.external_prompt().is_none() && view.modal.is_none()));
        assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
        v.simulate_keystrokes("ctrl-z");
        assert!(f.view.read_with(&v, |view, _| view.session.state().view(view.session.state().active().unwrap().id()).unwrap().dirty()));
        assert_eq!(std::fs::read(path).unwrap(), "draft λ\r\n".as_bytes());
    }
}

#[gpui::test]
fn external_save_failures_close_the_decision_without_launching(cx: &mut TestAppContext) {
    use lenz_content::write::readiness_fault::{at_path,Fault};
    use reader_core::save::Status;
    for fault in [Fault::BeforeReplace,Fault::AfterReplace] {
        let f=fixture(cx,Some(("external-fault.txt",b"saved")));
        let path=f.dir.path().join("external-fault.txt");
        settle(cx,&f,|view|view.session.state().current_source().is_some());
        let mut v=visual(cx,&f);draw(&mut v);
        f.view.update(&mut v,|view,_|view.enable_draft_input_for_test());
        v.write_to_clipboard(ClipboardItem::new_string("draft λ".into()));
        v.simulate_keystrokes("ctrl-a ctrl-v");
        let tab=f.view.read_with(&v,|view,_|view.session.state().active().unwrap().id());
        let guard=at_path(&path,fault);
        menu(&mut v,&f,CommandId::OpenWithDefaultApp);
        let ticket=f.view.read_with(&v,|view,_|view.session.external_prompt().unwrap().ticket);
        v.simulate_keystrokes("s");
        settle(cx,&f,|view|matches!(view.session.state().view(tab).unwrap().save_status(),Status::Rejected(_)|Status::Uncertain(_)));
        draw(&mut v);
        assert!(f.view.read_with(&v,|view,_|view.launched.is_empty() && view.modal.is_none() && view.session.external_prompt().is_none()));
        assert!(f.view.read_with(&v,|view,_|view.session.state().view(tab).unwrap().dirty() && view.session.state().view(tab).unwrap().can_undo()));
        f.view.update(&mut v,|view,cx|view.dispatch(Command::ChooseExternal {ticket,choice:reader_core::external::Choice::OpenSavedVersion},cx));
        assert!(f.view.read_with(&v,|view,_|view.launched.is_empty()));
        assert_eq!(std::fs::read(path).unwrap(),if fault==Fault::AfterReplace {"draft λ".as_bytes()}else{b"saved"});
        drop(guard);
    }
}

#[gpui::test]
fn external_launch_survives_housekeeping_before_normal_notification_handling(cx: &mut TestAppContext) {
    for cancel in [false,true] {
        let f=fixture(cx,Some(("delivery.txt",b"saved")));
        let path=f.dir.path().join("delivery.txt");
        settle(cx,&f,|view|view.session.state().current_source().is_some());
        let mut v=visual(cx,&f);draw(&mut v);
        f.view.update(&mut v,|view,_|view.enable_draft_input_for_test());
        v.write_to_clipboard(ClipboardItem::new_string("saved draft λ".into()));v.simulate_keystrokes("ctrl-a ctrl-v");
        menu(&mut v,&f,CommandId::OpenWithDefaultApp);
        // Hold normal GPUI polling while the existing direct render-command path accounts Save.
        f.view.update(&mut v,|view,_| {
            let ticket=view.session.external_prompt().unwrap().ticket;
            let _=view.session.dispatch(Command::ChooseExternal {ticket,choice:reader_core::external::Choice::SaveAndOpen});
            let until=std::time::Instant::now()+std::time::Duration::from_secs(10);
            while matches!(view.session.state().view(view.session.state().active().unwrap().id()).unwrap().save_status(),reader_core::save::Status::Saving|reader_core::save::Status::Rechecking) {
                assert!(std::time::Instant::now()<until);
                let (cols,rows)=view.sent_size[0];
                let _=view.session.dispatch(Command::SetTextSize {pane:0,cols,rows});
                std::thread::yield_now();
            }
            assert!(view.launched.is_empty());
        });
        assert_eq!(std::fs::read(&path).unwrap(),"saved draft λ".as_bytes());
        if cancel {v.simulate_keystrokes("escape");}
        else {f.view.update(&mut v,|view,cx|view.dispatch(Completion::AnalysisWake,cx));}
        assert_eq!(f.view.read_with(&v,|view,_|view.launched.len()),usize::from(!cancel),"housekeeping must not consume launch delivery");
        assert!(f.view.read_with(&v,|view,_|view.modal.is_none() && view.session.external_prompt().is_none()));
        f.view.update(&mut v,|view,cx|{view.poll(cx);view.dispatch(Completion::AnalysisWake,cx);});draw(&mut v);
        assert_eq!(f.view.read_with(&v,|view,_|view.launched.len()),usize::from(!cancel),"polling and repaint cannot launch twice");
        assert_eq!(std::fs::read(path).unwrap(),"saved draft λ".as_bytes());
    }
}
