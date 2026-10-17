//! Real GUI input against shared comparison/edit state, using GPUI's private test platform.
use super::merge_view::gui_tests::event;
use super::*;
use gpui::{AppContext, Entity, TestAppContext, WindowHandle};

#[path = "../../test_support/user_flows.rs"]
mod user_flows;

#[path = "followup_tests.rs"]
mod followup_tests;

#[path = "pr_reading_tests.rs"]
mod pr_reading_tests;

#[gpui::test]
fn gui_user_flow_discard_shorter_external_file_renders_before_diff_completes(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx, &"old\n".repeat(20), &"new\n".repeat(20));
    key(cx, &f, "right", false, true, true);
    std::fs::write(f.dir.path().join("right.txt"), "external\n").unwrap();
    key(cx, &f, "s", true, false, false);
    assert!(f.viewer.read_with(cx, |v, _| v.edits.has_dirty()));
    f.window
        .update(cx, |v, w, cx| {
            v.tool(
                Tool::Run(crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::Discard)),
                0.,
                w,
                cx,
            );
            // Paint the text-only frame before the async diff can repair its geometry.
            assert_ne!(v.comparison.generation, v.comparison.diffed);
            let _ = v.render(w, cx);
        })
        .unwrap();
    workflow_wait(cx, &f, |v| v.diffed == v.generation);
    assert_eq!(
        std::fs::read(f.dir.path().join("right.txt")).unwrap(),
        b"external\n"
    );
}

#[gpui::test]
fn gui_user_flow_merge_replay(cx: &mut TestAppContext) {
    let (dir, viewer) = merge_view::gui_tests::fixture(cx);
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone()))
        .unwrap();
    let f = Fixture {
        dir,
        viewer,
        window,
    };
    for _ in 0..3 {
        for &(choice, expected) in user_flows::MERGE_CHOICES {
            let disk = std::fs::read(f.dir.path().join("result")).unwrap();
            key(cx, &f, choice, false, true, true);
            f.viewer.read_with(cx, |v, _| {
                let m = v.merge.as_ref().unwrap();
                assert_eq!(m.text(), expected);
                assert_eq!(m.m.unresolved(), 0);
                assert!(m.dirty);
            });
            assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(), disk);
            key(cx, &f, "z", true, false, false);
            assert_eq!(
                f.viewer
                    .read_with(cx, |v, _| v.merge.as_ref().unwrap().m.unresolved()),
                1
            );
            key(cx, &f, "z", true, true, false);
            key(cx, &f, "s", true, false, false);
            assert_eq!(
                std::fs::read(f.dir.path().join("result")).unwrap(),
                expected.as_bytes()
            );
            assert!(
                !f.viewer
                    .read_with(cx, |v, _| v.merge.as_ref().unwrap().dirty)
            );
            key(cx, &f, "z", true, false, false);
            assert_eq!(
                f.viewer
                    .read_with(cx, |v, _| v.merge.as_ref().unwrap().m.unresolved()),
                1
            );
            assert_eq!(
                std::fs::read(f.dir.path().join("result")).unwrap(),
                expected.as_bytes()
            );
            f.window
                .update(cx, |v, w, cx| {
                    let _ = v.render(w, cx);
                })
                .unwrap();
        }
    }
}

#[gpui::test]
fn gui_whitespace_keeps_repeated_conflict_choices_separate(cx: &mut TestAppContext) {
    let (dir,viewer)=merge_view::gui_tests::fixture(cx);
    for (name,text) in [("base","base\nseparator\nbase\n"),("ours","ours\nseparator\nours\n"),("theirs","theirs\nseparator\ntheirs\n")] {
        std::fs::write(dir.path().join(name),text).unwrap();
    }
    let opened=crate::merge_session::open_files(&dir.path().join("base"),&dir.path().join("ours"),&dir.path().join("theirs"),&dir.path().join("result"),dir.path()).unwrap();
    viewer.update(cx,|v,cx|v.start_merge(opened,false,cx));
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();let f=Fixture{dir,viewer,window};
    key(cx,&f,"right",false,true,true);
    key(cx,&f,"w",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.merge.as_ref().unwrap().m.unresolved()),1);
    key(cx,&f,"n",false,false,false);
    key(cx,&f,"n",false,false,false);
    key(cx,&f,"left",false,true,true);
    key(cx,&f,"w",false,false,false);
    key(cx,&f,"s",true,false,false);
    assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(),b"ours\nseparator\ntheirs\n");
}

#[gpui::test]
fn gui_whitespace_output_change_requires_save_before_close(cx: &mut TestAppContext) {
    let (dir,viewer)=merge_view::gui_tests::fixture(cx);
    for (name,text) in [("base","a\n"),("ours"," a\n"),("theirs","a\nb\n")] {
        std::fs::write(dir.path().join(name),text).unwrap();
    }
    let opened=crate::merge_session::open_files(&dir.path().join("base"),&dir.path().join("ours"),&dir.path().join("theirs"),&dir.path().join("result"),dir.path()).unwrap();
    viewer.update(cx,|v,cx|v.start_merge(opened,false,cx));
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();let f=Fixture{dir,viewer,window};
    run(cx,&f,crate::catalog::Action::Merge(crate::merge_session::Act::AllFrom(crate::merge::Pick::Theirs)));
    key(cx,&f,"s",true,false,false);
    key(cx,&f,"w",false,false,false);
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.merge.as_ref().is_some_and(|m|m.dirty&&m.ask==Some(crate::merge_session::Ask::Close))));
    assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(),b"a\nb\n");
    key(cx,&f,"s",true,false,false);
    assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(),b" a\nb\n");
}

#[gpui::test]
fn gui_leaving_pr_clears_review_tabs_and_title(cx: &mut TestAppContext) {
    let f=repository_markdown_fixture(cx);let _provider=crate::pr::test_support::markdown_provider(f.dir.path());
    std::fs::write(f.dir.path().join("ref.md"),"local\n").unwrap();
    f.viewer.update(cx,|v,_|v.repo.as_mut().unwrap().execute(crate::repository_session::RepositoryCommand::OpenPr(Some("9".into()))));
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|r.pr.is_some()&&!r.busy()));
    assert!(pr_tab(cx,&f).is_some());
    run(cx,&f,crate::catalog::Action::Repository(crate::catalog::RepoAction::LocalChanges));
    workflow_wait(cx,&f,|v|v.spec==crate::repository_session::local_spec()&&v.repo.as_ref().is_some_and(|r|!r.busy()));
    f.viewer.read_with(cx,|v,_|{
        assert!(v.repo.as_ref().unwrap().pr.is_none());
        assert!(!v.repo.as_ref().unwrap().selection_title().unwrap().text.starts_with('#'));
    });
    assert_eq!(pr_tab(cx,&f),None);
    assert!(!toolbar_labels(cx,&f,1200.).iter().any(|(_,label)|label=="Conversation"));
    assert_eq!(std::fs::read(f.dir.path().join("ref.md")).unwrap(),b"local\n");
}

#[gpui::test]
fn gui_rendered_markdown_escape_leaves_focus(cx: &mut TestAppContext) {
    let f=repository_markdown_fixture(cx);
    for layout in [crate::markdown::MarkdownLayout::Inline,crate::markdown::MarkdownLayout::SideBySide] {
        enter_focus(cx,&f);
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(layout)));markdown_ready(cx,&f);
        key(cx,&f,"escape",false,false,false);
        assert!(f.viewer.read_with(cx,|v,_|!v.layout.focused(v.repo.as_ref())));
    }
}

#[gpui::test]
fn gui_user_flow_replay(cx: &mut TestAppContext) {
    use user_flows::{Snapshot, Step};
    for case in user_flows::CASES {
        let f = fixture(cx, case.left, case.right);
        let left = f.dir.path().join("left");
        let right = f.dir.path().join("right");
        for (directory, content) in [(&left, case.left), (&right, case.right)] {
            std::fs::create_dir(directory).unwrap();
            std::fs::write(directory.join("a.txt"), content).unwrap();
            std::fs::write(
                directory.join("b.txt"),
                if directory == &left {
                    "sentinel left\n"
                } else {
                    "sentinel right\n"
                },
            )
            .unwrap();
        }
        let spec = Spec::Plain {
            left: left.clone(),
            right: right.clone(),
        };
        let set = Arc::new(ChangeSet::load(&spec, f.dir.path()).unwrap());
        let doc =
            Arc::new(Doc::load(&set, 0, &left.join("a.txt"), &right.join("a.txt"), false).unwrap());
        let view = doc.view(&doc.line_diff(&Options::default()), 30.);
        f.viewer.update(cx, |v, cx| {
            *v = Viewer::new(
                set,
                spec,
                false,
                None,
                doc,
                view,
                false,
                Options::default(),
                gpui::font("monospace"),
                Instant::now(),
                false,
                cx,
            )
        });
        user_flows::replay(case, |step| {
            match step {
                Step::Inspect => {}
                Step::ApplyRight | Step::ApplyLeft => {
                    block(cx, &f, 0);
                    key(
                        cx,
                        &f,
                        if matches!(step, Step::ApplyRight) {
                            "right"
                        } else {
                            "left"
                        },
                        false,
                        true,
                        true,
                    );
                }
                Step::Undo => key(cx, &f, "z", true, false, false),
                Step::Redo => key(cx, &f, "z", true, true, false),
                Step::Save | Step::RefusedSave => key(cx, &f, "s", true, false, false),
                Step::Discard => run(cx,&f,crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::Discard)),
                Step::Reload => run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload)),
                Step::SwitchRoundTrip => {
                    key(cx, &f, "]", false, false, false);
                    workflow_wait(cx, &f, |v| {
                        v.current == 1 && v.doc().is_some() && v.diffed == v.generation
                    });
                    assert_eq!(
                        f.viewer.read_with(cx, |v, _| v
                            .doc()
                            .unwrap()
                            .text(Side::Right)
                            .to_owned()),
                        "sentinel right\n"
                    );
                    key(cx, &f, "[", false, false, false);
                }
                Step::External(content) => std::fs::write(right.join("a.txt"), content).unwrap(),
            }
            workflow_wait(cx, &f, |v| {
                v.current == 0 && v.doc().is_some() && v.diffed == v.generation
            });
            f.window
                .update(cx, |v, w, cx| {
                    let _ = v.render(w, cx);
                })
                .unwrap();
            if matches!(step, Step::RefusedSave) {
                assert!(f.viewer.read_with(cx, |v, _| {
                    v.write_error
                        .as_deref()
                        .is_some_and(|s| s.contains("nothing written"))
                }));
            }
            f.viewer.read_with(cx, |v, _| Snapshot {
                documents: [Side::Left, Side::Right].map(|s| v.doc().unwrap().text(s).to_owned()),
                disk: [left.join("a.txt"), right.join("a.txt")].map(|p| std::fs::read(p).unwrap()),
                dirty: v.edits.has_dirty(),
                last_applied_active: v.edits.applied(v.current).last().map(|r| r.active),
            })
        });
        assert_eq!(
            std::fs::read(right.join("b.txt")).unwrap(),
            b"sentinel right\n"
        );
        f.window.update(cx, |_, w, _| w.remove_window()).unwrap();
        cx.executor().run_until_parked();
    }
}

#[gpui::test]
fn gui_user_flow_modal_recovery(cx: &mut TestAppContext) {
    let f = fixture(cx, "old\n", "new\n");
    key(cx, &f, "right", false, true, true);
    let before = f.viewer.read_with(cx, |v, _| {
        (
            v.doc().unwrap().text(Side::Right).to_owned(),
            v.comparison.options,
            v.edits.has_dirty(),
        )
    });
    for _ in 0..3 {
        for [width, height] in [[900., 650.], [240., 160.], [1., 1.]] {
            cx.simulate_window_resize(f.window.into(), size(px(width), px(height)));
            cx.executor().run_until_parked();
            for modal in [
                "about", "help", "commands", "goto", "files", "find", "findset",
            ] {
                for (name, ctrl, shift, alt) in [
                    ("s", true, false, false),
                    ("z", true, false, false),
                    ("z", true, true, false),
                    ("left", false, true, true),
                ] {
                    match modal {
                        "about" => run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::About)),
                        "help" => key(cx, &f, "?", false, false, false),
                        "commands" => key(cx, &f, "f10", false, false, false),
                        "goto" => key(cx, &f, "g", true, false, false),
                        "files" => key(cx, &f, "p", true, false, false),
                        "find" => key(cx, &f, "f", true, false, false),
                        "findset" => key(cx, &f, "f", true, true, false),
                        _ => unreachable!(),
                    }
                    assert!(
                        f.viewer.read_with(cx, |v, _| match modal {
                            "about" => v.about.is_some(),
                            "help" => v.help,
                            "commands" => v.command_palette.is_some(),
                            "goto" => v.goto.is_some(),
                            "files" => v.switcher.is_some(),
                            "find" => v.find.is_some(),
                            "findset" => v.search.as_ref().is_some_and(|s| s.editing),
                            _ => unreachable!(),
                        }),
                        "{modal} must open at {width}x{height}"
                    );
                    f.window
                        .update(cx, |v, w, cx| {
                            let _ = v.render(w, cx);
                        })
                        .unwrap();
                    key(cx, &f, name, ctrl, shift, alt);
                    assert_eq!(
                        f.viewer.read_with(cx, |v, _| (
                            v.doc().unwrap().text(Side::Right).to_owned(),
                            v.comparison.options,
                            v.edits.has_dirty(),
                        )),
                        before,
                        "{modal}: {name} at {width}x{height}"
                    );
                    assert_eq!(text(&f), "new\n", "{modal}: Save must stay blocked");
                    key(cx, &f, "escape", false, false, false);
                    assert!(f.viewer.read_with(cx, |v, _| v.about.is_none()
                        && !v.help
                        && v.menu.is_none() && v.command_palette.is_none()
                        && v.goto.is_none()
                        && v.switcher.is_none()
                        && v.find.is_none()
                        && v.search.is_none()));
                }
            }
        }
    }
    // A no-match finder must leave a usable document and a working Save route.
    key(cx, &f, "p", true, false, false);
    for ch in "does-not-exist".chars() {
        key(cx, &f, &ch.to_string(), false, false, false);
    }
    key(cx, &f, "enter", false, false, false);
    key(cx, &f, "escape", false, false, false);
    key(cx, &f, "s", true, false, false);
    assert_eq!(text(&f), "old\n");
}

/// The platform's real text system, for tests that shape text with actual fonts.
///
/// gpui builds it only inside its platform, and the macOS platform must be created on the main thread,
/// which libtest never runs tests on. So on macOS one headless platform is built while the test binary
/// loads, on the main thread before `main`, and kept for the process; its CoreText text system is
/// `Send + Sync` and is shared by every test. Linux builds a platform per call, as before.
pub(super) fn native_text_system() -> std::sync::Arc<dyn gpui::PlatformTextSystem> {
    #[cfg(target_os = "macos")]
    {
        main_thread_text::TEXT.get().cloned().expect("text system built at load on the main thread")
    }
    #[cfg(not(target_os = "macos"))]
    {
        gpui_platform::current_platform(true).text_system()
    }
}

#[cfg(target_os = "macos")]
mod main_thread_text {
    use std::sync::{Arc, OnceLock};
    pub(super) static TEXT: OnceLock<Arc<dyn gpui::PlatformTextSystem>> = OnceLock::new();
    extern "C" fn build_on_main_thread() {
        let platform = gpui_platform::current_platform(true);
        let _ = TEXT.set(platform.text_system());
        std::mem::forget(platform); // the platform is not Send; it lives for the process with its text system
    }
    // Runs from dyld's initializer list, on the main thread, before libtest starts its worker threads.
    #[allow(unsafe_code)] // a load-time initializer in this test binary only
    #[used]
    #[unsafe(link_section = "__DATA,__mod_init_func")]
    static INIT: extern "C" fn() = build_on_main_thread;
}

pub(super) struct Fixture {
    pub(super) dir: tempfile::TempDir,
    pub(super) viewer: Entity<Viewer>,
    pub(super) window: WindowHandle<Viewer>,
}
fn fixture(cx: &mut TestAppContext, left: &str, right: &str) -> Fixture {
    fixture_named(cx, left, right, "txt")
}
pub(super) fn fixture_named(cx: &mut TestAppContext, left: &str, right: &str, extension: &str) -> Fixture {
    cx.skip_drawing();
    let dir = tempfile::Builder::new()
        .prefix(".lenz-gui-parity-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let lp = dir.path().join(format!("left.{extension}"));
    let rp = dir.path().join(format!("right.{extension}"));
    std::fs::write(&lp, left).unwrap();
    std::fs::write(&rp, right).unwrap();
    let spec = Spec::Plain {
        left: lp.clone(),
        right: rp.clone(),
    };
    let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
    let doc = Arc::new(Doc::load(&set, 0, &lp, &rp, false).unwrap());
    let view = doc.view(&doc.line_diff(&Options::default()), 30.);
    let viewer = cx.new(|cx| {
        Viewer::new(
            set,
            spec,
            false,
            None,
            doc,
            view,
            false,
            Options::default(),
            gpui::font("monospace"),
            Instant::now(),
            false,
            cx,
        )
    });
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone()))
        .unwrap();
    Fixture {
        dir,
        viewer,
        window,
    }
}
pub(super) fn key(cx: &mut TestAppContext, f: &Fixture, name: &str, ctrl: bool, shift: bool, alt: bool) {
    f.window
        .update(cx, |v, w, cx| {
            v.on_key(&event(name, ctrl, shift, alt), w, cx)
        })
        .unwrap();
    cx.executor().run_until_parked();
}
fn text(f: &Fixture) -> String {
    std::fs::read_to_string(f.dir.path().join("right.txt")).unwrap()
}
fn block(cx: &mut TestAppContext, f: &Fixture, target: usize) {
    for _ in 0..5 {
        if f.viewer
            .read_with(cx, |v, _| v.comparison.view.current_change())
            == Some(target)
        {
            return;
        }
        key(cx, f, "n", false, false, false);
    }
    panic!("change {target} was not reachable through GUI navigation");
}
#[gpui::test]
fn gui_find_command_follows_list_and_actual_diff_focus(cx:&mut TestAppContext) {
    let f=files_fixture(cx);
    f.viewer.update(cx,|v,_|{v.layout.set_files(true);v.nav_focused=true;});
    f.viewer.update(cx,|v,cx|v.interaction(commands::Interaction::Find,cx));
    assert!(f.viewer.read_with(cx,|v,_|v.file_search.active()&&v.find.is_none()));
    key(cx,&f,"q",false,false,false);let index=f.viewer.read_with(cx,|v,_|v.current);
    key(cx,&f,"f10",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()&&v.file_search.query()=="q"&&v.current==index));
    key(cx,&f,"escape",false,false,false);key(cx,&f,"escape",false,false,false);
    f.viewer.update(cx,|v,_|{v.nav_focused=false;v.diff_focus=Side::Left;v.sel=Some(Selection::caret(Side::Right,Pos::new(0,0)));});
    f.viewer.update(cx,|v,cx|v.interaction(commands::Interaction::Find,cx));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.find.as_ref().unwrap().side),Side::Left);
}

#[gpui::test]
fn gui_palette_lists_every_merge_and_version_action_at_small_sizes(cx:&mut TestAppContext) {
    use crate::catalog::Action;
    let (dir,viewer)=merge_view::gui_tests::fixture(cx);
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
    for [width,height] in [[640.,480.],[400.,240.]] {
        cx.simulate_window_resize(window.into(),gpui::size(px(width),px(height)));cx.executor().run_until_parked();
        window.update(cx,|v,w,cx|{let _=v.render(w,cx);v.on_key(&event("f10",false,false,false),w,cx);}).unwrap();
        viewer.read_with(cx,|v,_| {
            let p=v.command_palette.as_ref().unwrap();
            for act in crate::merge_session::Act::ALL {assert!(p.rows().any(|r|r.entry.action==Action::Merge(act)),"{act:?}");}
        });
        window.update(cx,|v,w,cx|v.on_key(&event("escape",false,false,false),w,cx)).unwrap();
    }
    let target=dir.path().join("binary");std::fs::write(&target,b"\0version").unwrap();
    viewer.update(cx,|v,cx|{v.merge=None;v.start_picker(version_picker::Opened{path:"binary".into(),target:target.clone(),snapshot:crate::version_session::TargetSnapshot::capture(&target),versions:crate::version_session::versions(vec![("Ours".into(),Some(b"\0version".to_vec()))]),why:"binary".into(),large_text:false,files:None,info:Vec::new()},false,cx);});
    window.update(cx,|v,w,cx|{let _=v.render(w,cx);v.on_key(&event("f10",false,false,false),w,cx);}).unwrap();
    viewer.read_with(cx,|v,_|{let p=v.command_palette.as_ref().unwrap();for act in version_picker::PickAct::ALL {assert!(p.rows().any(|r|r.entry.action==Action::Pick(act)),"{act:?}");}});
    assert_eq!(std::fs::read(target).unwrap(),b"\0version");
}

#[gpui::test]
fn gui_merge_toolbar_clicks_respect_about_and_preserve_dirty_result(cx:&mut TestAppContext) {
    use crate::{merge::Pick,merge_session::Control};
    let (dir,viewer)=merge_view::gui_tests::fixture(cx);
    let before=std::fs::read(dir.path().join("result")).unwrap();
    viewer.update(cx,|v,cx|v.merge_control(Control::Accept(0,Pick::Theirs),cx));
    let draft=viewer.read_with(cx,|v,_|v.merge.as_ref().unwrap().text());
    assert_ne!(draft.as_bytes(),before);
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
    viewer.update(cx,|v,cx|v.interaction(commands::Interaction::About,cx));
    let mut visual=gpui::VisualTestContext::from_window(window.into(),cx);
    visual.update(|w,cx|{let _=w.draw(cx);});
    for selector in ["m-save","m-undo","m-redo"] {
        // About is open for every click: the click only closes it
        viewer.update(cx,|v,cx|if v.about.is_none(){v.interaction(commands::Interaction::About,cx)});
        visual.update(|w,cx|{let _=w.draw(cx);});
        let bounds=visual.debug_bounds(selector).expect("primary merge toolbar control is rendered");
        visual.simulate_click(bounds.center(),Default::default());
        viewer.read_with(cx,|v,_|{
            let s=v.merge.as_ref().unwrap();
            assert!(v.about.is_none(),"a click outside closes About");assert!(s.dirty);assert_eq!(s.text(),draft);
            assert!(!s.find_editing());
        });
        assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),before);
    }
    viewer.update(cx,|v,_|v.about=None);
    window.update(cx,|v,w,cx|v.on_key(&event("s",true,false,false),w,cx)).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("result")).unwrap(),draft);
}

#[gpui::test]
fn gui_find_keys_keep_help_and_palette_modal(cx:&mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    run(cx,&f,crate::catalog::Action::Help(crate::catalog::HelpAction::Shortcuts));
    key(cx,&f,"f",true,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.help&&v.find.is_none()));
    key(cx,&f,"escape",false,false,false);
    key(cx,&f,"f10",false,false,false);
    key(cx,&f,"f",true,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()&&v.find.is_none()));
    key(cx,&f,"escape",false,false,false);
    key(cx,&f,"f",true,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.find.is_some()));
}

#[gpui::test]
fn gui_help_scroll_keys_keep_modal_and_block_document_writes(cx: &mut TestAppContext) {
    let f = fixture(cx, "old\n", "new\n");
    key(cx, &f, "right", false, true, true);
    let draft = f.viewer.read_with(cx, |v, _| v.doc().unwrap().text_right.to_string());
    assert_eq!(draft, "old\n");
    cx.simulate_window_resize(f.window.into(), size(px(400.0), px(240.0)));
    cx.executor().run_until_parked();
    key(cx, &f, "?", false, false, false);
    key(cx, &f, "end", false, false, false);
    assert!(f.viewer.read_with(cx, |v, _| v.help), "End must scroll Help without dismissing it");
    for (name, ctrl, shift, alt) in [("up", false, false, false), ("pagedown", false, false, false), ("s", true, false, false), ("z", true, false, false), ("left", false, true, true), ("f10", false, false, false)] {
        key(cx, &f, name, ctrl, shift, alt);
        f.viewer.read_with(cx, |v, _| {
            assert!(v.help, "Help must consume {name}");
            assert!(v.edits.dirty(v.current));
            assert_eq!(&*v.doc().unwrap().text_right, draft);
        });
        assert_eq!(text(&f), "new\n");
    }
    key(cx, &f, "escape", false, false, false);
    key(cx, &f, "s", true, false, false);
    assert_eq!(text(&f), draft);
}

#[gpui::test]
fn gui_help_shift_slash_opens_before_merge_find(cx: &mut TestAppContext) {
    let (dir, viewer) = merge_view::gui_tests::fixture(cx);
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone())).unwrap();
    let f = Fixture { dir, viewer, window };
    key(cx, &f, "/", false, true, false);
    f.viewer.read_with(cx, |v, _| {
        assert!(v.help, "Shift-slash must open Help in a merge");
        assert!(!v.merge.as_ref().unwrap().find_editing());
    });
    key(cx, &f, "end", false, false, false);
    key(cx, &f, "s", true, false, false);
    key(cx, &f, "escape", false, false, false);
    assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(), b"needle ours\n");
}

#[gpui::test]
fn gui_help_wrapped_pages_render_in_all_themes_and_workflows(cx: &mut TestAppContext) {
    use unicode_width::UnicodeWidthStr;
    let (dir, viewer) = merge_view::gui_tests::fixture(cx);
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone())).unwrap();
    let f = Fixture { dir, viewer, window };
    let disk = std::fs::read(f.dir.path().join("result")).unwrap();
    f.viewer.update(cx, |v, cx| v.merge_control(crate::merge_session::Control::Accept(0, crate::merge::Pick::Theirs), cx));
    let merge_draft = f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().text());
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    visual.update(|w, cx| { let focus = f.viewer.read(cx).focus.clone(); focus.focus(w, cx); });
    for merging in [true, false] {
        if !merging { f.viewer.update(cx, |v, _| v.merge = None); }
        for theme in theme::builtin_entries() {
            run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::ChangeTheme));
            let selected = f.viewer.read_with(cx, |v, _| v.theme_chooser.as_ref().unwrap().entries.iter().position(|e| e.id == theme.id).unwrap());
            key(cx, &f, "home", false, false, false);
            for _ in 0..selected { key(cx, &f, "down", false, false, false); }
            key(cx, &f, "enter", false, false, false);
            for [width, height] in [[640.0, 480.0], [400.0, 240.0]] {
                visual.simulate_resize(size(px(width), px(height)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
                visual.simulate_keystrokes("shift-/");
                assert!(f.viewer.read_with(cx, |v, _| v.help));
                if merging {
                    visual.update(|w, cx| { let _ = w.draw(cx); });
                    for selector in ["m-save", "m-undo", "m-redo"] {
                        let bounds = visual.debug_bounds(selector).unwrap();
                        visual.simulate_click(bounds.center(), Default::default());
                    }
                    visual.simulate_keystrokes("ctrl-s ctrl-z alt-shift-left f10");
                    f.viewer.read_with(cx, |v, _| {
                        let merge = v.merge.as_ref().unwrap();
                        assert!(v.help && merge.dirty && !merge.find_editing());
                        assert_eq!(merge.text(), merge_draft);
                    });
                    assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(), disk);
                }
                let (rows, visible, cols) = window.update(cx, |v, w, _| {
                    let ((_, _, width, _), visible) = help_layout(w.viewport_size());
                    (help_rows(w.viewport_size(), v.char_w), visible, ((width - 24.0) / v.char_w.max(1.0)).floor() as usize)
                }).unwrap();
                assert!(visible > 0);
                assert!(rows.iter().all(|(row, _)| row.width() <= cols), "Help must wrap in {width}x{height}");
                let mut reached = vec![false; rows.len()];
                loop {
                    visual.update(|w, cx| { let _ = w.draw(cx); });
                    assert!(visual.debug_bounds("help-dialog").is_some(), "Help must paint above either workflow");
                    let start = f.viewer.read_with(cx, |v, _| v.help_scroll);
                    for seen in reached.iter_mut().skip(start).take(visible) { *seen = true; }
                    if start + visible >= rows.len() { break; }
                    visual.simulate_keystrokes("pagedown");
                    assert!(f.viewer.read_with(cx, |v, _| v.help_scroll > start));
                }
                assert!(reached.into_iter().all(|seen| seen), "every explanation must be reachable");
                let content: String = rows.iter().map(|(row, _)| row.as_str()).collect();
                for (keys, action) in KEYMAP { assert!(content.contains(keys)); assert!(content.contains(action)); }
                visual.simulate_keystrokes("end up");
                let end = rows.len() - visible;
                assert_eq!(f.viewer.read_with(cx, |v, _| v.help_scroll), end - 1);
                visual.simulate_event(ScrollWheelEvent { delta: ScrollDelta::Lines(point(0.0, 1.0)), ..Default::default() });
                assert_eq!(f.viewer.read_with(cx, |v, _| v.help_scroll), end.saturating_sub(4));
                visual.simulate_keystrokes("end");
                let close = window.update(cx, |_, w, _| help_close_bounds(w.viewport_size()).center()).unwrap();
                visual.simulate_click(close, Default::default());
                assert!(!f.viewer.read_with(cx, |v, _| v.help), "Close must dismiss scrolled Help");
                visual.simulate_keystrokes("? end");
                visual.simulate_resize(size(px(1.0), px(1.0)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
                visual.simulate_keystrokes("end up escape");
                assert!(!f.viewer.read_with(cx, |v, _| v.help));
            }
        }
    }
}

#[gpui::test]
fn gui_help_mouse_keeps_source_apply_and_draft_unchanged(cx: &mut TestAppContext) {
    let f = fixture(cx, "old\n", "new\n");
    let (x, y) = f.window.update(cx, |v, w, cx| {
        let _ = v.render(w, cx);
        assert!(v.doc().unwrap().writable[1].is_ok());
        let connector = v.comparison.view.frame().connectors[0];
        (v.diff_x() + v.comparison_widths(f32::from(w.viewport_size().width) - v.diff_x())[0] + 2.0,
            v.top() + arrow_rows(&connector).0 * line_h())
    }).unwrap();
    key(cx, &f, "?", false, false, false);
    click(cx, &f, x, y);
    f.viewer.read_with(cx, |v, _| {
        // the click closes help (outside it) or leaves it open (inside); either way it goes no further
        assert!(!v.edits.dirty(v.current));
        assert_eq!(&*v.doc().unwrap().text_right, "new\n");
    });
    assert_eq!(text(&f), "new\n");
    key(cx, &f, "escape", false, false, false);
    click(cx, &f, x, y);
    assert!(f.viewer.read_with(cx, |v, _| v.edits.dirty(v.current)));
    assert_eq!(f.viewer.read_with(cx, |v, _| v.doc().unwrap().text_right.to_string()), "old\n");
    assert_eq!(text(&f), "new\n");
}

#[gpui::test]
fn gui_merge_question_consumes_choices_menus_and_mouse_until_dismissed(cx: &mut TestAppContext) {
    use crate::merge::Pick;
    use crate::merge_session::{Ask, Control};
    let (dir, viewer) = merge_view::gui_tests::fixture(cx);
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone())).unwrap();
    let f = Fixture { dir, viewer, window };
    let disk = std::fs::read(f.dir.path().join("result")).unwrap();
    let draft = f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().text());
    let (x, y) = f.window.update(cx, |v, w, cx| {
        let _ = v.render(w, cx);
        let &(x, y, _) = v.merge.as_ref().unwrap().controls.iter()
            .find(|(_, _, c)| matches!(c, Control::Accept(_, Pick::Theirs))).unwrap();
        (v.diff_x() + x + 2.0, v.top() + y + 2.0)
    }).unwrap();
    key(cx, &f, "enter", true, false, false);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().ask), Some(Ask::DoneWithConflicts));
    for (name, ctrl, shift, alt) in [
        ("right", false, true, true), ("left", false, true, true),
        ("down", false, true, true), ("backspace", false, true, true),
        (":", false, false, false), ("f10", false, false, false), ("f", true, false, false),
    ] {
        key(cx, &f, name, ctrl, shift, alt);
        click(cx, &f, x, y);
        f.viewer.read_with(cx, |v, _| {
            let s = v.merge.as_ref().unwrap();
            assert_eq!(s.ask, Some(Ask::DoneWithConflicts));
            assert_eq!(s.text(), draft);
            assert_eq!(s.m.unresolved(), 1);
            assert!(!s.dirty && !s.find_editing() && v.menu.is_none());
        });
        assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(), disk);
    }
    key(cx, &f, "escape", false, false, false);
    click(cx, &f, x, y);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().text()), "needle theirs\n");
    key(cx, &f, "s", true, false, false);
    assert_eq!(std::fs::read_to_string(f.dir.path().join("result")).unwrap(), "needle theirs\n");
}

#[gpui::test]
fn gui_merge_disk_question_precedes_existing_find(cx: &mut TestAppContext) {
    use crate::merge_session::Ask;
    for action in ["r", "s", "escape"] {
        let (dir, viewer) = merge_view::gui_tests::fixture(cx);
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone())).unwrap();
        let f = Fixture { dir, viewer, window };
        key(cx, &f, "f", true, false, false);
        key(cx, &f, "n", false, false, false);
        let find = f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().find_notice());
        let draft = f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().text());
        let path = f.dir.path().join("result");
        let external = "needle theirs\nextra\n";
        std::fs::write(&path, external).unwrap();
        // The same callback as reactivating a window after an external editor save.
        f.viewer.update(cx, |v, cx| v.check_merge_on_disk(cx));
        assert_eq!(f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().ask), Some(Ask::ChangedOnDisk));
        key(cx, &f, action, action == "s", false, false);
        f.viewer.read_with(cx, |v, _| {
            let s = v.merge.as_ref().unwrap();
            assert!(s.find_editing(), "the question must receive {action} before Find");
            assert_eq!(s.find_notice(), find);
            assert_eq!(s.ask, if action == "s" { Some(Ask::Overwrite { done: false }) } else { None });
            assert_eq!(s.text(), if action == "r" { external.to_string() } else { draft.clone() });
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
        if action == "s" {
            key(cx, &f, "escape", false, false, false);
            assert!(f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().ask.is_none()));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
        }
    }
}

fn merge_dialog_fixture(cx: &mut TestAppContext, picker: bool) -> Fixture {
    let (dir, viewer) = merge_view::gui_tests::fixture(cx);
    if picker {
        for (name, bytes) in [("base", &b"\0base\xff"[..]), ("ours", &b"\0ours\xff"[..]),
            ("theirs", &b"\0theirs\xfe"[..]), ("result", &b"\0ours\xff"[..])] {
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        let d = dir.path();
        let opened = crate::version_session::open_files(&d.join("base"), &d.join("ours"),
            &d.join("theirs"), &d.join("result"), d).unwrap().unwrap();
        viewer.update(cx, |v, cx| { v.merge = None; v.start_picker(opened, false, cx); });
    }
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone())).unwrap();
    Fixture { dir, viewer, window }
}

#[gpui::test]
fn gui_disk_question_closes_command_menu_before_merge_and_picker_actions(cx: &mut TestAppContext) {
    use crate::merge_session::Ask;
    for picker in [false, true] {
        for action in ["r", "s", "escape"] {
            let f = merge_dialog_fixture(cx, picker);
            let source = ["base", "ours", "theirs"].map(|name| std::fs::read(f.dir.path().join(name)).unwrap());
            if picker { key(cx, &f, "2", false, false, false); }
            else { key(cx, &f, "left", false, true, true); }
            let expected = if picker { &b"\0theirs\xfe"[..] } else { &b"needle theirs\n"[..] };
            key(cx, &f, "f10", false, false, false);
            assert!(f.viewer.read_with(cx, |v, _| v.command_palette.is_some()));
            f.viewer.update(cx, |v, cx| v.check_merge_on_disk(cx));
            assert!(f.viewer.read_with(cx, |v, _| v.command_palette.is_some()), "unchanged files preserve the open menu");
            let path = f.dir.path().join("result");
            let external = b"external result changed while the command menu was open\n";
            std::fs::write(&path, external).unwrap();
            f.viewer.update(cx, |v, cx| v.check_merge_on_disk(cx));
            f.viewer.read_with(cx, |v, _| {
                assert!(v.menu.is_none() && v.command_palette.is_none());
                let ask = if picker { v.picker_v.as_ref().unwrap().ask } else { v.merge.as_ref().unwrap().ask };
                assert_eq!(ask, Some(Ask::ChangedOnDisk));
            });
            // The raised question continues to block both menus and underlying choices.
            key(cx, &f, "f10", false, false, false);
            if picker { key(cx, &f, "1", false, false, false); }
            else { key(cx, &f, "right", false, true, true); }
            f.viewer.read_with(cx, |v, _| {
                assert!(v.menu.is_none() && v.command_palette.is_none());
                if picker { assert_eq!(v.picker_v.as_ref().unwrap().chosen, Some(1)); }
                else { assert_eq!(v.merge.as_ref().unwrap().text().as_bytes(), expected); }
            });
            key(cx, &f, action, action == "s", false, false);
            f.viewer.read_with(cx, |v, _| {
                assert!(v.menu.is_none() && v.command_palette.is_none());
                let ask = if picker { v.picker_v.as_ref().unwrap().ask } else { v.merge.as_ref().unwrap().ask };
                assert_eq!(ask, if action == "s" { Some(Ask::Overwrite { done: false }) } else { None });
                if picker {
                    let s = v.picker_v.as_ref().unwrap();
                    assert_eq!(s.chosen, if action == "r" { None } else { Some(1) });
                } else {
                    assert_eq!(v.merge.as_ref().unwrap().text().as_bytes(), if action == "r" { external } else { expected });
                }
            });
            assert_eq!(std::fs::read(&path).unwrap(), external);
            if action == "s" {
                key(cx, &f, "s", true, false, false);
                assert_eq!(std::fs::read(&path).unwrap(), expected);
                assert!(f.viewer.read_with(cx, |v, _| if picker { v.picker_v.as_ref().unwrap().ask.is_none() } else { v.merge.as_ref().unwrap().ask.is_none() }));
            }
            assert_eq!(["base", "ours", "theirs"].map(|name| std::fs::read(f.dir.path().join(name)).unwrap()), source);
        }
    }
}

#[gpui::test]
fn gui_native_close_question_closes_command_menu_before_save_or_cancel(cx: &mut TestAppContext) {
    use crate::merge_session::Ask;
    for picker in [false, true] {
        for save in [false, true] {
            let f = merge_dialog_fixture(cx, picker);
            let path = f.dir.path().join("result");
            let before = std::fs::read(&path).unwrap();
            let source = ["base", "ours", "theirs"].map(|name| std::fs::read(f.dir.path().join(name)).unwrap());
            if picker { key(cx, &f, "2", false, false, false); }
            else { key(cx, &f, "left", false, true, true); }
            key(cx, &f, "f10", false, false, false);
            assert!(f.viewer.read_with(cx, |v, _| v.command_palette.is_some()));
            // The native window's close callback invokes these same guards.
            assert!(!f.viewer.update(cx, |v, cx| if picker { v.picker_v_may_close(cx) } else { v.merge_may_close(cx) }));
            f.viewer.read_with(cx, |v, _| {
                assert!(v.menu.is_none() && v.command_palette.is_none());
                assert_eq!(if picker { v.picker_v.as_ref().unwrap().ask } else { v.merge.as_ref().unwrap().ask }, Some(Ask::Close));
            });
            assert_eq!(std::fs::read(&path).unwrap(), before);
            key(cx, &f, if save { "s" } else { "escape" }, save, false, false);
            if save {
                let expected = if picker { &b"\0theirs\xfe"[..] } else { &b"needle theirs\n"[..] };
                assert_eq!(std::fs::read(&path).unwrap(), expected);
                assert!(f.viewer.read_with(cx, |v, _| if picker { v.picker_v.as_ref().unwrap().ask.is_none() } else { v.merge.as_ref().unwrap().ask.is_none() }));
            } else {
                assert_eq!(std::fs::read(&path).unwrap(), before);
                assert!(f.viewer.read_with(cx, |v, _| v.merge.is_none() && v.picker_v.is_none()));
            }
            assert_eq!(["base", "ours", "theirs"].map(|name| std::fs::read(f.dir.path().join(name)).unwrap()), source);
        }
    }
}

#[gpui::test]
fn gui_file_switcher_closes_before_changed_set_or_refresh_repaint(cx: &mut TestAppContext) {
    for refresh in [false, true] {
        let f = files_fixture(cx);
        for count in [1, 0] {
            key(cx, &f, "p", true, false, false);
            assert!(f.viewer.read_with(cx, |v, _| v.switcher.as_ref().unwrap().hits.len() > count));
            f.window.update(cx, |v, w, cx| {
                let mut next = (*v.set).clone();
                next.comparisons.truncate(count);
                next.groups.clear();
                if refresh { v.accept_repository_refresh(next, v.spec.clone(), cx); }
                else { v.show_change_set(next, cx); }
                assert!(v.switcher.is_none());
                let _ = v.render(w, cx);
            }).unwrap();
            assert_eq!(std::fs::read_to_string(f.dir.path().join("right/00.txt")).unwrap(), "new\n".repeat(200));
        }
    }
}

#[gpui::test]
fn gui_grouped_image_commands_keep_every_mode_and_adjustment(cx:&mut TestAppContext) {
    let f=fixture(cx,"old","new");let lp=f.dir.path().join("left.png");let rp=f.dir.path().join("right.png");
    image::RgbaImage::from_pixel(2,2,image::Rgba([0,0,0,255])).save(&lp).unwrap();
    image::RgbaImage::from_pixel(2,2,image::Rgba([255,0,0,255])).save(&rp).unwrap();
    let spec=Spec::Plain{left:lp,right:rp};let set=ChangeSet::load(&spec,f.dir.path()).unwrap();
    f.viewer.update(cx,|v,cx|{v.spec=spec;v.show_change_set(set,cx);});
    workflow_wait(cx,&f,|v|v.image_doc().is_some());key(cx,&f,"f10",false,false,false);
    f.viewer.read_with(cx,|v,_| {
        let p=v.command_palette.as_ref().unwrap();
        for (i,entry) in crate::image_session::image_commands().iter().enumerate() {
            assert!(p.rows().any(|r|r.entry.action==crate::catalog::Action::Image(i)),"{}",entry.label);
        }
    });
}

pub(super) fn click(cx: &mut TestAppContext, f: &Fixture, x: f32, y: f32) {
    let event = MouseDownEvent {
        button: MouseButton::Left,
        position: point(px(x), px(y)),
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    };
    f.window
        .update(cx, |v, w, cx| v.on_mouse_down(&event, w, cx))
        .unwrap();
    cx.executor().run_until_parked();
}

fn files_fixture(cx: &mut TestAppContext) -> Fixture {
    cx.skip_drawing();
    let dir = tempfile::Builder::new().prefix(".lenz-gui-parity-files-").tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let left = dir.path().join("left");
    let right = dir.path().join("right");
    std::fs::create_dir(&left).unwrap();
    std::fs::create_dir(&right).unwrap();
    for i in 0..40 {
        std::fs::write(left.join(format!("{i:02}.txt")), "old\n".repeat(100)).unwrap();
        std::fs::write(right.join(format!("{i:02}.txt")), "new\n".repeat(200)).unwrap();
    }
    std::os::unix::fs::symlink(".",left.join("directory-link")).unwrap();
    std::os::unix::fs::symlink(".",right.join("directory-link")).unwrap();
    let spec = Spec::Plain { left: left.clone(), right: right.clone() };
    let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
    assert_eq!(set.comparisons.len(),40);
    let doc = Arc::new(Doc::load(&set, 0, &left.join("00.txt"), &right.join("00.txt"), false).unwrap());
    let view = doc.view(&doc.line_diff(&Options::default()), 10.);
    let viewer = cx.new(|cx| Viewer::new(set, spec, false, None, doc, view, false,
        Options::default(), gpui::font("monospace"), Instant::now(), false, cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone())).unwrap();
    Fixture { dir, viewer, window }
}

#[test]
fn gui_files_pane_keys_move_files_and_never_move_the_diff_caret() {
    let mut cx = TestAppContext::single();
    let f = files_fixture(&mut cx);
    let (x, y) = f.viewer.read_with(&cx, |v, _| (v.diff_x() + num_w() + 30., v.top() + line_h() / 2.));
    click(&mut cx, &f, x, y);
    let caret = f.viewer.read_with(&cx, |v, _| v.sel.unwrap());
    // a click on the first file's row (under the "." heading) gives Files the keyboard
    click(&mut cx, &f, 20., y + line_h());
    // Left goes to the file's folder heading (its summary) and Down back to the file: the caret never moves
    key(&mut cx, &f, "left", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.sel), Some(caret));
    assert!(f.viewer.read_with(&cx, |v, _| v.comparison.folders.summary().is_some()));
    key(&mut cx, &f, "down", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| (v.comparison.current, v.comparison.folders.summary().is_none())), (0, true));
    key(&mut cx, &f, "down", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.comparison.current), 1);
    key(&mut cx, &f, "up", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.comparison.current), 0);
    key(&mut cx, &f, "end", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.comparison.current), 39);
    key(&mut cx, &f, "down", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.comparison.current), 39);
    key(&mut cx, &f, "home", false, false, false);
    key(&mut cx, &f, "pagedown", false, false, false);
    assert!(f.viewer.read_with(&cx, |v, _| v.comparison.current > 0));
    key(&mut cx, &f, "tab", false, false, false);
    let current = f.viewer.read_with(&cx, |v, _| v.comparison.current);
    key(&mut cx, &f, "down", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.comparison.current), current);
}

fn poll_list_search(cx:&mut TestAppContext,f:&Fixture) {
    f.window.update(cx,|v,w,cx|v.poll_pane_search(w,cx)).unwrap();
}

#[test]
fn gui_file_search_previews_without_loading_and_cancel_restores_scroll() {
    let mut cx=TestAppContext::single();let f=files_fixture(&mut cx);
    let before=f.viewer.update(&mut cx,|v,_| {
        v.nav_focused=true;v.nav_scroll=8.;
        (v.comparison.current,v.comparison.generation,v.loaded.len(),v.nav.len())
    });
    key(&mut cx,&f,"3",false,false,false);poll_list_search(&mut cx,&f);
    f.viewer.read_with(&cx,|v,_| {
        assert_eq!(v.file_search.current(),Some(3));
        assert_eq!((v.comparison.current,v.comparison.generation,v.loaded.len(),v.nav.len()),before);
        assert!(v.sidebar_rows().iter().any(|r|!r.found.is_empty()));
    });
    key(&mut cx,&f,"down",false,false,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.file_search.current()),Some(13));
    key(&mut cx,&f,"f3",false,true,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.file_search.current()),Some(3));
    key(&mut cx,&f,"escape",false,false,false);
    f.viewer.read_with(&cx,|v,_| {
        assert!(!v.file_search.active());assert_eq!(v.nav_scroll,8.);
        assert_eq!((v.comparison.current,v.comparison.generation,v.loaded.len(),v.nav.len()),before);
    });
}

#[test]
fn gui_file_search_confirm_retains_query_and_copy_uses_candidate_path() {
    let mut cx=TestAppContext::single();let f=files_fixture(&mut cx);
    f.viewer.update(&mut cx,|v,_|v.nav_focused=true);
    key(&mut cx,&f,"3",false,false,false);poll_list_search(&mut cx,&f);
    key(&mut cx,&f,"c",true,false,false);
    assert_eq!(cx.update(|cx|cx.read_from_clipboard().unwrap().text().unwrap()),"03.txt");
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.comparison.current),0);
    key(&mut cx,&f,"f10",false,true,false);
    f.viewer.read_with(&cx,|v,_|assert!(v.menu.as_ref().unwrap().items.iter().all(|(_,a)|matches!(a,MenuAction::File(_,3,_)))));
    key(&mut cx,&f,"escape",false,false,false);
    key(&mut cx,&f,"enter",false,false,false);
    f.viewer.read_with(&cx,|v,_| {
        assert_eq!(v.comparison.current,3);assert_eq!(v.file_search.query(),"3");assert!(v.nav_focused);
    });
    key(&mut cx,&f,"down",false,false,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.comparison.current),3);
    key(&mut cx,&f,"escape",false,false,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.comparison.current),3);
}

#[test]
fn gui_file_typing_reserves_plain_actions_and_modified_text_is_not_a_query() {
    let mut cx=TestAppContext::single();let f=files_fixture(&mut cx);
    let generation=f.viewer.update(&mut cx,|v,_| {v.nav_focused=true;v.comparison.generation});
    for key_name in ["q","y","b","g","?",":"," "] {
        key(&mut cx,&f,key_name,false,false,false);
    }
    key(&mut cx,&f,"i",false,false,true);
    key(&mut cx,&f,"h",true,false,false);
    f.viewer.read_with(&cx,|v,_| {
        assert_eq!(v.file_search.query(),"qybg?: ");assert!(v.menu.is_none());assert!(!v.help);
        assert_eq!(v.comparison.generation,generation);assert_eq!(v.comparison.current,0);
    });
    key(&mut cx,&f,"a",true,false,false);
    cx.update(|cx|cx.write_to_clipboard(ClipboardItem::new_string("CAFÉ\nignored".into())));
    key(&mut cx,&f,"v",true,false,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.file_search.query().to_owned()),"CAFÉ");
    key(&mut cx,&f,"backspace",false,false,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.file_search.query().to_owned()),"CAF");
    key(&mut cx,&f,"f",true,true,false);key(&mut cx,&f,"q",false,false,false);
    f.viewer.read_with(&cx,|v,_| {assert_eq!(v.search.as_ref().unwrap().query,"q");assert_eq!(v.file_search.query(),"CAF");});
}

#[test]
fn gui_log_search_keeps_selection_and_exposes_matching_metadata() {
    let mut cx=TestAppContext::single();let f=fixture(&mut cx,"old","new");
    f.viewer.update(&mut cx,|v,_| {
        let mut p=Picker::new(f.dir.path().into(),true);p.focus=Pane::Log;
        p.log.extend((0..3).map(|i|crate::history::Commit {
            id:format!("{:040x}",i+1),parents:Vec::new(),author:"Alice".into(),email:"alice@example.test".into(),time:0,offset:0,
            subject:if i==1 {"Fix needle".into()} else {"Other needle".into()},
        }));
        p.anchor=1;p.scroll=1.;v.repo = Some(crate::repository_session::RepositorySession::with_picker(p));
    });
    let before=f.viewer.read_with(&cx,|v,_|(v.comparison.generation,v.loaded.len(),v.set.clone()));
    for k in ["n","e","e","d","l","e"] {key(&mut cx,&f,k,false,false,false);}
    poll_list_search(&mut cx,&f);
    key(&mut cx,&f,"down",false,false,false);
    f.viewer.read_with(&cx,|v,_| {
        let p=v.repo.as_ref().map(|r| &r.picker).unwrap();assert_eq!(p.anchor,1);assert_eq!(p.pane_search.current(),Some(2));
        assert_eq!(v.comparison.generation,before.0);assert_eq!(v.loaded.len(),before.1);assert!(Arc::ptr_eq(&v.set,&before.2));
        assert_eq!(p.rows(),4);
    });
    key(&mut cx,&f,"c",true,false,false);
    assert_eq!(cx.update(|cx|cx.read_from_clipboard().unwrap().text().unwrap()),format!("{:040x}",2));
    key(&mut cx,&f,"escape",false,false,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.repo.as_ref().map(|r| &r.picker).unwrap().scroll),1.);
    f.viewer.update(&mut cx,|v,_|v.repo.as_mut().map(|r| &mut r.picker).unwrap().filter_editing=true);
    key(&mut cx,&f,"f",true,false,false);
    for k in ["e","x","a","m","p","l","e"] {key(&mut cx,&f,k,false,false,false);}
    poll_list_search(&mut cx,&f);
    f.viewer.read_with(&cx,|v,_|assert!(v.log_frame().unwrap().rows.iter().any(|r|r.subject.contains("alice@example.test"))));
}

#[gpui::test]
fn gui_log_modified_navigation_preserves_comparison_selection_outside_search(cx:&mut TestAppContext) {
    let f=repository_markdown_fixture(cx);
    let initial=f.viewer.read_with(cx,|v,_|v.repo.as_ref().map(|r| &r.picker).unwrap().pick_generation);
    key(cx,&f,"down",true,false,false);
    let accepted=f.viewer.read_with(cx,|v,_| {
        let p=v.repo.as_ref().map(|r| &r.picker).unwrap();
        assert_eq!(p.anchor,1);assert_eq!(p.second,Some(2));assert!(!p.range);
        assert!(p.pick_generation>initial);
        p.pick_generation
    });
    key(cx,&f,"f",true,false,false);
    for c in "Markdown".chars() {key(cx,&f,&c.to_string(),false,false,false);}
    poll_list_search(cx,&f);
    key(cx,&f,"down",true,false,false);
    f.viewer.read_with(cx,|v,_| {
        let p=v.repo.as_ref().map(|r| &r.picker).unwrap();
        assert!(p.pane_search.active());assert_eq!(p.anchor,1);assert_eq!(p.second,Some(2));assert!(!p.range);
        assert_eq!(p.pick_generation,accepted,"modified candidate navigation must not load a comparison");
    });
}

#[test]
fn gui_find_uses_focused_text_side_and_sources_keep_filter_focus() {
    let mut cx=TestAppContext::single();let f=fixture(&mut cx,"left","right");
    f.viewer.update(&mut cx,|v,_| {v.diff_focus=Side::Left;v.sel=Some(Selection::caret(Side::Right,Pos::new(0,0)));});
    key(&mut cx,&f,"f",true,false,false);
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.comparison.find.as_ref().unwrap().side),Side::Left);
    f.viewer.update(&mut cx,|v,_| {let mut p=Picker::new(f.dir.path().into(),true);p.focus=Pane::Sidebar;v.repo = Some(crate::repository_session::RepositorySession::with_picker(p));});
    key(&mut cx,&f,"f",true,false,false);key(&mut cx,&f,"q",false,false,false);
    f.viewer.read_with(&cx,|v,_| {assert!(v.comparison.find.is_none());assert_eq!(v.repo.as_ref().map(|r| &r.picker).unwrap().sources.filter,"q");});
}

#[test]
fn gui_wheel_over_log_never_scrolls_diff_horizontally() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, &"o".repeat(500), &"n".repeat(500));
    f.viewer.update(&mut cx, |v, _| {
        v.repo = Some(crate::repository_session::RepositorySession::with_picker(crate::repository_session::Picker::new(f.dir.path().into(), true)));
        v.char_w = 8.;
    });
    f.window.update(&mut cx, |v, w, cx| {
        let anchor = v.comparison.view.anchor_line();
        v.on_wheel(&ScrollWheelEvent { position: point(px(v.diff_x() + 50.), px(toolbar_h() + 10.)),
            delta: ScrollDelta::Lines(point(-1., -1.)), ..Default::default() }, w, cx);
        assert_eq!(v.hscroll, [0.;2], "log wheel must never scroll diff text sideways");
        assert_eq!(v.comparison.view.anchor_line(), anchor);
    }).unwrap();
}

#[test]
fn gui_sources_movement_keys_never_move_the_diff_caret() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "old words\n", "new words\n");
    let (x, y) = f.viewer.read_with(&cx, |v, _| (v.diff_x() + num_w() + 30., v.top() + line_h() / 2.));
    click(&mut cx, &f, x, y);
    let caret = f.viewer.read_with(&cx, |v, _| v.sel.unwrap());
    f.viewer.update(&mut cx, |v, _| {
        let mut p = crate::repository_session::Picker::new(f.dir.path().into(), true);
        p.focus = Pane::Sidebar;
        v.repo = Some(crate::repository_session::RepositorySession::with_picker(p));
    });
    for name in ["left", "right", "home", "end", "pageup", "pagedown"] {
        key(&mut cx, &f, name, false, false, false);
        assert_eq!(f.viewer.read_with(&cx, |v, _| v.sel), Some(caret), "{name}");
    }
}

#[test]
fn gui_prompt_swallows_wheel_before_underlying_panes() {
    let mut cx = TestAppContext::single();
    let f = files_fixture(&mut cx);
    key(&mut cx, &f, "g", true, false, false);
    f.window.update(&mut cx, |v, w, cx| {
        let anchor = v.comparison.view.anchor_line();
        v.on_wheel(&ScrollWheelEvent { position: point(px(v.diff_x() + 50.), px(v.top() + 10.)),
            delta: ScrollDelta::Lines(point(0., -1.)), ..Default::default() }, w, cx);
        assert_eq!(v.comparison.view.anchor_line(), anchor);
    }).unwrap();
}

#[test]
fn gui_result_panel_movement_does_not_move_the_diff_caret() {
    let mut cx = TestAppContext::single();
    let f = files_fixture(&mut cx);
    let (x, y) = f.viewer.read_with(&cx, |v, _| (v.diff_x() + num_w() + 30., v.top() + line_h() / 2.));
    click(&mut cx, &f, x, y);
    let caret = f.viewer.read_with(&cx, |v, _| v.sel.unwrap());
    f.viewer.update(&mut cx, |v, _| {
        let mut search = SetSearch::new(String::new(), false, Default::default());
        search.editing = false;
        v.search = Some(search);
    });
    let y = f.window.update(&mut cx, |v, _, _| v.panes.viewport[1] - v.panel_h() + line_h() + 2.).unwrap();
    click(&mut cx, &f, x, y);
    for name in ["left", "right", "up", "down", "home", "end", "pageup", "pagedown"] {
        key(&mut cx, &f, name, false, false, false);
        assert_eq!(f.viewer.read_with(&cx, |v, _| v.sel), Some(caret), "{name}");
    }
}

#[test]
fn gui_independent_diff_keyboard_and_wheel_use_their_own_pane() {
    let mut cx = TestAppContext::single();
    let f = files_fixture(&mut cx);
    f.viewer.update(&mut cx, |v, _| v.comparison.view.set_mode(ScrollMode::Aligned));
    let right = f.viewer.read_with(&cx, |v, _| v.comparison.view.anchor_line_on(Side::Right));
    key(&mut cx, &f, "tab", false, false, false);
    key(&mut cx, &f, "down", false, false, false);
    assert!((f.viewer.read_with(&cx, |v, _| v.comparison.view.anchor_line_on(Side::Right)) - right - 1.).abs() < 0.0001);
    f.window.update(&mut cx, |v, w, cx| {
        let left = v.comparison.view.anchor_line_on(Side::Left);
        v.on_wheel(&ScrollWheelEvent { position: point(px(v.diff_x() + 40.), px(v.top() + 10.)),
            delta: ScrollDelta::Lines(point(0., -1.)), ..Default::default() }, w, cx);
        assert!((v.comparison.view.anchor_line_on(Side::Left) - left - 3.).abs() < 0.0001);
        let anchors = [v.comparison.view.anchor_line_on(Side::Left), v.comparison.view.anchor_line_on(Side::Right)];
        v.on_wheel(&ScrollWheelEvent { position: point(px(20.), px(v.top() + 10.)),
            delta: ScrollDelta::Lines(point(0., -1.)), ..Default::default() }, w, cx);
        assert!(v.nav_scroll > 0.);
        assert_eq!([v.comparison.view.anchor_line_on(Side::Left), v.comparison.view.anchor_line_on(Side::Right)], anchors);
    }).unwrap();
}

#[test]
fn gui_tab_cycle_skips_hidden_repository_panes_and_reaches_files() {
    let mut cx = TestAppContext::single();
    let f = files_fixture(&mut cx);
    f.viewer.update(&mut cx, |v, _| {
        let mut p = crate::repository_session::Picker::new(f.dir.path().into(), true);
        p.sidebar = false; p.log_shown = false; p.focus = Pane::Diff;
        v.repo = Some(crate::repository_session::RepositorySession::with_picker(p));
    });
    key(&mut cx, &f, "tab", false, true, false);
    key(&mut cx, &f, "down", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.comparison.current), 1);
    key(&mut cx, &f, "tab", false, false, false);
    key(&mut cx, &f, "tab", false, false, false);
    key(&mut cx, &f, "tab", false, false, false);
    key(&mut cx, &f, "down", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.comparison.current), 2);
}

#[test]
fn gui_clicking_an_empty_diff_side_releases_the_other_sides_caret() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "old words\n", "new words\n");
    let (left_x, right_x, y) = f.window.update(&mut cx, |v, w, _| {
        (v.diff_x() + 40., f32::from(w.viewport_size().width) - 40., v.top())
    }).unwrap();
    click(&mut cx, &f, left_x, y + line_h() / 2.);
    assert!(f.viewer.read_with(&cx, |v, _| v.sel.is_some()));
    click(&mut cx, &f, right_x, y + 4. * line_h());
    assert!(f.viewer.read_with(&cx, |v, _| v.sel.is_none()));
}

#[test]
fn gui_parity_real_keys_keep_single_line_fold_disabled_and_fold_multiline_rows() {
    let mut cx = TestAppContext::single();
    let single = fixture(&mut cx, "old\nsame\n", "new\nsame\n");
    block(&mut cx, &single, 0);
    key(&mut cx, &single, "x", false, false, false);
    assert!(
        !single
            .viewer
            .read_with(&cx, |v, _| v.comparison.view.folded(0))
    );
    key(&mut cx, &single, ":", false, false, false);
    assert!(
        single
            .viewer
            .read_with(&cx, |v, _| v.command_palette.as_ref().unwrap().rows().any(
                |r| r.entry.action == crate::catalog::Action::Comparison(Command::ToggleFold)
                    && r.disabled.as_deref().is_some_and(|why| why.contains("no multiline change selected"))
            ))
    );
    key(&mut cx, &single, "escape", false, false, false);
    let multi = fixture(&mut cx, "old1\nold2\nold3\nsame\n", "new1\nnew2\nsame\n");
    block(&mut cx, &multi, 0);
    key(&mut cx, &multi, "x", false, false, false);
    assert!(multi.viewer.read_with(&cx, |v, _| {
        v.comparison
            .view
            .frame()
            .left
            .iter()
            .any(|r| matches!(r.content, Content::Folded { .. }))
    }));
    key(&mut cx, &multi, "x", false, false, false);
    assert!(
        !multi
            .viewer
            .read_with(&cx, |v, _| v.comparison.view.folded(0))
    );
    run(&mut cx,&multi,crate::catalog::Action::Comparison(Command::ToggleFold));
    assert!(
        multi
            .viewer
            .read_with(&cx, |v, _| v.comparison.view.folded(0))
    );
    key(&mut cx, &multi, "x", false, true, false);
    assert!(
        !multi
            .viewer
            .read_with(&cx, |v, _| v.comparison.view.folded(0))
    );
}

#[test]
fn gui_parity_real_options_and_find_select_exact_unicode_bytes_and_painted_range() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "before\n", "café 👩‍💻 café\n");
    key(&mut cx, &f, "w", false, false, false);
    assert_eq!(
        f.viewer
            .read_with(&cx, |v, _| v.comparison.options.whitespace),
        WhitespaceMode::Trim
    );
    key(&mut cx, &f, "h", false, false, false);
    assert_eq!(
        f.viewer
            .read_with(&cx, |v, _| v.comparison.options.highlight),
        HighlightLevel::Char
    );
    key(&mut cx, &f, "i", false, false, false);
    assert!(
        f.viewer
            .read_with(&cx, |v, _| v.comparison.options().ignore_case)
    );
    key(&mut cx, &f, "s", false, false, false);
    assert_eq!(
        f.viewer.read_with(&cx, |v, _| v.comparison.view.mode()),
        ScrollMode::Aligned
    );
    key(&mut cx, &f, "tab", false, false, false); // focus the right text Side
    key(&mut cx, &f, "f", true, false, false);
    for c in "café".chars() {
        key(&mut cx, &f, &c.to_string(), false, false, false);
    }
    f.viewer.read_with(&cx, |v, _| {
        let selected = v.sel.unwrap();
        assert_eq!((selected.anchor.col, selected.head.col), (0, 5));
        assert_eq!(selected.text(v.raw(Side::Right)), "café");
        let rows = resolve(&v.palette,
            &v.comparison.view.frame().right,
            v.raw(Side::Right),
            None,
            v.sel.as_ref(),
            &v.found(Side::Right, &v.comparison.view.frame().right),
        );
        assert!(rows.iter().filter_map(|r| r.line.as_ref()).any(|r| {
            r.selected == Some(0..5)
                && r.bg
                    .iter()
                    .any(|(range, color)| *range == (0..5) && *color == v.palette.found_bg)
        }));
    });
    key(&mut cx, &f, "f3", false, false, false);
    f.viewer.read_with(&cx, |v, _| {
        let sel = v.sel.unwrap();
        assert_eq!(sel.text(v.raw(Side::Right)), "café");
        assert_eq!(sel.anchor.col, 18);
        assert_eq!(sel.head.col, 23);
    });
    key(&mut cx, &f, "escape", false, false, false);
    assert!(f.viewer.read_with(&cx, |v, _| v.comparison.find.is_none()));
    assert_eq!(text(&f), "café 👩‍💻 café\n");
}

#[test]
fn gui_parity_apply_menu_selective_history_undo_redo_write_real_files() {
    let left = "a\ninsert one\ninsert two\nb\nc\nd\ne\nf\ng\nh\ni\n";
    let right = "a\nb\nc\nd\ne\nf\nG\nh\ni\n";
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, left, right);
    block(&mut cx, &f, 1);
    key(&mut cx, &f, "right", false, true, true);
    block(&mut cx, &f, 0);
    run(&mut cx,&f,crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::ApplyRight));
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), left);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.edits.applied(0).len()), 2);
    run(&mut cx,&f,crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::History));
    let (x, y) = f
        .window
        .update(&mut cx, |v, _, _| {
            (
                v.diff_x() + 20.,
                v.panes.viewport[1] - v.panel_h() + line_h() + 2.,
            )
        })
        .unwrap();
    click(&mut cx, &f, x, y);
    let selective = left.replace("\ng\n", "\nG\n");
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), selective);
    assert!(f.viewer.read_with(&cx, |v, _| !v.edits.applied(0)[0].active
        && v.edits.applied(0)[1].active));
    key(&mut cx, &f, "z", true, false, false);
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), left);
    key(&mut cx, &f, "z", true, true, false);
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), selective);
    for _ in 0..3 {
        key(&mut cx, &f, "z", true, false, false);
    }
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), right);
    for _ in 0..3 {
        key(&mut cx, &f, "z", true, true, false);
    }
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), selective);
}

#[test]
fn gui_parity_stale_save_refuses_overwrite_and_retains_draft() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "new\n", "old\n");
    block(&mut cx, &f, 0);
    std::fs::write(f.dir.path().join("right.txt"), "external bytes\n").unwrap();
    key(&mut cx, &f, "right", false, true, true);
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), "external bytes\n");
    assert!(f.viewer.read_with(&cx, |v, _| {
        v.write_error
            .as_deref()
            .unwrap()
            .contains("nothing written")
    }));
    assert_eq!(
        f.viewer
            .read_with(&cx, |v, _| v.doc().unwrap().text_right.to_string()),
        "new\n"
    );
    assert!(
        f.viewer
            .read_with(&cx, |v, _| v.edits.dirty(0) && v.edits.applied(0).len()==1)
    );
    run(&mut cx,&f,crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::Discard));
    assert!(!f.viewer.read_with(&cx,|v,_|v.edits.dirty(0)));
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.doc().unwrap().text(Side::Right).to_string()),"external bytes\n");
}

#[test]
fn gui_draft_undo_redo_save_and_discard_have_explicit_disk_boundaries() {
    use crate::edit_session::EditMenuCommand;
    let mut cx=TestAppContext::single();let f=fixture(&mut cx,"left\n","right\n");
    key(&mut cx,&f,"right",false,true,true);
    assert_eq!(text(&f),"right\n");
    f.viewer.read_with(&cx,|v,_|{assert!(v.edits.dirty(0));assert_eq!(v.doc().unwrap().text(Side::Right),"left\n");});
    key(&mut cx,&f,"z",true,false,false);
    assert_eq!(text(&f),"right\n");
    assert_eq!(f.viewer.read_with(&cx,|v,_|v.doc().unwrap().text(Side::Right).to_string()),"right\n");
    key(&mut cx,&f,"z",true,true,false);
    assert_eq!(text(&f),"right\n");
    f.window.update(&mut cx,|v,w,cx|v.tool(Tool::Run(crate::catalog::Action::Edit(EditMenuCommand::Save)),0.,w,cx)).unwrap();
    cx.executor().run_until_parked();
    assert_eq!(text(&f),"left\n");assert!(!f.viewer.read_with(&cx,|v,_|v.edits.has_dirty()));
    key(&mut cx,&f,"z",true,false,false);
    assert_eq!(text(&f),"left\n");
    f.viewer.read_with(&cx,|v,_|{assert!(v.edits.dirty(0));assert_eq!(v.doc().unwrap().text(Side::Right),"right\n");});
    run(&mut cx,&f,crate::catalog::Action::Edit(EditMenuCommand::Discard));
    f.viewer.read_with(&cx,|v,_|{assert!(!v.edits.dirty(0));assert_eq!(v.doc().unwrap().text(Side::Right),"left\n");});
    assert_eq!(text(&f),"left\n");
}

#[gpui::test]
fn gui_dirty_draft_survives_switch_eviction_refresh_and_blocks_replacement_or_close(cx:&mut TestAppContext) {
    let f=files_fixture(cx);
    let path=f.viewer.read_with(cx,|v,_|match &v.set.comparisons[0].right{Source::File(p)=>p.clone(),_=>panic!()});
    let bytes=std::fs::read(&path).unwrap();
    key(cx,&f,"right",false,true,true);
    assert!(f.viewer.read_with(cx,|v,_|v.edits.dirty(0)));
    key(cx,&f,"]",false,false,false);
    f.viewer.update(cx,|v,_|{v.loaded.remove(&0);v.comparison.cache.0.remove(&0);});
    key(cx,&f,"[",false,false,false);
    workflow_wait(cx,&f,|v|v.current==0&&v.doc().is_some_and(|d|d.text(Side::Right)=="old\n".repeat(100))&&v.diffed==v.generation);
    let spec=f.viewer.read_with(cx,|v,_|v.spec.clone());
    let fresh=ChangeSet::load(&spec,f.dir.path()).unwrap();
    f.viewer.update(cx,|v,cx|v.accept_repository_refresh(fresh,spec,cx));
    workflow_wait(cx,&f,|v|v.doc().is_some_and(|d|d.text(Side::Right)=="old\n".repeat(100))&&v.diffed==v.generation);
    let before=f.viewer.read_with(cx,|v,_|v.set.clone());let empty=ChangeSet::load(&Spec::Empty,f.dir.path()).unwrap();
    f.viewer.update(cx,|v,cx|{v.show_change_set(empty,cx);assert!(!v.edits_may_close(cx));});
    f.viewer.read_with(cx,|v,_|{assert!(Arc::ptr_eq(&v.set,&before));assert!(v.edits.has_dirty());assert!(v.write_error.as_ref().unwrap().contains("Unsaved"));});
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
    run(cx,&f,crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::Discard));
    assert!(f.viewer.update(cx,|v,cx|v.edits_may_close(cx)));
    assert_eq!(std::fs::read(&path).unwrap(),bytes);
}

#[gpui::test]
fn gui_reload_keeps_dirty_side_and_refreshes_clean_side_before_next_apply(cx:&mut TestAppContext) {
    let f=fixture(cx,"left\n","right\n");key(cx,&f,"left",false,true,true);
    assert!(f.viewer.read_with(cx,|v,_|v.edits.dirty(0)));
    std::fs::write(f.dir.path().join("right.txt"),"external\n").unwrap();
    run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));
    workflow_wait(cx,&f,|v|v.doc().is_some_and(|d|d.text(Side::Left)=="right\n" && d.text(Side::Right)=="external\n")&&v.diffed==v.generation);
    key(cx,&f,"right",false,true,true);
    assert_eq!(text(&f),"external\n");
    key(cx,&f,"s",true,false,false);
    assert_eq!(text(&f),"right\n");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("left.txt")).unwrap(),"right\n");
    assert!(!f.viewer.read_with(cx,|v,_|v.edits.has_dirty()));
}

#[gpui::test]
fn gui_missing_external_file_keeps_dirty_text_draft_visible(cx:&mut TestAppContext) {
    let f=fixture(cx,"draft\n","disk\n");key(cx,&f,"right",false,true,true);
    std::fs::remove_file(f.dir.path().join("right.txt")).unwrap();
    run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));
    workflow_wait(cx,&f,|v|v.doc().is_some_and(|d|d.placeholder.is_none() && d.text(Side::Right)=="draft\n")&&v.diffed==v.generation);
    key(cx,&f,"s",true,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.edits.dirty(0)));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.doc().unwrap().text(Side::Right).to_string()),"draft\n");
    assert!(!f.dir.path().join("right.txt").exists());
}

#[gpui::test]
fn gui_revert_hunk_is_a_draft_and_whole_file_discard_rejects_pending_edits(cx:&mut TestAppContext) {
    use crate::{edit_session::EditMenuCommand,file_actions::FileAction};
    let f=discard_fixture(cx);run(cx,&f,crate::catalog::Action::Edit(EditMenuCommand::Revert));
    assert_eq!(text(&f),"working\n");
    f.viewer.read_with(cx,|v,_|{assert!(v.edits.dirty(0));assert_eq!(v.doc().unwrap().text(Side::Right),"indexed\n");assert!(v.file_disabled(FileAction::Discard,0).unwrap().contains("unsaved"));});
    run(cx,&f,crate::catalog::Action::File(FileAction::Discard));
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()),"a disabled entry keeps the palette open with its reason");
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.file_actions.confirmation().is_none()&&!v.file_actions.busy()));
    assert_eq!(text(&f),"working\n");
    key(cx,&f,"s",true,false,false);assert_eq!(text(&f),"indexed\n");
    assert_eq!(crate::git::run(f.dir.path(),&["show",":right.txt"]).unwrap(),b"indexed\n");
    run(cx,&f,crate::catalog::Action::Edit(EditMenuCommand::Undo));assert_eq!(text(&f),"indexed\n");
    key(cx,&f,"s",true,false,false);assert_eq!(text(&f),"working\n");
}

#[test]
fn gui_parity_go_to_unicode_caret_and_keyboard_menu_keep_semantic_positions() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "old\n", "aé👩‍💻z\n");
    key(&mut cx, &f, "g", true, false, false);
    for c in "1:3".chars() {
        key(&mut cx, &f, &c.to_string(), false, false, false);
    }
    key(&mut cx, &f, "enter", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.sel.unwrap().head.col), 3);
    key(&mut cx, &f, "right", false, false, false);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.sel.unwrap().head.col), 7);
    key(&mut cx, &f, "left", false, true, false);
    assert_eq!(
        f.viewer
            .read_with(&cx, |v, _| v.sel.unwrap().text(v.raw(Side::Right))),
        "👩"
    );
    key(&mut cx, &f, ":", false, false, false);
    key(&mut cx, &f, "end", false, false, false);
    f.viewer.read_with(&cx, |v, _| {
        let p = v.command_palette.as_ref().unwrap();
        assert_eq!(p.selected(), Some(p.len() - 1));
    });
    key(&mut cx, &f, "home", false, false, false);
    key(&mut cx, &f, "pagedown", false, false, false);
    assert!(
        f.viewer
            .read_with(&cx, |v, _| v.command_palette.as_ref().unwrap().selected().is_some_and(|row| row > 0)),
        "PageDown moves down a page"
    );
    key(&mut cx, &f, "escape", false, false, false);
    assert!(f.viewer.read_with(&cx, |v, _| v.command_palette.is_none()));
}

#[test]
fn gui_parity_sidebar_click_outside_applied_panel_cannot_undo_file() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "new\n", "old\n");
    block(&mut cx, &f, 0);
    key(&mut cx, &f, "right", false, true, true);
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f), "new\n");
    f.viewer.update(&mut cx, |v, _| {
        v.layout.set_files(false);
        v.repo = Some(crate::repository_session::RepositorySession::with_picker(crate::repository_session::Picker::new(
            f.dir.path().into(),
            true,
        )));
    });
    let y = f
        .window
        .update(&mut cx, |v, _, _| {
            v.panes.viewport[1] - v.panel_h() + line_h() + 2.
        })
        .unwrap();
    click(&mut cx, &f, 20., y);
    assert_eq!(
        text(&f),
        "new\n",
        "a sidebar click has no applied-history action"
    );
    assert!(f.viewer.read_with(&cx, |v, _| v.edits.applied(0)[0].active));
}

#[test]
fn gui_palette_option_entries_execute_shared_set_commands() {
    let mut cx=TestAppContext::single();let f=fixture(&mut cx," old \n","old\n");
    for command in [Command::SetWhitespace(WhitespaceMode::Trim),Command::SetHighlight(HighlightLevel::Char),Command::SetScroll(ScrollMode::Aligned)] {
        run(&mut cx,&f,crate::catalog::Action::Comparison(command));
    }
    f.viewer.read_with(&cx,|v,_|{
        assert_eq!(v.comparison.options.whitespace,WhitespaceMode::Trim);
        assert_eq!(v.comparison.options.highlight,HighlightLevel::Char);
        assert_eq!(v.comparison.view.mode(),ScrollMode::Aligned);
        assert!(v.command_palette.is_none());
    });
}

#[test]
fn gui_change_set_search_is_canceled_before_rendering_a_smaller_selection() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "old\n", "new\n");
    let mut sets = Vec::new();
    for (name, count) in [("large", 7), ("small", 4)] {
        let left = f.dir.path().join(format!("{name}-left"));
        let right = f.dir.path().join(format!("{name}-right"));
        std::fs::create_dir(&left).unwrap();
        std::fs::create_dir(&right).unwrap();
        for i in 0..count {
            std::fs::write(left.join(format!("{i}.txt")), "old\n").unwrap();
            std::fs::write(right.join(format!("{i}.txt")), if i==6{"needle\n"}else{"other\n"}).unwrap();
        }
        sets.push(ChangeSet::load(&Spec::Plain { left, right }, f.dir.path()).unwrap());
    }
    f.viewer.update(&mut cx, |v,cx| v.show_change_set(sets.remove(0),cx));
    key(&mut cx,&f,"f",true,true,false);
    for ch in "needle".chars() {key(&mut cx,&f,&ch.to_string(),false,false,false);}
    key(&mut cx,&f,"enter",false,false,false);
    let until = Instant::now() + std::time::Duration::from_secs(10);
    while !f.viewer.read_with(&cx, |v,_| v.search.as_ref().is_some_and(|s|s.hits.iter().any(|h|h.index==6))) {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.executor().run_until_parked();
        assert!(Instant::now()<until,"real search did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let token=f.viewer.read_with(&cx,|v,_|v.search.as_ref().unwrap().cancel.clone());
    f.viewer.update(&mut cx, |v,cx|v.show_change_set(sets.remove(0),cx));
    // Exercise the reported panic at the actual render call site, before checking cleanup.
    f.window.update(&mut cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    f.viewer.read_with(&cx,|v,_|assert!(v.search.is_none()));
    assert!(token.load(std::sync::atomic::Ordering::Relaxed),"superseded producer must be canceled");
    cx.executor().run_until_parked();
    f.window.update(&mut cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    let left=f.dir.path().join("large-left");
    let empty=ChangeSet::load(&Spec::Plain{left:left.clone(),right:left},f.dir.path()).unwrap();
    f.viewer.update(&mut cx,|v,cx|v.show_change_set(empty,cx));
    f.window.update(&mut cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    f.viewer.read_with(&cx,|v,_|{assert!(v.set.comparisons.is_empty());assert!(v.nav.is_empty());assert!(v.doc().is_none());assert_eq!(v.comparison.current,0);});
}

#[test]
fn gui_about_menu_blocks_edit_keys_and_closes_without_changing_files() {
    let mut cx=TestAppContext::single();
    let f=fixture(&mut cx,"old\n","new\n");
    key(&mut cx,&f,":",false,false,false);
    assert!(f.viewer.read_with(&cx,|v,_|v.command_palette.as_ref().unwrap().rows().any(|r|r.entry.action==crate::catalog::Action::Interaction(commands::Interaction::About))),"About must be available in the GUI palette");
    key(&mut cx,&f,"escape",false,false,false);
    run(&mut cx,&f,crate::catalog::Action::Interaction(commands::Interaction::About));
    key(&mut cx,&f,"right",false,true,true);
    assert_eq!(text(&f),"new\n","About must not pass edits through to the comparison");
    key(&mut cx,&f,"escape",false,false,false);
    key(&mut cx,&f,"right",false,true,true);
    key(&mut cx,&f,"s",true,false,false);
    assert_eq!(text(&f),"old\n","dismissing About must restore comparison controls");
}

#[test]
fn gui_report_command_is_reachable_without_editing_the_document() {
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "old\n", "new\n");
    key(&mut cx, &f, ":", false, false, false);
    assert!(f.viewer.read_with(&cx, |v, _| v.command_palette.as_ref().unwrap().rows().any(|r| r.entry.action == crate::catalog::Action::Interaction(commands::Interaction::ReportLastError))));
    key(&mut cx, &f, "escape", false, false, false);
    run(&mut cx, &f, crate::catalog::Action::Interaction(commands::Interaction::ReportLastError));
    assert_eq!(text(&f), "new\n");
    assert!(f.viewer.read_with(&cx, |v, _| v.menu.is_none() && v.command_palette.is_none()));
}

#[test]
fn gui_report_saved_child() {
    let Ok(mode) = std::env::var("LENZ_TEST_GUI_REPORT") else { return };
    crate::diagnostics::install("GUI");
    if mode == "saved" { crate::diagnostics::failure("Read comparison", "SECRET_PANIC_TEXT", "Check file permissions."); }
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "old\n", "new\n");
    if mode == "reload" {
        std::fs::remove_file(f.dir.path().join("right.txt")).unwrap();
        run(&mut cx, &f, crate::catalog::Action::Interaction(commands::Interaction::Reload));
        let until = Instant::now() + std::time::Duration::from_secs(5);
        while f.viewer.read_with(&cx, |v, _| !v.doc().is_some_and(|doc| matches!(doc.placeholder_kind(), Some(Placeholder::Unreadable(_))))) {
            cx.executor().advance_clock(std::time::Duration::from_millis(20));
            cx.executor().run_until_parked();
            assert!(Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let saved = crate::diagnostics::last_report().expect("failed GUI reload must save diagnostics");
        let report = std::fs::read_to_string(saved.path).unwrap();
        assert!(report.contains("right.txt"));
        assert!(report.contains("Read comparison"));
        return;
    }
    run(&mut cx, &f, crate::catalog::Action::Interaction(commands::Interaction::ReportLastError));
    let until = Instant::now() + std::time::Duration::from_secs(5);
    while f.viewer.read_with(&cx, |v, _| v.report_request.busy()) {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));
        cx.executor().run_until_parked();
        assert!(Instant::now() < until, "GUI report lookup did not complete");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if mode == "saved" {
        let url = cx.opened_url().expect("the real command must open an issue draft");
        assert!(url.starts_with("https://github.com/aantoniadis/lenz/issues/new?"));
        assert!(!url.contains("SECRET_PANIC_TEXT"));
        assert!(f.viewer.read_with(&cx, |v, _| v.toast.as_ref().unwrap().0.contains("Review report:")));
    } else {
        assert!(cx.opened_url().is_none());
        assert!(f.viewer.read_with(&cx, |v, _| v.toast.as_ref().unwrap().0.contains("No saved Lenz error report")));
    }
    assert_eq!(text(&f), "new\n");
}

#[test]
fn gui_report_menu_opens_a_draft_or_explains_missing_diagnostics() {
    for mode in ["saved", "missing", "reload"] {
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ui::parity_tests::gui_report_saved_child", "--nocapture"])
            .env("LENZ_TEST_GUI_REPORT", mode).env("LENZ_LOG_DIR", directory.path()).output().unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
}

#[gpui::test]
fn gui_about_logo_is_lazy_and_released_on_close(cx: &mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    assert!(f.viewer.read_with(cx,|v,_|v.about.is_none()));
    f.window.update(cx,|v,w,cx|v.tool(Tool::About,0.0,w,cx)).unwrap();
    let weak=f.viewer.read_with(cx,|v,_|Arc::downgrade(&v.about.as_ref().unwrap().image));
    assert!(weak.upgrade().is_some());
    f.window.update(cx,|v,w,cx|{
        let close=about::About::close_bounds(w.viewport_size());
        let ev=MouseDownEvent{position:close.origin+point(px(2.0),px(2.0)),button:MouseButton::Left,..Default::default()};
        v.on_mouse_down(&ev,w,cx);
    }).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.about.is_none()));
    assert!(weak.upgrade().is_none(),"closed About must release its RenderImage");
    assert_eq!(text(&f),"new\n");
}

#[gpui::test]
fn gui_about_scroll_end_then_up_remains_bounded(cx: &mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    cx.simulate_window_resize(f.window.into(),size(px(300.0),px(150.0)));
    cx.executor().run_until_parked();
    f.viewer.update(cx,|v,_|v.char_w=8.0);
    f.window.update(cx,|v,w,cx|v.tool(Tool::About,0.0,w,cx)).unwrap();
    key(cx,&f,"end",false,false,false);
    let end=f.viewer.read_with(cx,|v,_|v.about.as_ref().unwrap().scroll);
    assert!(end>0&&end<100,"End must store a finite viewport scroll position");
    key(cx,&f,"up",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.about.as_ref().unwrap().scroll),end.saturating_sub(1));
    for _ in 0..100{key(cx,&f,"down",false,false,false);}
    assert_eq!(f.viewer.read_with(cx,|v,_|v.about.as_ref().unwrap().scroll),end);
    f.window.update(cx,|v,w,cx|v.on_wheel(&ScrollWheelEvent{delta:ScrollDelta::Lines(point(0.0,1.0)),..Default::default()},w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.about.as_ref().unwrap().scroll),end.saturating_sub(3));
}

#[gpui::test]
fn gui_about_empty_view_remains_usable_after_close(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","new\n");
    let empty=ChangeSet::load(&Spec::Empty,f.dir.path()).unwrap();
    f.viewer.update(cx,|v,cx|{
        let doc=Arc::new(Doc::new("","",Path::new(""),Path::new("")));
        let view=doc.view(&[],30.0);
        *v=Viewer::new(Arc::new(empty),Spec::Empty,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);
    });
    assert!(f.viewer.read_with(cx,|v,_|v.nothing.is_some()),"the empty launcher must not show a perpetual loading state");
    f.window.update(cx,|v,w,cx|{v.tool(Tool::About,0.0,w,cx);let _=v.render(w,cx);}).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.about.is_some()));
    key(cx,&f,"escape",false,false,false);
    key(cx,&f,"z",true,false,false);
    run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));
    f.window.update(cx,|v,w,cx|{v.reload(cx);let _=v.render(w,cx);}).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.about.is_none()&&v.set.comparisons.is_empty()&&v.doc().is_none()));
}

#[gpui::test]
async fn gui_theme_menu_switches_without_writing(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","new\n");
    run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::ChangeTheme));
    let row=f.viewer.read_with(cx,|v,_|v.theme_chooser.as_ref().unwrap().entries.iter().position(|e|e.id=="catppuccin-latte").unwrap());
    key(cx,&f,"home",false,false,false);for _ in 0..row{key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);
    let snapshot=cx.update(|cx|cx.global::<themes::GuiThemes>().0.snapshot());
    assert_eq!(snapshot.selection,theme::ThemeSelection::Named("catppuccin-latte".into()));assert_eq!(snapshot.palette.bg,0xeff1f5);assert_eq!(text(&f),"new\n");
}

#[gpui::test]
fn gui_theme_presets_repaint_independent_views_without_reloading(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","new\n");let other=fixture(cx,"a\n","b\n");
    let doc=f.viewer.read_with(cx,|v,_|v.doc().unwrap().clone());
    for entry in theme::builtin_entries(){
        f.window.update(cx,|v,w,cx|v.tool(Tool::Theme,0.0,w,cx)).unwrap();
        let row=f.viewer.read_with(cx,|v,_|v.theme_chooser.as_ref().unwrap().entries.iter().position(|e|e.id==entry.id).unwrap());
        key(cx,&f,"home",false,false,false);for _ in 0..row{key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.palette.bg),entry.palette.bg);
        assert_eq!(other.viewer.read_with(cx,|v,_|v.palette.bg),entry.palette.bg);
        assert!(f.viewer.read_with(cx,|v,_|Arc::ptr_eq(v.doc().unwrap(),&doc)));
    }
    assert_eq!(text(&f),"new\n");assert_eq!(text(&other),"b\n");
}

#[gpui::test]
fn gui_theme_modal_blocks_save_and_apply_and_clamps_after_resize(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","new\n");
    f.window.update(cx,|v,w,cx|v.tool(Tool::Theme,0.0,w,cx)).unwrap();
    for size in [(300.0,150.0),(1.0,1.0),(600.0,220.0)]{
        cx.simulate_window_resize(f.window.into(),gpui::size(px(size.0),px(size.1)));cx.executor().run_until_parked();
        key(cx,&f,"end",false,false,false);key(cx,&f,"up",false,false,false);
        key(cx,&f,"s",true,false,false);key(cx,&f,"right",false,true,true);
        f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
        assert!(f.viewer.read_with(cx,|v,_|{let c=v.theme_chooser.as_ref().unwrap();c.selected<c.entries.len()&&c.scroll<=c.entries.len()}));
    }
    key(cx,&f,"escape",false,false,false);assert!(f.viewer.read_with(cx,|v,_|v.theme_chooser.is_none()));assert_eq!(text(&f),"new\n");
    f.window.update(cx,|v,w,cx|{v.tool(Tool::About,0.0,w,cx);v.tool(Tool::Theme,0.0,w,cx);}).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.about.is_some()&&v.theme_chooser.is_none()));
}

#[gpui::test]
fn gui_theme_mouse_selects_a_clipped_hit_row(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","new\n");f.window.update(cx,|v,w,cx|{v.tool(Tool::Theme,0.0,w,cx);let _=v.render(w,cx);}).unwrap();
    let at=f.viewer.read_with(cx,|v,_|{let c=v.theme_chooser.as_ref().unwrap();let index=c.entries.iter().position(|e|e.id=="gruvbox").unwrap();c.hits.iter().find(|(i,_)|*i==index).unwrap().1.origin});
    click(cx,&f,f32::from(at.x)+2.0,f32::from(at.y)+2.0);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.palette.bg),0x282828);assert_eq!(text(&f),"new\n");
}

#[gpui::test]
fn gui_theme_empty_constructor_and_merge_question_keep_documents_safe(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","new\n");let empty=Arc::new(ChangeSet::load(&Spec::Empty,f.dir.path()).unwrap());
    f.viewer.update(cx,|v,cx|{let doc=Arc::new(Doc::new("","",Path::new(""),Path::new("")));let view=doc.view(&[],20.0);*v=Viewer::new(empty,Spec::Empty,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);});
    let last=theme::builtin_entries().last().unwrap().palette.bg;
    run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::ChangeTheme));key(cx,&f,"end",false,false,false);key(cx,&f,"enter",false,false,false);f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();assert!(f.viewer.read_with(cx,|v,_|v.doc().is_none()&&v.palette.bg==last));
    let(dir,viewer)=merge_view::gui_tests::fixture(cx);let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
    window.update(cx,|v,w,cx|{v.open_themes(cx);v.on_key(&event("s",true,false,false),w,cx);v.on_key(&event("escape",false,false,false),w,cx);v.merge.as_mut().unwrap().ask=Some(merge_view::Ask::ChangedOnDisk);v.tool(Tool::Theme,0.0,w,cx);assert!(v.theme_chooser.is_none());}).unwrap();assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"needle ours\n");
}

#[gpui::test]
fn gui_theme_worker_updates_repaint_and_show_source_only_changes(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","new\n");let palette=theme::builtin("catppuccin").unwrap();
    for source in ["first","renamed"]{cx.update(|cx|cx.update_global::<themes::GuiThemes,_>(|g,_|{let effect=g.0.apply(theme::ThemeUpdate::System{request:1,theme:Some(theme::LoadedTheme{name:source.into(),palette:*palette}),notice:None});assert!(effect.repaint);}));cx.executor().run_until_parked();assert_eq!(f.viewer.read_with(cx,|v,_|v.palette.bg),palette.bg);}
    let revision=cx.update(|cx|cx.global::<themes::GuiThemes>().0.snapshot().revision);f.window.update(cx,|v,_,cx|v.open_themes(cx)).unwrap();
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();assert_eq!(cx.update(|cx|cx.global::<themes::GuiThemes>().0.snapshot().revision),revision);assert_eq!(text(&f),"new\n");
}

#[gpui::test]
fn gui_theme_apply_undo_reload_preserves_semantics(cx: &mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    let choose=|cx:&mut TestAppContext,id:&str| {
        run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::ChangeTheme));
        let row=f.viewer.read_with(cx,|v,_|v.theme_chooser.as_ref().unwrap().entries.iter().position(|e|e.id==id).unwrap());
        key(cx,&f,"home",false,false,false);
        for _ in 0..row{key(cx,&f,"down",false,false,false);}
        key(cx,&f,"enter",false,false,false);
    };
    choose(cx,"catppuccin");block(cx,&f,0);
    key(cx,&f,"right",false,true,true);key(cx,&f,"s",true,false,false);assert_eq!(text(&f),"old\n");
    choose(cx,"gruvbox");key(cx,&f,"z",true,false,false);key(cx,&f,"s",true,false,false);assert_eq!(text(&f),"new\n");
    choose(cx,"catppuccin-latte");
    std::fs::write(f.dir.path().join("right.txt"),"external\n").unwrap();
    run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));
    let until=Instant::now()+std::time::Duration::from_secs(5);
    while !f.viewer.read_with(cx,|v,_|v.doc().is_some_and(|d|d.text_right.as_ref()=="external\n")) {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));cx.executor().run_until_parked();
        assert!(Instant::now()<until,"GUI reload did not finish");std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(text(&f),"external\n");assert_eq!(f.viewer.read_with(cx,|v,_|v.palette.bg),0xeff1f5);
}

#[test]
fn gui_option_recompare_keeps_expanded_rows_and_both_scroll_positions() {
    let mut cx=TestAppContext::single();let left=(0..200).map(|i|format!("line {i}\n")).collect::<String>();let right=left.replace("line 80\n","edited 80\n");let f=fixture(&mut cx,&left,&right);
    let before=f.viewer.update(&mut cx,|v,_|{
        v.comparison.view.set_collapsed(true);v.comparison.view.set_mode(ScrollMode::Aligned);
        let fold=v.comparison.view.frame().left.iter().find_map(|r|if let Content::Fold{id,..}=r.content{Some(id)}else{None}).unwrap();v.comparison.view.expand(fold);
        v.comparison.view.place_line_at(Side::Left,32.25);v.comparison.view.place_line_at(Side::Right,45.75);v.comparison.view.frame()
    });
    f.viewer.update(&mut cx,|v,cx|{v.comparison_command(Command::SetWhitespace(WhitespaceMode::Ignore),cx);assert_eq!(v.comparison.view.frame(),before,"the in-flight option change must keep the painted frame");});
    cx.executor().run_until_parked();
    f.viewer.read_with(&cx,|v,_|assert_eq!(v.comparison.view.frame(),before,"unchanged diff geometry must not snap or refold on completion"));
}

#[test]
fn gui_active_option_entry_preserves_frame_and_worker_generation() {
    let mut cx=TestAppContext::single();let f=fixture(&mut cx,"old\n","new\n");
    for current in [|v:&Viewer|Command::SetWhitespace(v.comparison.options.whitespace),|v:&Viewer|Command::SetHighlight(v.comparison.options.highlight)] {
        let (generation,before,command)=f.viewer.read_with(&cx,|v,_|(v.comparison.generation,v.comparison.view.frame(),current(v)));
        run(&mut cx,&f,crate::catalog::Action::Comparison(command));
        cx.executor().run_until_parked();
        f.viewer.read_with(&cx,|v,_|{assert!(v.command_palette.is_none());assert_eq!(v.comparison.generation,generation);assert_eq!(v.comparison.view.frame(),before);});
    }
}

fn discard_fixture(cx:&mut TestAppContext)->Fixture{
    let f=fixture(cx,"head\n","staged\n");
    let root=f.dir.path();crate::git::run(root,&["init","-q"]).unwrap();crate::git::run(root,&["add","right.txt"]).unwrap();
    crate::git::run(root,&["-c","user.name=Test","-c","user.email=test@example.test","-c","core.hooksPath=/dev/null","-c","commit.gpgsign=false","commit","-qm","initial"]).unwrap();
    std::fs::write(root.join(".gitattributes"),"*.txt text eol=crlf\n").unwrap();
    std::fs::write(root.join("right.txt"),"indexed\r\n").unwrap();crate::git::run(root,&["add","right.txt"]).unwrap();std::fs::write(root.join("right.txt"),"working\n").unwrap();
    let spec=Spec::Git{cached:false,revs:Vec::new(),paths:vec!["right.txt".into()],untracked:false};let set=Arc::new(ChangeSet::load(&spec,root).unwrap());let doc=Arc::new(Doc::load(&set,0,Path::new("right.txt"),Path::new("right.txt"),false).unwrap());let view=doc.view(&doc.line_diff(&Options::default()),30.);
    f.viewer.update(cx,|v,cx|{*v=Viewer::new(set,spec,false,Some("right.md".into()),doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);v.layout.set_files(true);});f
}

#[gpui::test]
fn gui_blame_navigation_bounds_completed_cache_and_releases_pending_requests(cx: &mut TestAppContext) {
    let f = fixture(cx, "old\n", "new\n");
    workflow_git(f.dir.path(), &["init", "-q"]);
    for i in 0..12 {
        std::fs::write(f.dir.path().join(format!("file-{i:02}.txt")), "base\n").unwrap();
    }
    workflow_git(f.dir.path(), &["add", "."]);
    workflow_git(f.dir.path(), &["commit", "-qm", "Blame fixture"]);
    for i in 0..12 {
        std::fs::write(f.dir.path().join(format!("file-{i:02}.txt")), "changed\n").unwrap();
    }
    let spec = Spec::Git { cached: false, revs: Vec::new(), paths: Vec::new(), untracked: false };
    let set = Arc::new(ChangeSet::load(&spec, f.dir.path()).unwrap());
    assert_eq!(set.comparisons.len(), 12);
    let path = Path::new(&set.comparisons[0].path);
    let doc = Arc::new(Doc::load(&set, 0, path, path, false).unwrap());
    let view = doc.view(&doc.line_diff(&Options::default()), 30.);
    f.viewer.update(cx, |v, cx| {
        *v = Viewer::new(set, spec, false, None, doc, view, false, Options::default(),
            gpui::font("monospace"), Instant::now(), false, cx);
    });
    // Leave before the first UI poll: hidden unfinished work must lose its owner,
    // even when its document remains hot in the cache.
    f.window.update(cx, |v, w, cx| {
        v.on_key(&event("b", false, false, true), w, cx);
        let retired = v.blames.values().map(Blame::request_id).collect::<Vec<_>>();
        assert_eq!(retired.len(), 2);
        assert!(v.blames.values().all(Blame::busy));
        v.on_key(&event("]", false, false, false), w, cx);
        assert!(retired.iter().all(|request| request.strong_count() == 0));
        assert_eq!(v.blames.len(), 2);
    }).unwrap();
    let ready = |v: &Viewer| v.doc().is_some() && v.comparison.diffed == v.comparison.generation
        && [Side::Left, Side::Right].into_iter().all(|side| v.blame_key(side)
            .and_then(|key| v.blames.get(&key)).is_some_and(|b| !b.busy() && b.error().is_none() && b.line(0).is_some()));
    workflow_wait(cx, &f, ready);
    let cached = f.viewer.read_with(cx, |v, _| v.blames.iter()
        .map(|(key, annotations)| (key.clone(), annotations.request_id())).collect::<Vec<_>>());
    key(cx, &f, "[", false, false, false);
    workflow_wait(cx, &f, ready);
    key(cx, &f, "]", false, false, false);
    f.viewer.read_with(cx, |v, _| {
        assert_eq!(v.current, 1);
        assert!(cached.iter().all(|(key, request)| v.blames.get(key)
            .is_some_and(|annotations| !annotations.busy() && request.ptr_eq(&annotations.request_id()))),
            "a completed hot revisit must reuse annotations without a fresh 100ms poll");
    });
    for next in 2..12 {
        key(cx, &f, "]", false, false, false);
        workflow_wait(cx, &f, ready);
        f.viewer.read_with(cx, |v, _| {
            assert_eq!(v.current, next);
            assert!(v.loaded.len() <= 8);
            assert_eq!(v.blames.len(), 2 * v.loaded.len());
            assert!(v.blames.keys().all(|key| v.loaded.keys().any(|&index|
                [Side::Left, Side::Right].into_iter().any(|side| v.blame_key_at(index, side).as_ref() == Some(key)))));
        });
    }
    assert!(cached.iter().all(|(_, request)| request.strong_count() == 0), "document eviction must also release completed annotations");
    // Toggle before the old 100ms poller wakes: reopening identical keys owns new requests.
    f.window.update(cx, |v, w, cx| {
        let retired = v.blames.values().map(Blame::request_id).collect::<Vec<_>>();
        v.on_key(&event("b", false, false, true), w, cx);
        assert!(v.blames.is_empty());
        v.on_key(&event("b", false, false, true), w, cx);
        assert_eq!(v.blames.len(), 2);
        assert!(retired.iter().all(|request| request.strong_count() == 0));
        assert!(v.blames.values().all(|annotations| annotations.request_id().strong_count() == 1));
        v.on_key(&event("b", false, false, true), w, cx);
        assert!(v.blames.is_empty());
    }).unwrap();
    for i in 0..12 {
        assert_eq!(std::fs::read_to_string(f.dir.path().join(format!("file-{i:02}.txt"))).unwrap(), "changed\n");
    }
}
fn wait_file_action(cx:&mut TestAppContext,f:&Fixture){let until=Instant::now()+std::time::Duration::from_secs(5);while f.viewer.read_with(cx,|v,_|v.file_actions.busy()){
    cx.executor().advance_clock(std::time::Duration::from_millis(20));cx.executor().run_until_parked();assert!(Instant::now()<until);std::thread::sleep(std::time::Duration::from_millis(2));
}}
#[gpui::test]
fn gui_file_context_discard_confirmation_preserves_staging_and_exposes_undo(cx:&mut TestAppContext){
    use crate::file_actions::FileAction;
    let f=discard_fixture(cx);
    let y=f.viewer.read_with(cx,|v,_|v.top()+v.nav.iter().position(|r|matches!(r,NavRow::File{index:0,..})).unwrap()as f32*line_h()+2.);
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(10.),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    let row=f.viewer.read_with(cx,|v,_|v.menu.as_ref().unwrap().items.iter().position(|(_,a)|matches!(a,MenuAction::File(FileAction::Discard,0,_))).unwrap());
    key(cx,&f,"home",false,false,false);for _ in 0..row{key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);wait_file_action(cx,&f);
    assert!(f.viewer.read_with(cx,|v,_|v.file_actions.confirmation().unwrap().contains("right.txt")));
    for name in ["right","left","s","z"]{key(cx,&f,name,true,true,true);}click(cx,&f,10.,y);assert_eq!(text(&f),"working\n");
    key(cx,&f,"escape",false,false,false);assert_eq!(text(&f),"working\n");
    run(cx,&f,crate::catalog::Action::File(FileAction::Discard));wait_file_action(cx,&f);key(cx,&f,"enter",false,false,false);wait_file_action(cx,&f);
    assert_eq!(std::fs::read(f.dir.path().join("right.txt")).unwrap(),b"indexed\r\n");assert_eq!(crate::git::run(f.dir.path(),&["show",":right.txt"]).unwrap(),b"indexed\n");
    let spec=f.viewer.read_with(cx,|v,_|v.spec.clone());let refreshed=ChangeSet::load(&spec,f.dir.path()).unwrap();f.viewer.update(cx,|v,cx|v.accept_repository_refresh(refreshed,spec,cx));cx.executor().run_until_parked();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.set.comparisons.len()),1,"the discarded file must stay visible for undo after refresh");
    run(cx,&f,crate::catalog::Action::File(FileAction::UndoDiscard));wait_file_action(cx,&f);assert_eq!(text(&f),"working\n");
}
#[gpui::test]
fn gui_discard_refuses_file_changed_after_confirmation(cx:&mut TestAppContext){
    let f=discard_fixture(cx);run(cx,&f,crate::catalog::Action::File(crate::file_actions::FileAction::Discard));wait_file_action(cx,&f);
    std::fs::write(f.dir.path().join("right.txt"),"external\n").unwrap();key(cx,&f,"enter",false,false,false);wait_file_action(cx,&f);
    assert_eq!(text(&f),"external\n");assert!(f.viewer.read_with(cx,|v,_|v.toast.as_ref().unwrap().0.contains("changed on disk")));
}

#[gpui::test]
fn gui_whole_file_restore_undeletes_without_changing_the_index(cx:&mut TestAppContext) {
    use crate::file_actions::FileAction;
    let f=discard_fixture(cx);std::fs::remove_file(f.dir.path().join("right.txt")).unwrap();
    let spec=f.viewer.read_with(cx,|v,_|v.spec.clone());let set=ChangeSet::load(&spec,f.dir.path()).unwrap();
    f.viewer.update(cx,|v,cx|v.accept_repository_refresh(set,spec,cx));
    workflow_wait(cx,&f,|v|v.doc().is_some() && v.set.comparisons[0].status==Status::Deleted);
    assert!(f.viewer.read_with(cx,|v,_|v.file_disabled(FileAction::Discard,v.comparison.current).is_none()));
    run(cx,&f,crate::catalog::Action::File(FileAction::Discard));wait_file_action(cx,&f);
    assert!(f.viewer.read_with(cx,|v,_|v.file_actions.confirmation().is_some()));
    key(cx,&f,"enter",false,false,false);wait_file_action(cx,&f);
    assert_eq!(std::fs::read(f.dir.path().join("right.txt")).unwrap(),b"indexed\r\n");
    assert_eq!(crate::git::run(f.dir.path(),&["show",":right.txt"]).unwrap(),b"indexed\n");
}

#[gpui::test]
fn gui_selected_conflict_reset_keeps_other_resolution_and_disk_bytes(cx:&mut TestAppContext) {
    use crate::{merge::{BlockState,Pick},merge_session::Control};
    let (dir,viewer)=merge_view::gui_tests::fixture(cx);
    let middle="keep 1\nkeep 2\nkeep 3\nkeep 4\nkeep 5\nkeep 6\n";
    for (name,text) in [("base",format!("a\n{middle}z\n")),("ours",format!("A\n{middle}Z\n")),("theirs",format!("B\n{middle}Y\n")),("result",format!("A\n{middle}Z\n"))] {std::fs::write(dir.path().join(name),text).unwrap();}
    let opened=merge_view::open_files(&dir.path().join("base"),&dir.path().join("ours"),&dir.path().join("theirs"),&dir.path().join("result"),dir.path()).unwrap();
    let before=std::fs::read(dir.path().join("result")).unwrap();
    viewer.update(cx,|v,cx| {
        v.start_merge(opened,false,cx);
        assert_eq!(v.merge.as_ref().unwrap().m.blocks().len(),2);
        v.merge_control(Control::Accept(0,Pick::Ours),cx);v.merge_control(Control::Accept(1,Pick::Theirs),cx);
        v.merge.as_mut().unwrap().focus_block(0);
    });
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
    // per-conflict choices live on the block: its right-click menu, opened from the keyboard with Shift-F10
    window.update(cx,|v,w,cx|{let _=v.render(w,cx);v.on_key(&event("f10",false,true,false),w,cx);}).unwrap();
    let row=viewer.read_with(cx,|v,_|v.menu.as_ref().expect("block menu").items.iter().position(|(_,a)|matches!(a,MenuAction::Merge(Control::Reopen(0)))).expect("reopen listed"));
    window.update(cx,|v,w,cx|{v.on_key(&event("home",false,false,false),w,cx);for _ in 0..row{v.on_key(&event("down",false,false,false),w,cx);}v.on_key(&event("enter",false,false,false),w,cx);}).unwrap();
    viewer.read_with(cx,|v,_|{let m=&v.merge.as_ref().unwrap().m;assert_eq!(m.state(0),BlockState::Unresolved);assert_eq!(m.state(1),BlockState::Resolved);});
    assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),before);
}

#[gpui::test]
fn gui_discard_confirmation_renders_all_themes_and_narrow_sizes_without_writes(cx:&mut TestAppContext){
    let f=discard_fixture(cx);run(cx,&f,crate::catalog::Action::File(crate::file_actions::FileAction::Discard));wait_file_action(cx,&f);
    for entry in theme::builtin_entries(){
        f.viewer.update(cx,|v,_|v.palette=entry.palette.clone());
        for (width,height) in [(800.,600.),(300.,150.),(80.,80.)]{
            cx.simulate_window_resize(f.window.into(),gpui::size(px(width),px(height)));cx.executor().run_until_parked();
            f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
            key(cx,&f,"end",false,false,false);key(cx,&f,"up",false,false,false);
            assert_eq!(text(&f),"working\n");assert!(f.viewer.read_with(cx,|v,_|v.file_actions.confirmation().is_some()));
        }
    }
    key(cx,&f,"escape",false,false,false);assert!(f.viewer.read_with(cx,|v,_|v.file_actions.confirmation().is_none()));
}

#[gpui::test]
fn gui_text_context_copy_preserves_unicode_selection_and_blocks_underlying_arrows(cx:&mut TestAppContext){
    let f=fixture(cx,"old\n","α new\n");f.viewer.update(cx,|v,_|v.sel=Some(Selection::all(Side::Right,v.raw(Side::Right))));
    f.window.update(cx,|v,w,cx|{let(start,end)=v.pane_span(w,Side::Right);v.on_right_down(&MouseDownEvent{position:point(px((start+end)/2.),px(v.top()+1.)),button:MouseButton::Right,..Default::default()},w,cx);}).unwrap();
    key(cx,&f,"home",false,false,false);key(cx,&f,"enter",false,false,false);assert_eq!(cx.update(|cx|cx.read_from_clipboard().unwrap().text().unwrap()),"α new\n");assert_eq!(text(&f),"α new\n");
}

#[gpui::test]
fn gui_file_menu_undo_targets_the_clicked_file_and_keeps_the_other_file(cx:&mut TestAppContext){
    let f=files_fixture(cx);f.viewer.update(cx,|v,cx|{v.move_change(0,Side::Right,cx);});cx.executor().run_until_parked();
    key(cx,&f,"s",true,false,false);
    let path=f.viewer.read_with(cx,|v,_|match &v.set.comparisons[0].right{Source::File(p)=>p.clone(),_=>panic!()});let changed=std::fs::read_to_string(&path).unwrap();
    f.viewer.update(cx,|v,cx|v.open(1,Place::Top,cx));cx.executor().run_until_parked();
    let y=f.viewer.read_with(cx,|v,_|v.top()+v.nav.iter().position(|r|matches!(r,NavRow::File{index:0,..})).unwrap()as f32*line_h()+2.);
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(10.),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    let row=f.viewer.read_with(cx,|v,_|v.menu.as_ref().unwrap().items.iter().position(|(_,a)|matches!(a,MenuAction::File(crate::file_actions::FileAction::UndoLenz,0,_))).unwrap());key(cx,&f,"home",false,false,false);for _ in 0..row{key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);cx.executor().run_until_parked();assert_eq!(std::fs::read_to_string(&path).unwrap(),changed);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.current),1);
    f.viewer.update(cx,|v,cx|v.open(0,Place::Top,cx));cx.executor().run_until_parked();key(cx,&f,"s",true,false,false);
    assert_ne!(std::fs::read_to_string(&path).unwrap(),changed);
}

#[gpui::test]
fn gui_horizontal_wheel_keeps_the_other_text_panes_hit_positions(cx: &mut TestAppContext) {
    let f=fixture(cx,&"left ".repeat(100),&"right ".repeat(100));
    f.viewer.update(cx,|v,_|{v.char_w=8.;v.layout.set_files(false);});
    let (lx,rx,y)=f.window.update(cx,|v,w,_|{let width=(f32::from(w.viewport_size().width)-v.diff_x()-GUTTER_W)/2.;(v.diff_x()+num_w()+16.,v.diff_x()+width+GUTTER_W+num_w()+16.,v.top()+line_h()/2.)}).unwrap();
    click(cx,&f,lx,y);let before=f.viewer.read_with(cx,|v,_|v.sel.unwrap());
    f.window.update(cx,|v,w,cx|v.on_wheel(&ScrollWheelEvent{position:point(px(rx),px(y)),delta:ScrollDelta::Lines(point(-1.,0.)),..Default::default()},w,cx)).unwrap();
    click(cx,&f,lx,y);assert_eq!(f.viewer.read_with(cx,|v,_|v.sel.unwrap()),before,"right wheel must preserve left hit testing");
    click(cx,&f,rx,y);assert!(f.viewer.read_with(cx,|v,_|v.sel.unwrap().head.col)>before.head.col);
    let caret=f.viewer.read_with(cx,|v,_|v.sel.unwrap());key(cx,&f,"right",true,false,true);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.sel),Some(caret),"horizontal keyboard scrolling must not move the caret");
    click(cx,&f,rx,y);assert!(f.viewer.read_with(cx,|v,_|v.sel.unwrap().head.col)>caret.head.col);
}

#[gpui::test]
fn gui_horizontal_merge_wheel_keyboard_and_modal_use_separate_panes(cx:&mut TestAppContext){
    let(dir,viewer)=merge_view::gui_tests::fixture(cx);
    for name in ["base","ours","theirs","result"]{std::fs::write(dir.path().join(name),format!("{name}{}\n","界\tword ".repeat(100))).unwrap();}
    let opened=merge_view::open_files(&dir.path().join("base"),&dir.path().join("ours"),&dir.path().join("theirs"),&dir.path().join("result"),dir.path()).unwrap();
    viewer.update(cx,|v,cx|{v.start_merge(opened,false,cx);v.layout.set_files(false);});
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
    let bytes=std::fs::read(dir.path().join("result")).unwrap();
    window.update(cx,|v,w,cx|{
        let _=v.render(w,cx);let pane_w=v.merge.as_ref().unwrap().pane_w;
        let pos=point(px(v.diff_x()+pane_w[0]+pane_w[1]+2.0*GUTTER_W+num_w()+30.0),px(v.top()+5.0));
        let wheel=ScrollWheelEvent{position:pos,delta:ScrollDelta::Pixels(point(px(-40.),px(0.))),..Default::default()};
        v.on_wheel(&wheel,w,cx);assert_eq!(v.merge.as_ref().unwrap().horizontal,[0.,0.,40.]);
        v.on_key(&event("right",true,false,true),w,cx);let offsets=v.merge.as_ref().unwrap().horizontal;assert!(offsets[1]>0.);assert_eq!(offsets[0],0.);assert_eq!(offsets[2],40.);
        v.merge.as_mut().unwrap().ask=Some(merge_view::Ask::ChangedOnDisk);v.on_wheel(&wheel,w,cx);v.on_key(&event("right",true,false,true),w,cx);assert_eq!(v.merge.as_ref().unwrap().horizontal,offsets);
        v.on_key(&event("escape",false,false,false),w,cx);v.merge_frame(20.,[10000.;3]);assert_eq!(v.merge.as_ref().unwrap().horizontal,[0.;3]);
        let _=v.render(w,cx);
    }).unwrap();assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),bytes);
}

#[gpui::test]
fn gui_horizontal_shift_wheel_clamps_tabs_and_unicode_without_moving_the_caret(cx:&mut TestAppContext){
    let f=fixture(cx,"short\n",&format!("{}END\n","界\t".repeat(150)));f.viewer.update(cx,|v,_|v.layout.set_files(false));
    f.window.update(cx,|v,w,cx|{
        let _=v.render(w,cx);let pos=point(px(f32::from(w.viewport_size().width)-60.),px(v.top()+5.0));
        let anchor=v.comparison.view.anchor_line();
        let wheel=ScrollWheelEvent{position:pos,delta:ScrollDelta::Pixels(point(px(0.),px(-40.))),modifiers:gpui::Modifiers{shift:true,..Default::default()},..Default::default()};
        v.on_wheel(&wheel,w,cx);assert_eq!(v.hscroll,[0.,40.]);assert_eq!(v.comparison.view.anchor_line(),anchor);
        for _ in 0..1000{v.on_wheel(&wheel,w,cx);}let at_end=v.hscroll[1];assert!(at_end>40.);v.on_wheel(&wheel,w,cx);assert_eq!(v.hscroll[1],at_end);
        v.menu=Some(Menu{x:0.,y:0.,hover:None,items:vec![("Copy".into(),MenuAction::InteractionCommand(commands::Interaction::Copy))]});v.on_wheel(&wheel,w,cx);assert_eq!(v.hscroll[1],at_end);v.menu=None;
        v.text_w=[10000.;2];v.clamp_horizontal(Side::Right);assert_eq!(v.hscroll,[0.;2]);
    }).unwrap();assert_eq!(text(&f),format!("{}END\n","界\t".repeat(150)));
}

#[gpui::test]
fn gui_pr_details_blocks_underlying_horizontal_scroll_and_options(cx:&mut TestAppContext){
    let f=fixture(cx,&"old ".repeat(300),&"new ".repeat(300));
    f.viewer.update(cx,|v,_|{
        let mut pr:crate::pr::PullRequest=serde_json::from_str(include_str!("../../fixtures/pr/view.json")).unwrap();
        pr.body=(0..100).map(|i|format!("Description line {i}\n")).collect();
        v.repo.get_or_insert_with(||crate::repository_session::RepositorySession::with_picker(Picker::new(f.dir.path().into(),false))).pr=Some(crate::pr_session::PrState::new(pr,crate::pr::Fetched{number:14130,merge_base:"base".into(),head:"head".into()}));
    });
    key(cx,&f,"d",false,false,false);
    key(cx,&f,"right",true,false,true);
    key(cx,&f,"i",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.hscroll),[0.0;2]);
    assert!(!f.viewer.read_with(cx,|v,_|v.comparison.options().ignore_case));
    assert_eq!(text(&f),"new ".repeat(300));
}

#[gpui::test]
fn gui_pr_details_sections_full_content_wheel_and_close_preserve_the_diff(cx:&mut TestAppContext){
    use crate::pr_session::{DetailsTab,PrCommand};
    let f=fixture(cx,"old text\n","new text\n");
    f.viewer.update(cx,|v,_|{
        let mut pr:crate::pr::PullRequest=serde_json::from_str(include_str!("../../fixtures/pr/view.json")).unwrap();
        pr.body="Description 漢字 é\n\n".repeat(100)+"DESCRIPTION_END";
        let mut state=crate::pr_session::PrState::new(pr,crate::pr::Fetched{number:14130,merge_base:"base".into(),head:"head".into()});
        state.marks=None;
        state.seed_review(crate::pr::Review{files:vec![],discussion:vec![crate::pr::DiscussionComment{comment:crate::pr::Comment{author:None,body:"Discussion 漢字\n\n".repeat(100)+"DISCUSSION_END",created_at:"2026-10-05".into(),..Default::default()},review_state:Some("APPROVED".into()),..Default::default()}],placed:Default::default(),listed:Default::default(),..Default::default()},&v.set.comparisons).unwrap();
        v.repo.get_or_insert_with(||crate::repository_session::RepositorySession::with_picker(Picker::new(f.dir.path().into(),false))).pr=Some(state);
    });
    let page=|cx:&mut TestAppContext|f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().page.rows.iter().map(|r|r.text()).collect::<Vec<_>>().join("\n"));
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)),w,cx)).unwrap();
    key(cx,&f,"end",false,false,false);
    assert!(page(cx).contains("DESCRIPTION_END")&&page(cx).contains("DISCUSSION_END"),"the description and the discussion, whole");
    let end=f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().scroll);assert!(end>0);
    f.window.update(cx,|v,_,cx|v.pr_command(PrCommand::Show(DetailsTab::Conversation),cx)).unwrap();
    key(cx,&f,"up",false,false,false);assert_eq!(f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().scroll),end-1,"the same tab keeps its scroll");
    key(cx,&f,"tab",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().and_then(|r| r.pr.as_ref()).unwrap().details().unwrap().tab),DetailsTab::Commits);
    key(cx,&f,"tab",false,false,false);
    assert!(!f.viewer.read_with(cx,|v,_|v.pr_details_shown()),"then Files changed");
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)));
    f.viewer.update(cx,|v,_|{let pr=v.repo.as_mut().and_then(|r| r.pr.as_mut()).unwrap();let mut review=pr.review.clone().unwrap();review.discussion[0].comment.body="REFRESHED_COMMENT".into();pr.seed_review(review,&v.set.comparisons).unwrap();});
    key(cx,&f,"home",false,false,false);
    assert!(page(cx).contains("REFRESHED_COMMENT"));
    let (px_,py_)=f.viewer.read_with(cx,|v,_|{let (x,y,w,h)=v.pr_pane();(x+w/2.,y+h/2.)});
    f.window.update(cx,|v,w,cx|v.on_wheel(&ScrollWheelEvent{position:point(px(px_),px(py_)),delta:ScrollDelta::Pixels(point(px(0.0),px(-80.0))),..Default::default()},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().scroll)>0,"the wheel scrolls the tab");
    key(cx,&f,"escape",false,false,false);assert!(!f.viewer.read_with(cx,|v,_|v.pr_details_shown()));
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Commits)));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().and_then(|r| r.pr.as_ref()).unwrap().details().unwrap().tab),DetailsTab::Commits);
    assert_eq!(text(&f),"new text\n");
}

#[gpui::test]
fn gui_markdown_owner_dispatch_poll_refresh_and_invalidate(cx: &mut TestAppContext) {
    use crate::markdown::PresentationMode;
    let f = fixture_named(cx, "# old\n", "# new\n", "md");
    assert!(f.viewer.read_with(cx, |v, _| v.markdown_poll_task.is_none()));
    f.viewer.update(cx, |v, cx| v.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx));
    let settle = |cx: &mut TestAppContext| {
        let until = Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.executor().advance_clock(std::time::Duration::from_millis(20));
            cx.executor().run_until_parked();
            if f.viewer.read_with(cx, |v, _| !v.comparison.markdown_loading() && !v.comparison.refining && v.comparison.diffed == v.comparison.generation) { break; }
            assert!(Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    settle(cx);
    let first = f.viewer.read_with(cx, |v, _| v.comparison.markdown_projection().unwrap().clone());
    f.viewer.update(cx, |v, cx| v.refine_more(cx));
    settle(cx);
    assert!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_projection().unwrap().revision > first.revision));
    f.viewer.update(cx, |v, cx| v.comparison_command(Command::ToggleCase, cx));
    assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode()), PresentationMode::Rendered);
    settle(cx);
    f.viewer.read_with(cx, |v, _| {
        let next = v.comparison.markdown_projection().unwrap();
        assert!(next.revision > first.revision);
        assert!(Arc::ptr_eq(&next.pair, &first.pair));
    });
    f.viewer.update(cx, |v, cx| v.comparison_command(Command::SetPresentation(PresentationMode::Source), cx));
    assert!(f.viewer.read_with(cx, |v, _| !v.comparison.markdown_loading() && v.markdown_poll_task.is_none()));
    f.viewer.update(cx, |v, cx| v.reload(cx));
    assert!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_projection().is_none()));
    settle(cx);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode()), PresentationMode::Source);
}

#[gpui::test]
fn gui_markdown_empty_repository_refresh_releases_previous_presentation(cx: &mut TestAppContext) {
    use crate::markdown::PresentationMode;
    let f = fixture_named(cx, "# old\n", "# new\n", "md");
    f.viewer.update(cx, |v, cx| v.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx));
    markdown_ready(cx, &f);
    f.window.update(cx, |v, w, cx| { assert!(v.markdown_frame(w, cx).is_some()); }).unwrap();
    let pair = f.viewer.read_with(cx, |v, _| Arc::downgrade(&v.comparison.markdown_projection().unwrap().pair));
    let (x,y)=f.window.update(cx,|v,w,_|v.markdown_test_text(w,Side::Right,"new")).unwrap();click(cx,&f,x,y);
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.menu.is_some()&&v.comparison.markdown_selection().is_some()));
    f.viewer.update(cx, |v, cx| {
        let mut empty = (*v.set).clone();
        empty.comparisons.clear(); empty.groups.clear();
        v.accept_repository_refresh(empty, v.spec.clone(), cx);
        assert!(v.markdown.layout.is_none(), "refresh must release native resources immediately");assert!(v.menu.is_none()&&v.sel.is_none()&&v.comparison.markdown_selection().is_none());
    });
    assert!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_projection().is_none()));
    assert!(f.viewer.read_with(cx, |v, _| !v.comparison.markdown_loading() && v.markdown_poll_task.is_none()));
    assert!(pair.upgrade().is_none());
}

#[gpui::test]
fn gui_markdown_presentation_availability_matches_pending_dispatch(cx: &mut TestAppContext) {
    use crate::markdown::PresentationMode;
    let f = fixture_named(cx, "# old\n", "# new\n", "md");
    let rendered = Command::SetPresentation(PresentationMode::Rendered);
    let source = Command::SetPresentation(PresentationMode::Source);
    f.viewer.update(cx, |v, cx| v.comparison_command(rendered, cx));
    let until = Instant::now() + std::time::Duration::from_secs(5);
    while f.viewer.read_with(cx, |v, _| v.comparison.markdown_loading()) {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));
        cx.executor().run_until_parked();
        assert!(Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    f.viewer.update(cx, |v, cx| {
        let ready = v.comparison.markdown_projection().unwrap().clone();
        v.comparison_command(Command::ToggleCase, cx);
        assert_ne!(v.comparison.diffed, v.comparison.generation, "real diff result is still pending");
        let doc = v.doc().cloned().unwrap();
        assert_eq!(v.comparison.presentation_disabled(PresentationMode::Source, Some(&doc)), None);
        v.comparison_command(source, cx);
        assert_eq!(v.comparison.markdown_mode(), PresentationMode::Source);
        assert_eq!(v.comparison.presentation_disabled(PresentationMode::Rendered, Some(&doc)), None);
        v.comparison_command(rendered, cx);
        assert_eq!(v.comparison.markdown_notice(), None);
        assert_eq!(v.comparison.markdown_mode(), PresentationMode::Rendered);
        assert!(Arc::ptr_eq(&ready, v.comparison.markdown_projection().unwrap()));
        let mut empty = (*v.set).clone();
        empty.comparisons.clear(); empty.groups.clear();
        v.accept_repository_refresh(empty, v.spec.clone(), cx);
        assert!(v.doc().is_none());
        let unavailable = v.comparison.presentation_disabled(PresentationMode::Rendered, None).unwrap();
        v.comparison_command(rendered, cx);
        assert_eq!(v.comparison.markdown_notice(), Some(unavailable));
        assert!(!v.comparison.markdown_loading());
        assert_eq!(v.comparison.presentation_disabled(PresentationMode::Source, None), None);
        v.comparison_command(source, cx);
        assert_eq!(v.comparison.markdown_mode(), PresentationMode::Source);
    });
    let plain = fixture(cx, "plain old", "plain new");
    plain.viewer.update(cx, |v, cx| {
        let doc = v.doc().cloned().unwrap();
        let unavailable = v.comparison.presentation_disabled(PresentationMode::Rendered, Some(&doc)).unwrap();
        v.comparison_command(rendered, cx);
        assert_eq!(v.comparison.markdown_notice(), Some(unavailable));
        assert!(!v.comparison.markdown_loading());
        assert_eq!(v.comparison.markdown_mode(), PresentationMode::Source);
    });
}

#[gpui::test]
fn gui_markdown_pending_and_ready_prevent_source_writes(cx: &mut TestAppContext) {
    use crate::markdown::{MarkdownCommand, PresentationMode};
    let f = fixture_named(cx, "# old\n", "# new\n", "md");
    f.viewer.update(cx, |v,cx| v.move_change(0, Side::Right, cx));
    cx.executor().run_until_parked();
    key(cx,&f,"s",true,false,false);
    f.viewer.update(cx, |v,cx| {
        assert_eq!(v.edits.applied(0).len(),1);
        let doc=v.doc().cloned().unwrap();
        v.comparison.markdown_command_document(MarkdownCommand::SetMode(PresentationMode::Rendered),Some(&doc));
        assert!(v.comparison.markdown_loading());
        v.undo(false,cx);
        assert_eq!(v.write_error.as_deref(),Some("Switch to Source to edit"));
        assert!(v.edits.applied(0)[0].active);
    });
    assert_eq!(std::fs::read_to_string(f.dir.path().join("right.md")).unwrap(),"# old\n");
    let until=Instant::now()+std::time::Duration::from_secs(5);
    while f.viewer.read_with(cx,|v,_|v.comparison.markdown_loading()) {
        f.viewer.update(cx,|v,_|{v.comparison.markdown_poll();});
        assert!(Instant::now()<until);std::thread::yield_now();
    }
    for (name,ctrl,shift,alt) in [("z",true,false,false),("y",true,false,false),("right",false,true,true)] {
        key(cx,&f,name,ctrl,shift,alt);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.write_error.clone()).as_deref(),Some("Switch to Source to edit"));
        assert_eq!(std::fs::read_to_string(f.dir.path().join("right.md")).unwrap(),"# old\n");
    }
    f.viewer.update(cx, |v,cx| {
        let doc=v.doc().cloned().unwrap();
        v.comparison.markdown_command_document(MarkdownCommand::SetMode(PresentationMode::Source),Some(&doc));
        v.undo(false,cx);
    });
    cx.executor().run_until_parked();key(cx,&f,"s",true,false,false);
    assert_eq!(std::fs::read_to_string(f.dir.path().join("right.md")).unwrap(),"# new\n");
}

pub(super) fn markdown_ready(cx: &mut TestAppContext, f: &Fixture) {
    markdown_projected(cx, f);
    // then the rendered layout, which the first paint starts shaping in the background
    let until = Instant::now() + std::time::Duration::from_secs(5);
    while f.window.update(cx, |v, w, cx| { let _ = v.markdown_frame(w, cx); v.markdown_building() }).unwrap() {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));
        cx.executor().run_until_parked();
        assert!(Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// The projection only: the rendered layout may still be shaping.
pub(super) fn markdown_projected(cx: &mut TestAppContext, f: &Fixture) {
    let until = Instant::now() + std::time::Duration::from_secs(5);
    while f.viewer.read_with(cx, |v, _| v.comparison.markdown_loading()) {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));
        cx.executor().run_until_parked();
        assert!(Instant::now() < until);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[gpui::test]
fn gui_markdown_menu_renders_and_blocks_source_writes(cx: &mut TestAppContext) {
    let f = fixture_named(cx, "# Old\n", "# New\n", "md");
    run(cx,&f,crate::catalog::Action::Comparison(commands::Command::Preview(crate::markdown::MarkdownLayout::Inline)));
    markdown_ready(cx, &f);
    let before = std::fs::read(f.dir.path().join("right.md")).unwrap();
    key(cx, &f, "left", false, true, true);
    key(cx, &f, "z", true, false, false);
    assert_eq!(std::fs::read(f.dir.path().join("right.md")).unwrap(), before);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode()), crate::markdown::PresentationMode::Rendered);
    let source_selection=f.viewer.read_with(cx,|v,_|v.sel);
    key(cx,&f,"a",true,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.sel),source_selection,"rendered Select All must preserve Source caret");
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_selection().is_some()));
}

/// Snapshot both preview layers and the outer version workflow across modal input.
#[derive(Debug, PartialEq)]
pub(super) struct PickerPreviewState {
    mode: crate::markdown::PresentationMode,
    source_selection: Option<Selection>,
    rendered_selection: Option<crate::markdown::RenderSelection>,
    anchors: [Option<crate::markdown::SourceAnchor>; 2],
    chosen: Option<usize>,
    dirty: bool,
    ask: Option<crate::merge_session::Ask>,
    bytes: [Vec<u8>; 3],
}
pub(super) fn picker_preview_state(cx: &TestAppContext, f: &Fixture) -> PickerPreviewState {
    f.viewer.read_with(cx, |v, _| {
        let picker = v.picker_v.as_ref().unwrap();
        PickerPreviewState {
            mode: v.comparison.markdown_mode(),
            source_selection: v.sel,
            rendered_selection: v.comparison.markdown_selection(),
            anchors: [
                v.comparison.markdown_anchor(Side::Left),
                v.comparison.markdown_anchor(Side::Right),
            ],
            chosen: picker.chosen,
            dirty: picker.dirty,
            ask: picker.ask,
            bytes: [
                f.dir.path().join("left.md"),
                f.dir.path().join("right.md"),
                picker.o.target.clone(),
            ].map(|p| std::fs::read(p).unwrap()),
        }
    })
}

fn workflow_source_hash(root:&Path)->u64 {
    use std::hash::{Hash,Hasher};let mut hash=std::collections::hash_map::DefaultHasher::new();
    for name in ["gone.md","new.md","ref.md"]{std::fs::read(root.join(name)).ok().hash(&mut hash);}hash.finish()
}
fn workflow_git(root:&Path,args:&[&str]) {
    assert!(std::process::Command::new("git").args(args).current_dir(root)
        .env("GIT_CONFIG_GLOBAL","/dev/null").env("GIT_CONFIG_NOSYSTEM","1")
        .env("GIT_AUTHOR_NAME","Test").env("GIT_AUTHOR_EMAIL","test@example.invalid")
        .env("GIT_COMMITTER_NAME","Test").env("GIT_COMMITTER_EMAIL","test@example.invalid")
        .output().unwrap().status.success(),"git {args:?}");
}
pub(super) fn workflow_wait(cx:&mut TestAppContext,f:&Fixture,done:impl Fn(&Viewer)->bool) {
    let until=Instant::now()+std::time::Duration::from_secs(10);
    while !f.viewer.read_with(cx,|v,_|done(v)) {
        cx.executor().advance_clock(std::time::Duration::from_millis(20));cx.executor().run_until_parked();
        assert!(Instant::now()<until,"workflow did not settle: {:?}", f.viewer.read_with(cx,|v,_|(v.set.comparisons.len(),v.current,v.comparison.generation,v.comparison.diffed,v.doc().is_some(),v.repo.as_ref().map(|r| &r.picker).map(|p|(p.rows(),p.loading,p.anchor,p.focus)),v.toast.as_ref().map(|t|t.0.clone()),v.repo.as_ref().and_then(|r| r.pr.as_ref()).map(|s|(s.review_loading(),s.review.is_some())),v.layout.files(),v.nav.len(),v.top(),v.picker_w())));std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
#[gpui::test]
fn gui_reopening_a_repository_returns_to_the_commit_file_and_line(cx:&mut TestAppContext) {
    cx.skip_drawing();let dir=tempfile::tempdir().unwrap();let root=dir.path();workflow_git(root,&["init","-q","-b","main"]);
    std::fs::write(root.join("a.txt"),"a\n").unwrap();
    std::fs::write(root.join("b.txt"),(1..=300).map(|i|format!("line {i}\n")).collect::<String>()).unwrap();
    workflow_git(root,&["add","-A"]);workflow_git(root,&["commit","-qm","one"]);
    let first=String::from_utf8_lossy(&crate::git::run(root,&["rev-parse","HEAD"]).unwrap()).trim().to_string();
    std::fs::write(root.join("b.txt"),(1..=300).map(|i|format!("LINE {i}\n")).collect::<String>()).unwrap();
    workflow_git(root,&["commit","-qam","two"]);
    let state=tempfile::tempdir().unwrap();let toplevel=crate::changeset::toplevel(root).unwrap();let file=crate::place::path_in(state.path(),&toplevel);
    // Saved places carry git's toplevel, which on macOS resolves a `/var` temp directory to `/private/var`.
    crate::place::save_to(&file,&crate::place::Place{version:crate::place::VERSION,root:toplevel,source:crate::place::Source::Log{commit:first.clone()},file:Some("b.txt".into()),line:Some(200)}).unwrap();
    let mut request=crate::cli::parse_with(&[],root,&Default::default()).unwrap();
    let launch=crate::place::for_launch_in(&mut request,root,true,state.path());
    assert!(matches!(request.launch,crate::cli::Launch::Log{..}),"a saved commit opens the log");
    let set=Arc::new(ChangeSet::load(&Spec::Empty,root).unwrap());let doc=Arc::new(Doc::new("","",Path::new(""),Path::new("")));let view=doc.view(&[],30.);
    let viewer=cx.new(|cx|Viewer::new(set,Spec::Empty,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx));
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();let f=Fixture{dir,viewer,window};
    f.viewer.update(cx,|v,cx|{let session=crate::repository_session::RepositorySession::open_loaded(f.dir.path().into(),true,None,None,v.set.clone(),v.spec.clone());v.with_session(session,cx);v.restore_place(launch,cx);});
    // drawing is skipped in tests; render as the app does after each repository update
    let until=Instant::now()+std::time::Duration::from_secs(10);
    while !f.viewer.read_with(cx,|v,_|v.pending_place.is_none()&&v.doc().is_some()&&v.set.comparisons.get(v.comparison.current).is_some_and(|c|c.path=="b.txt")) {
        f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
        cx.executor().advance_clock(std::time::Duration::from_millis(20));cx.executor().run_until_parked();
        assert!(Instant::now()<until,"restored file never opened");std::thread::sleep(std::time::Duration::from_millis(1));
    }
    workflow_wait(cx,&f,|v|v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    let (line,source,recorded)=f.viewer.read_with(cx,|v,_|(v.comparison.view.anchor_line_on(Side::Right) as i64+1,v.repo.as_ref().and_then(|r|r.place_source()),v.place_recorder.as_ref().and_then(|r|r.last().cloned())));
    assert!((line-200).abs()<=1,"line {line}");
    assert_eq!(source,Some(crate::place::Source::Log{commit:first.clone()}));
    let recorded=recorded.expect("the place is recorded");
    assert_eq!((recorded.file.as_deref(),recorded.source),(Some("b.txt"),crate::place::Source::Log{commit:first}));
}
fn repository_markdown_fixture(cx:&mut TestAppContext)->Fixture {
    cx.skip_drawing();let dir=tempfile::tempdir().unwrap();let root=dir.path();workflow_git(root,&["init","-q","-b","main"]);
    std::fs::write(root.join("gone.md"),"# Deleted\n").unwrap();std::fs::write(root.join("ref.md"),"[guide][id]\n\n[id]: old\n").unwrap();
    workflow_git(root,&["add","."]);workflow_git(root,&["commit","-qm","Before Markdown"]);
    std::fs::remove_file(root.join("gone.md")).unwrap();std::fs::write(root.join("new.md"),"# Added\n").unwrap();std::fs::write(root.join("ref.md"),"[guide][id]\n\n[id]: new\n").unwrap();
    workflow_git(root,&["add","-A"]);workflow_git(root,&["commit","-qm","After Markdown"]);
    let set=Arc::new(ChangeSet::load(&Spec::Empty,root).unwrap());let doc=Arc::new(Doc::new("","",Path::new(""),Path::new("")));let view=doc.view(&[],30.);
    let viewer=cx.new(|cx|Viewer::new(set,Spec::Empty,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx));
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();let f=Fixture{dir,viewer,window};
    f.viewer.update(cx,|v,cx|{let session=crate::repository_session::RepositorySession::open_loaded(f.dir.path().into(),true,None,None,v.set.clone(),v.spec.clone());v.with_session(session,cx)});
    workflow_wait(cx,&f,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|!p.loading&&p.rows()>=3));
    f.viewer.update(cx,|v,_|v.repo.as_mut().map(|r| &mut r.picker).unwrap().focus=Pane::Log);
    key(cx,&f,"home",false,false,false);key(cx,&f,"down",false,false,false);
    workflow_wait(cx,&f,|v|v.set.comparisons.len()==3&&v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){key(cx,&f,"1",false,false,true);}
    f
}
#[gpui::test]
fn gui_markdown_repository_added_deleted_commits_keep_authoritative_hunks(cx:&mut TestAppContext) {
    use crate::markdown::PresentationMode;
    let f=repository_markdown_fixture(cx);let before=std::fs::read(f.dir.path().join("ref.md")).unwrap();let source_hash_before=workflow_source_hash(f.dir.path());
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),PresentationMode::Source);
    for name in ["gone.md","new.md","ref.md"] {
        let y=f.viewer.read_with(cx,|v,_|v.top()+v.nav.iter().position(|r|matches!(r,NavRow::File{index,..}if v.set.comparisons[*index].path==name)).unwrap()as f32*line_h()+2.);
        click(cx,&f,f.viewer.read_with(cx,|v,_|v.picker_w()+80.),y);
        workflow_wait(cx,&f,|v|v.set.comparisons.get(v.current).is_some_and(|c|c.path==name)&&v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));markdown_ready(cx,&f);
        f.window.update(cx,|v,w,cx|{assert!(v.markdown_frame(w,cx).is_some());let projection=v.comparison.markdown_projection().unwrap();for id in 0..v.comparison.view.change_count(){assert!(projection.represents_hunk(id));}if name=="gone.md"{assert_eq!(v.labels(v.current).right,"(deleted)");assert!(projection.pair.sides[1].source.is_empty());}
            if name=="new.md"{assert_eq!(v.labels(v.current).left,"(absent)");assert!(projection.pair.sides[0].source.is_empty());}}).unwrap();
        let count=f.viewer.read_with(cx,|v,_|v.comparison.view.change_count());for id in 0..count{block(cx,&f,id);}
        if name=="ref.md"{let (x,y,a)=f.window.update(cx,|v,w,_|v.markdown_test_marker(w,0,Side::Left)).unwrap();click(cx,&f,x,y);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_anchor(a.side)),Some(a));run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);}
    }
    f.viewer.update(cx,|v,_|v.repo.as_mut().map(|r| &mut r.picker).unwrap().focus=Pane::Log);key(cx,&f,"down",false,false,false);
    workflow_wait(cx,&f,|v|v.set.comparisons.len()==2&&v.comparison.diffed==v.comparison.generation);markdown_ready(cx,&f);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),PresentationMode::Rendered);
    let source_hash_after=workflow_source_hash(f.dir.path());assert_eq!(source_hash_before,source_hash_after);
    assert_eq!(std::fs::read(f.dir.path().join("ref.md")).unwrap(),before);
}

#[gpui::test]
fn gui_source_load_more_branches_click_reads_the_next_page(cx: &mut TestAppContext) {
    let f = repository_markdown_fixture(cx);
    for i in 0..44 { workflow_git(f.dir.path(), &["branch", &format!("topic-{i:02}")]); }
    f.viewer.update(cx, |v, cx| {
        let session = crate::repository_session::RepositorySession::open_loaded(f.dir.path().into(), true, None, None, v.set.clone(), v.spec.clone());
        v.with_session(session, cx);
    });
    workflow_wait(cx, &f, |v| v.repo.as_ref().unwrap().picker.sources.loaded);
    let row = f.viewer.read_with(cx, |v, _| {
        let p = &v.repo.as_ref().unwrap().picker;
        assert_eq!(p.sources.branches.len(), 20);
        p.sources.rows(&v.palette, &p.branches).iter().position(|r| matches!(r.action, Some(crate::repository_session::SourceAction::ShowAllBranches))).unwrap()
    });
    let y = f.viewer.read_with(cx, |v, _| toolbar_h() + (row as f32 - v.repo.as_ref().unwrap().picker.sources.scroll + 1.5) * line_h());
    click(cx, &f, 30., y);
    workflow_wait(cx, &f, |v| v.repo.as_ref().unwrap().picker.sources.branches.len() == 40);
}

#[gpui::test]
fn gui_pr_click_keeps_a_loader_after_the_toast_and_clears_on_completion(cx: &mut TestAppContext) {
    let f = repository_markdown_fixture(cx);
    let _provider = crate::pr::test_support::markdown_provider(f.dir.path());
    let before = workflow_source_hash(f.dir.path());
    f.viewer.update(cx, |v, cx| {
        let session = crate::repository_session::RepositorySession::open_loaded(f.dir.path().into(), true, None, None, v.set.clone(), v.spec.clone());
        v.with_session(session, cx);
    });
    workflow_wait(cx, &f, |v| v.repo.as_ref().unwrap().picker.sources.prs.iter().any(|p| p.number == 9));
    let row = f.viewer.read_with(cx, |v, _| {
        let p = &v.repo.as_ref().unwrap().picker;
        p.sources.rows(&v.palette, &p.branches).iter().position(|r| matches!(r.action, Some(crate::repository_session::SourceAction::Pr(9)))).unwrap()
    });
    let old = f.viewer.read_with(cx, |v, _| v.doc().unwrap().text_right.clone());
    let y = f.viewer.read_with(cx, |v, _| toolbar_h() + (row as f32 - v.repo.as_ref().unwrap().picker.sources.scroll + 1.5) * line_h());
    click(cx, &f, 30., y);
    f.viewer.update(cx, |v, _| v.toast = None);
    for (width, height, theme) in [(1200., 800., "darcula"), (320., 160., "light")] {
        cx.simulate_window_resize(f.window.into(), size(px(width), px(height)));
        cx.update(|cx| cx.update_global::<themes::GuiThemes, _>(|g, _| g.0.execute(theme::ThemeCommand::Select(theme::ThemeSelection::Named(theme.into())))));
        f.window.update(cx, |v, w, cx| {
            let _ = v.render(w, cx);
            let banner = v.banner().expect("a clicked PR needs persistent visible feedback");
            assert!(banner.contains("Loading pull request #9"), "{banner}");
            assert_eq!(v.doc().unwrap().text_right, old, "keep the previous comparison while fetching");
        }).unwrap();
    }
    click(cx, &f, 30., toolbar_h() + 1.5 * line_h());
    assert!(f.viewer.read_with(cx, |v, _| v.banner().is_none_or(|b| !b.contains("Loading pull request"))), "another source cancels PR feedback immediately");
    workflow_wait(cx, &f, |v| !v.repo.as_ref().unwrap().busy());
    assert!(f.viewer.read_with(cx, |v, _| v.repo.as_ref().unwrap().pr.is_none()), "a stale PR completion must stay cancelled");
    cx.simulate_window_resize(f.window.into(), size(px(1200.), px(800.)));
    f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
    click(cx, &f, 30., y);
    workflow_wait(cx, &f, |v| v.repo.as_ref().unwrap().pr.as_ref().is_some_and(|p| p.pr.number == 9) && v.doc().is_some() && v.comparison.generation == v.comparison.diffed);
    f.viewer.read_with(cx, |v, _| assert!(v.banner().is_none_or(|b| !b.contains("Loading pull request"))));
    assert!(f.viewer.read_with(cx, |v, _| v.layout.focused(v.repo.as_ref())));
    run(cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Refresh));
    assert!(f.viewer.read_with(cx, |v, _| v.banner().is_some_and(|b| b.contains("Loading pull request #9"))), "refreshing in Focus also shows the loader");
    workflow_wait(cx, &f, |v| !v.repo.as_ref().unwrap().busy());
    assert!(f.viewer.read_with(cx, |v, _| v.banner().is_none_or(|b| !b.contains("Loading pull request"))));
    assert_eq!(workflow_source_hash(f.dir.path()), before);
}

#[gpui::test]
fn gui_markdown_offline_pr_sidebar_opens_real_comparison_and_modal(cx:&mut TestAppContext) {
    let f=repository_markdown_fixture(cx);let provider=crate::pr::test_support::markdown_provider(f.dir.path());
    let before=std::fs::read(f.dir.path().join("ref.md")).unwrap();let source_hash_before=workflow_source_hash(f.dir.path());
    f.viewer.update(cx,|v,cx|{let session=crate::repository_session::RepositorySession::open_loaded(f.dir.path().into(),true,None,None,v.set.clone(),v.spec.clone());v.with_session(session,cx)});
    workflow_wait(cx,&f,|v|v.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.sources.prs.iter().any(|p|p.number==9)));
    let at=f.viewer.read_with(cx,|v,_|{let p=v.repo.as_ref().map(|r| &r.picker).unwrap();p.sources.rows(&v.palette,&p.branches).iter().position(|r|matches!(r.action,Some(crate::repository_session::SourceAction::Pr(9)))).unwrap()});
    f.viewer.update(cx,|v,_|v.repo.as_mut().map(|r| &mut r.picker).unwrap().focus=Pane::Sidebar);key(cx,&f,"home",false,false,false);for _ in 0..at{key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);
    workflow_wait(cx,&f,|v|v.repo.as_ref().and_then(|r| r.pr.as_ref()).is_some_and(|s|s.pr.number==9&&!s.review_loading())&&v.doc().is_some()&&v.comparison.generation==v.comparison.diffed);
    f.viewer.update(cx,|v,_|{let s=v.repo.as_mut().and_then(|r| r.pr.as_mut()).unwrap();s.execute(crate::pr_session::PrCommand::Show(crate::pr_session::DetailsTab::Conversation));assert!(s.review.is_some());s.execute(crate::pr_session::PrCommand::Close);});
    let y=f.viewer.read_with(cx,|v,_|v.top()+v.nav.iter().position(|r|matches!(r,NavRow::File{index,..}if v.set.comparisons[*index].path=="ref.md")).unwrap()as f32*line_h()+2.);
    click(cx,&f,f.viewer.read_with(cx,|v,_|v.picker_w()+80.),y);
    workflow_wait(cx,&f,|v|v.set.comparisons[v.current].path=="ref.md"&&v.comparison.generation==v.comparison.diffed);
    run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));markdown_ready(cx,&f);
    f.viewer.read_with(cx,|v,_|{let projection=v.comparison.markdown_projection().unwrap();for id in 0..v.comparison.view.change_count(){assert!(projection.represents_hunk(id));}});
    let (x,y)=f.window.update(cx,|v,w,_|v.markdown_test_text(w,Side::Right,"guide")).unwrap();click(cx,&f,x,y);
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    let at=f.viewer.read_with(cx,|v,_|v.menu.as_ref().unwrap().items.iter().position(|(_,a)|matches!(a,MenuAction::PrLine(Side::Right))).unwrap());key(cx,&f,"home",false,false,false);for _ in 0..at{key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().and_then(|r| r.pr.as_ref()).unwrap().page(120,0).unwrap().rows.iter().any(|r|r.text().contains("Source line thread"))));key(cx,&f,"escape",false,false,false);
    block(cx,&f,0);let (x,y,a)=f.window.update(cx,|v,w,_|v.markdown_test_marker(w,0,Side::Left)).unwrap();click(cx,&f,x,y);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_anchor(a.side)),Some(a));
    run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(crate::pr_session::DetailsTab::Conversation)));let mode=f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode());key(cx,&f,":",false,false,false);key(cx,&f,"z",true,false,false);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),mode);key(cx,&f,"escape",false,false,false);
    let source_hash_after=workflow_source_hash(f.dir.path());assert_eq!(source_hash_before,source_hash_after);
    assert_eq!(std::fs::read(f.dir.path().join("ref.md")).unwrap(),before);
    let calls=std::fs::read_to_string(f.dir.path().join(".git/markdown-provider/requests.log")).unwrap();assert!(calls.contains("pr view 9")&&calls.contains("api graphql"));drop(provider);
}

#[gpui::test]
fn gui_markdown_real_file_switch_reload_and_independent_close_release_resources(cx:&mut TestAppContext) {
    use crate::markdown::PresentationMode;
    cx.skip_drawing();let dir=tempfile::tempdir().unwrap();let left=dir.path().join("left");let right=dir.path().join("right");std::fs::create_dir(&left).unwrap();std::fs::create_dir(&right).unwrap();
    for name in ["a.md","b.txt","c.md"] {std::fs::write(left.join(name),"# Old\n").unwrap();std::fs::write(right.join(name),"# New\n\n**words**\n".repeat(100)).unwrap();}
    let spec=Spec::Plain{left:left.clone(),right:right.clone()};let set=Arc::new(ChangeSet::load(&spec,dir.path()).unwrap());let doc=Arc::new(Doc::load(&set,0,&left.join("a.md"),&right.join("a.md"),false).unwrap());let view=doc.view(&doc.line_diff(&Options::default()),30.);
    let viewer=cx.new(|cx|Viewer::new(set,spec,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx));let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();let f=Fixture{dir,viewer,window};
    run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);
    f.window.update(cx,|v,w,cx|{assert!(v.markdown_frame(w,cx).is_some());}).unwrap();
    let pair=f.viewer.read_with(cx,|v,_|Arc::downgrade(&v.comparison.markdown_projection().unwrap().pair));
    key(cx,&f,"]",false,false,false);assert!(f.viewer.read_with(cx,|v,_|v.markdown.layout.is_none()&&v.comparison.markdown_selection().is_none()));
    key(cx,&f,"]",false,false,false);workflow_wait(cx,&f,|v|v.current==2&&v.doc().is_some()&&v.comparison.generation==v.comparison.diffed&&!v.comparison.markdown_loading());
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),PresentationMode::Rendered);assert!(pair.upgrade().is_none());
    std::fs::write(right.join("c.md"),"# Tiny\n").unwrap();run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));workflow_wait(cx,&f,|v|v.doc().is_some_and(|d|d.text_right.len()==7)&&v.comparison.generation==v.comparison.diffed&&!v.comparison.markdown_loading());
    markdown_ready(cx,&f);
    f.window.update(cx,|v,w,cx|{assert!(v.markdown_frame(w,cx).is_some());assert_eq!(v.comparison.markdown_projection().unwrap().pair.sides[1].source.as_ref(),"# Tiny\n");}).unwrap();
    let other=fixture_named(cx,"# Other\n","# Independent\n","md");assert_eq!(other.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),PresentationMode::Source);
    run(cx,&other,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
    other.window.update(cx,|_,w,_|w.remove_window()).unwrap();drop(other);cx.executor().run_until_parked();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_projection().unwrap().pair.sides[1].source.as_ref().to_owned()),"# Tiny\n");
    assert_eq!(std::fs::read(right.join("c.md")).unwrap(),b"# Tiny\n");
}

#[gpui::test]
fn gui_markdown_actual_empty_constructor_lists_all_presentation_commands(cx:&mut TestAppContext) {
    let f=fixture_named(cx,"# Before\n","# After\n","md");let set=Arc::new(ChangeSet::load(&Spec::Empty,f.dir.path()).unwrap());
    f.viewer.update(cx,|v,cx|{let doc=Arc::new(Doc::new("","",Path::new(""),Path::new("")));let view=doc.view(&[],20.);*v=Viewer::new(set,Spec::Empty,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);});
    key(cx,&f,":",false,false,false);
    f.viewer.read_with(cx,|v,_|{let p=v.command_palette.as_ref().unwrap();for &command in commands::ALL {let cycle=matches!(command,Command::CycleScroll|Command::CycleWhitespace|Command::CycleHighlight);assert!(cycle||p.rows().any(|r|r.entry.action==crate::catalog::Action::Comparison(command)),"{command:?}");}assert_eq!(v.comparison.presentation_disabled(crate::markdown::PresentationMode::Rendered,None),Some("Comparison loading"));});
    key(cx,&f,"escape",false,false,false);key(cx,&f,"z",true,false,false);run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);assert!(v.markdown_frame(w,cx).is_none());assert!(v.comparison.markdown_projection().is_none());assert!(!v.comparison.markdown_loading());}).unwrap();
}


#[gpui::test]
fn gui_pane_drag_preserves_selection_and_session_sizes_and_resets(cx: &mut TestAppContext) {
    let f=files_fixture(cx);
    f.window.update(cx,|v,w,cx| {
        v.layout.set_files(true);
        let _=v.render(w,cx);
        let width=f32::from(w.viewport_size().width)-v.diff_x();
        let edge=v.diff_x()+v.comparison_widths(width)[0]+GUTTER_W/2.0;
        let y=v.top()-hdr_h()/2.0;
        v.sel=Some(Selection::caret(Side::Right,Pos::new(0,0)));
        let selection=v.sel;
        v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(edge),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx);
        assert!(v.panes.drag.is_some());
        v.on_mouse_move(&MouseMoveEvent{position:point(px(edge+80.),px(y)),pressed_button:Some(MouseButton::Left),modifiers:Default::default()},w,cx);
        assert!(v.panes.comparison>0.5);
        assert_eq!(v.sel,selection);
        let ratio=v.panes.comparison;
        v.on_mouse_up(&MouseUpEvent{button:MouseButton::Left,position:point(px(edge+80.),px(y)),modifiers:Default::default(),click_count:1},w,cx);
        assert!(v.panes.drag.is_none());
        v.open(1,Place::Top,cx);
        assert_eq!(v.panes.comparison,ratio);
        v.panes.viewport[0]=60.;
        let widths=v.comparison_widths(20.);
        assert!(widths.iter().all(|w|w.is_finite()&&*w>=0.0));
        assert_eq!(v.panes.comparison,ratio);
        v.interaction(commands::Interaction::ResetPaneSizes,cx);
        assert_eq!(v.panes.comparison,0.5);
        assert_eq!(v.panes.navigator,SIDEBAR_W);
        assert!(v.panes.drag.is_none());
    }).unwrap();
}

#[gpui::test]
fn gui_pane_handles_leave_modals_and_gutter_actions_in_control(cx: &mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    f.window.update(cx,|v,w,cx| {
        v.layout.set_files(false);let _=v.render(w,cx);
        let edge=v.comparison_widths(f32::from(w.viewport_size().width))[0]+GUTTER_W/2.0;
        let y=v.top()-hdr_h()/2.0;
        v.help=true;assert!(!v.begin_resize(edge,y));v.help=false;
        v.goto=Some(String::new());assert!(!v.begin_resize(edge,y));v.goto=None;
        v.menu=Some(Menu{x:0.,y:0.,items:Vec::new(),hover:None});assert!(!v.begin_resize(edge,y));v.menu=None;
        assert!(v.begin_resize(edge,y));v.reset_pane_sizes();
        assert!(!v.dragging);
    }).unwrap();
}

#[gpui::test]
#[ignore = "synthetic pane state/reflow timing probe using GPUI test text system; run explicitly after implementation"]
fn gui_pane_drag_reflow_responsiveness(cx: &mut TestAppContext) {
    let markdown=(0..60).map(|i|format!("Paragraph {i}: {}\n\n", "wrapped text ".repeat(12))).collect::<String>();
    let f=fixture_named(cx,&markdown,&markdown.replace("text","words"),"md");
    f.viewer.update(cx,|v,cx|v.comparison_command(Command::SetPresentation(crate::markdown::PresentationMode::Rendered),cx));
    markdown_ready(cx,&f);
    f.window.update(cx,|v,w,cx| {
        v.layout.set_files(false);let _=v.render(w,cx);
        let edge=v.comparison_widths(f32::from(w.viewport_size().width))[0]+GUTTER_W/2.0;
        assert!(v.begin_resize(edge,v.top()-hdr_h()/2.0));
        let start=Instant::now();
        for motion in 0..40 {
            v.resize_move(edge+(motion%10) as f32*8.0,v.top()-hdr_h()/2.0,true,cx);
            let _=v.render(w,cx);
        }
        eprintln!("synthetic pane state/reflow (GPUI test text system): 40 frames in {:?}",start.elapsed());
        assert!(v.panes.comparison>0.5);
    }).unwrap();
}


#[gpui::test]
fn gui_find_modals_cancel_existing_pane_drag(cx: &mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    f.window.update(cx,|v,w,cx| {
        v.layout.set_files(false);let _=v.render(w,cx);
        let edge=v.comparison_widths(f32::from(w.viewport_size().width))[0]+GUTTER_W/2.0;
        let y=v.top()-hdr_h()/2.0;
        assert!(v.begin_resize(edge,y));v.open_find();
        let ratio=v.panes.comparison;
        v.on_mouse_move(&MouseMoveEvent{position:point(px(edge+80.),px(y)),pressed_button:Some(MouseButton::Left),modifiers:Default::default()},w,cx);
        assert!(v.panes.drag.is_none());assert_eq!(v.panes.comparison,ratio);
        assert!(!v.begin_resize(edge,y));
    }).unwrap();
    let (_,viewer)=merge_view::gui_tests::fixture(cx);
    viewer.update(cx,|v,cx| {
        let widths=v.merge_widths(v.panes.viewport[0]-v.diff_x());
        let edge=v.diff_x()+widths[0]+GUTTER_W/2.0;let y=v.top()-hdr_h()/2.0;
        assert!(v.begin_resize(edge,y));v.merge.as_mut().unwrap().begin_find(Default::default());
        assert!(!v.resize_move(edge+80.,y,true,cx));assert!(v.panes.drag.is_none());
        assert!(!v.begin_resize(edge,y));
    });
}

#[gpui::test]
fn gui_unequal_annotated_panes_keep_source_visible_and_pointer_mapping(cx:&mut TestAppContext) {
    let f=discard_fixture(cx);
    f.window.update(cx,|v,w,cx| {
        v.layout.set_files(false);v.toggle_blame(cx);let _=v.render(w,cx);
        for side in [Side::Left,Side::Right] {
            let edge=v.diff_x()+v.comparison_widths(v.panes.viewport[0]-v.diff_x())[0]+GUTTER_W/2.0;
            let y=v.top()-hdr_h()/2.0;assert!(v.begin_resize(edge,y));
            v.resize_move(if side==Side::Left{0.0}else{v.panes.viewport[0]},y,true,cx);
            v.on_mouse_up(&MouseUpEvent::default(),w,cx);let _=v.render(w,cx);
            let widths=v.comparison_widths(v.panes.viewport[0]-v.diff_x());
            assert!(v.blame_key(side).is_some());assert!(v.ann_w(side)<BLAME_W);
            assert!(widths[side as usize]-v.ann_w(side)-num_w()-16.0>=39.9);
            let start=if side==Side::Left{v.diff_x()}else{v.diff_x()+widths[0]+GUTTER_W};
            let x=start+v.ann_w(side)+num_w()+10.0;assert!(x<start+widths[side as usize]);
            let (hit,tx)=v.pane_at(w,x).unwrap();assert_eq!(hit,side);assert!((tx-2.0).abs()<0.1);
            v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(v.top()+line_h()/2.0)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx);
            assert_eq!(v.sel.unwrap().side,side);assert_eq!(v.sel.unwrap().head.line,0);
            v.on_mouse_up(&MouseUpEvent::default(),w,cx);
        }
    }).unwrap();
}

#[test]
fn gui_long_log_pauses_and_end_resumes_through_render() {
    use crate::git::testing::{commit_all, repo, write};
    let mut cx = TestAppContext::single();
    let f = fixture(&mut cx, "old\n", "new\n");
    let history = repo();
    for i in 0..40 {
        write(history.path(), "file", format!("{i}\n"));
        commit_all(history.path(), &format!("c{i}"));
    }
    let drain = |cx: &mut TestAppContext| {
        let until = Instant::now() + std::time::Duration::from_secs(10);
        while f.viewer.update(cx, |v, _| {
            let rows = v.log_rows();
            let p = v.repo.as_mut().map(|r| &mut r.picker).unwrap();
            p.poll_stream(rows);
            p.loading
        }) {
            assert!(Instant::now() < until, "the log did not pause or finish");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    f.viewer.update(&mut cx, |v, cx| {
        let mut p = Picker::new(history.path().into(), true);
        p.window = 5;
        p.focus = Pane::Log;
        p.start_stream();
        v.with_session(crate::repository_session::RepositorySession::with_picker(p), cx);
    });
    drain(&mut cx);
    let paused = f.viewer.read_with(&cx, |v, _| {
        let p = v.repo.as_ref().map(|r| &r.picker).unwrap();
        assert!(p.paused && !p.loading, "a long history pauses");
        p.log.len()
    });
    assert!(paused < 40);
    key(&mut cx, &f, "end", false, false, false);
    f.window.update(&mut cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
    assert!(f.viewer.read_with(&cx, |v, _| v.repo.as_ref().map(|r| &r.picker).unwrap().loading), "End in the log resumes reading on the next frame");
    drain(&mut cx);
    assert!(f.viewer.read_with(&cx, |v, _| v.repo.as_ref().map(|r| &r.picker).unwrap().log.len()) > paused);
}

/// The GUI opens repositories through the shared session, as `gui.rs` does: the first comparison is already
/// loaded, the log streams first, the sidebar inventory fills in, and a followed file's history opens once the
/// inventory has resolved the repository — all driven by the GUI's own poll loop.
#[gpui::test]
fn gui_opens_the_repository_through_the_shared_session(cx: &mut TestAppContext) {
    use crate::git::testing::{commit_all, repo, write};
    let f = fixture(cx, "old\n", "new\n");
    let history = repo();
    let root = history.path();
    write(root, "a.txt", "1\n");
    commit_all(root, "a1");
    write(root, "a.txt", "2\n");
    commit_all(root, "a2");
    let followed_commit = crate::git::testing::git(root, &["rev-parse", "HEAD"]).trim().to_string();
    write(root, "b.txt", "1\n");
    commit_all(root, "b1");
    let open = |cx: &mut TestAppContext, shown: bool, follow: Option<&str>| {
        let (set, spec, _) = crate::repository_session::initial_log_comparison(root, &crate::repository_session::local_spec()).unwrap();
        f.viewer.update(cx, |v, cx| {
            let set = Arc::new(set);
            let session = crate::repository_session::RepositorySession::open_loaded(root.into(), shown, None, follow.map(str::to_string), set.clone(), spec.clone());
            v.spec = spec;
            v.show_change_set(set, cx);
            v.with_session(session, cx);
        });
    };
    let picker = |v: &Viewer| {
        let p = &v.repo.as_ref().unwrap().picker;
        (p.anchor, p.log.len(), p.sources.branches.iter().map(|b| b.name.clone()).collect::<Vec<_>>(), p.file_paths.is_some())
    };
    open(cx, true, None);
    workflow_wait(cx, &f, |v| v.repo.as_ref().is_some_and(|r| !r.busy() && r.picker.sources.loaded && r.picker.log.len() == 3));
    f.viewer.read_with(cx, |v, _| {
        let (anchor, rows, branches, followed) = picker(v);
        assert_eq!(rows, 3);
        assert!(branches.iter().any(|b| b == "main"), "the sidebar inventory arrived: {branches:?}");
        assert_eq!(anchor, 1, "the clean repository opens on its HEAD commit");
        assert!(!followed);
    });
    // a hidden picker (Focus Mode) reads the inventory and the log only when it is shown
    open(cx, false, None);
    cx.executor().advance_clock(std::time::Duration::from_millis(100));
    cx.executor().run_until_parked();
    f.viewer.read_with(cx, |v, _| {
        let p = &v.repo.as_ref().unwrap().picker;
        assert!(!p.shown && !p.sources.loaded && p.log.is_empty() && !p.loading);
    });
    key(cx, &f, "escape", false, false, false);
    workflow_wait(cx, &f, |v| v.repo.as_ref().is_some_and(|r| !r.busy() && r.picker.shown && r.picker.sources.loaded && r.picker.log.len() == 3));
    open(cx, true, Some("a.txt"));
    workflow_wait(cx, &f, |v| v.repo.as_ref().is_some_and(|r| !r.busy()) && matches!(&v.spec, Spec::Git { paths, .. } if paths == &["a.txt"]));
    f.viewer.read_with(cx, |v, _| {
        let p = &v.repo.as_ref().unwrap().picker;
        assert_eq!(p.log.id(p.log_row(p.anchor).unwrap()), followed_commit, "the followed file's last commit is selected");
        let Spec::Git { revs, .. } = &v.spec else { panic!() };
        assert_eq!(revs.last(), Some(&followed_commit));
        assert_eq!(v.set.comparisons.len(), 1, "the comparison the session loaded is on screen");
        assert_eq!(v.set.comparisons[0].path, std::path::PathBuf::from("a.txt"));
    });
}

/// Sidebar sources run as the session's `Source` command: Enter on Uncommitted changes and on a stash shows
/// the comparison the session loaded.
#[gpui::test]
fn gui_sidebar_sources_load_through_the_shared_session(cx: &mut TestAppContext) {
    use crate::git::testing::{commit_all, git, repo, write};
    use crate::repository_session::SourceAction;
    let f = fixture(cx, "old\n", "new\n");
    let history = repo();
    let root = history.path();
    write(root, "a.txt", "1\n");
    commit_all(root, "a1");
    write(root, "a.txt", "stashed\n");
    git(root, &["stash", "-q"]);
    let stash = git(root, &["rev-parse", "stash@{0}"]).trim().to_string();
    write(root, "b.txt", "local\n");
    let (set, spec, _) = crate::repository_session::initial_log_comparison(root, &crate::repository_session::local_spec()).unwrap();
    f.viewer.update(cx, |v, cx| {
        let set = Arc::new(set);
        let session = crate::repository_session::RepositorySession::open_loaded(root.into(), true, None, None, set.clone(), spec.clone());
        v.spec = spec;
        v.show_change_set(set, cx);
        v.with_session(session, cx);
    });
    workflow_wait(cx, &f, |v| v.repo.as_ref().is_some_and(|r| !r.busy() && r.picker.sources.loaded));
    let choose = |cx: &mut TestAppContext, wanted: &dyn Fn(&SourceAction) -> bool| {
        let at = f.viewer.update(cx, |v, _| {
            let p = &mut v.repo.as_mut().unwrap().picker;
            p.focus = Pane::Sidebar;
            let rows = p.sources.rows(&v.palette, &p.branches);
            rows.iter().position(|r| r.action.as_ref().is_some_and(wanted)).unwrap_or_else(|| panic!("the sidebar lists the source: {:?}", rows.iter().map(|r| (&r.text, &r.action)).collect::<Vec<_>>()))
        });
        key(cx, &f, "home", false, false, false);
        for _ in 0..at {
            key(cx, &f, "down", false, false, false);
        }
        key(cx, &f, "enter", false, false, false);
    };
    // a stash first, so the local comparison below has to replace it; its section starts collapsed
    choose(cx, &|a| matches!(a, SourceAction::Toggle("Stashes")));
    choose(cx, &|a| matches!(a, SourceAction::Stash(sha) if *sha == stash));
    workflow_wait(cx, &f, |v| v.spec == crate::repository_session::stash_spec(&stash) && v.repo.as_ref().is_some_and(|r| !r.busy()));
    f.viewer.read_with(cx, |v, _| assert_eq!(v.set.comparisons.iter().map(|c| c.path.clone()).collect::<Vec<_>>(), ["a.txt"]));
    choose(cx, &|a| matches!(a, SourceAction::Local));
    workflow_wait(cx, &f, |v| v.spec == crate::repository_session::local_spec() && v.set.comparisons.iter().any(|c| c.path == "b.txt") && v.repo.as_ref().is_some_and(|r| !r.busy()));
}

/// The log filter, branch compare and file history run as session commands from the real keys: typing
/// filters at once, Enter searches commit messages in git, Esc leaves each state.
#[gpui::test]
fn gui_log_filters_compare_and_history_run_through_the_shared_session(cx: &mut TestAppContext) {
    use crate::git::testing::{commit_all, git, repo, write};
    let f = fixture(cx, "old\n", "new\n");
    let history = repo();
    let root = history.path();
    write(root, "a.txt", "1\n");
    commit_all(root, "alpha one");
    write(root, "b.txt", "a different file\n");
    commit_all(root, "beta two");
    git(root, &["branch", "topic"]);
    write(root, "a.txt", "2\n");
    commit_all(root, "alpha three");
    let (set, spec, _) = crate::repository_session::initial_log_comparison(root, &crate::repository_session::local_spec()).unwrap();
    f.viewer.update(cx, |v, cx| {
        let set = Arc::new(set);
        let session = crate::repository_session::RepositorySession::open_loaded(root.into(), true, None, None, set.clone(), spec.clone());
        v.with_session(session, cx);
    });
    let idle = |v: &Viewer| v.repo.as_ref().is_some_and(|r| !r.busy() && r.picker.sources.loaded);
    workflow_wait(cx, &f, |v| idle(v) && v.repo.as_ref().unwrap().picker.log.len() == 3);
    let shown = |cx: &mut TestAppContext| f.viewer.read_with(cx, |v, _| v.repo.as_ref().unwrap().picker.rows() - 1);
    f.viewer.update(cx, |v, _| v.repo.as_mut().unwrap().picker.focus = Pane::Log);
    key(cx, &f, "/", true, false, false);
    for c in ["b", "e", "t", "a", "x"] {
        key(cx, &f, c, false, false, false);
    }
    assert_eq!(shown(cx), 0, "typing filters the loaded log at once");
    key(cx, &f, "backspace", false, false, false);
    assert_eq!(shown(cx), 1);
    // Enter: git's own message search, run by the session
    for c in ["space", "t", "w", "o"] {
        key(cx, &f, c, false, false, false);
    }
    key(cx, &f, "enter", false, false, false);
    workflow_wait(cx, &f, |v| idle(v) && v.repo.as_ref().unwrap().picker.grep_ids.is_some());
    assert_eq!(shown(cx), 1);
    key(cx, &f, "escape", false, false, false);
    assert_eq!(shown(cx), 3, "Esc clears the filter");
    // branch compare: the log walks HEAD...topic and the comparison reads from topic's tip to the current one
    f.viewer.update(cx, |v, cx| v.compare_with_current("topic".into(), false, cx));
    workflow_wait(cx, &f, |v| idle(v) && matches!(&v.spec, Spec::Git { revs, .. } if revs == &["topic", "HEAD"]));
    assert_eq!(shown(cx), 1, "only the commit on one side");
    key(cx, &f, "escape", false, false, false);
    workflow_wait(cx, &f, |v| idle(v) && v.repo.as_ref().unwrap().picker.compare.is_none() && v.repo.as_ref().unwrap().picker.log.len() == 3);
    // file history, from a hidden picker: the session shows the picker and narrows the log to the file
    f.viewer.update(cx, |v, cx| v.set_picker_shown(false, cx));
    f.viewer.update(cx, |v, cx| v.show_file_history("b.txt".into(), cx));
    workflow_wait(cx, &f, |v| idle(v) && v.repo.as_ref().unwrap().picker.file_paths.is_some() && matches!(&v.spec, Spec::Git { paths, .. } if paths == &["b.txt"]));
    assert!(f.viewer.read_with(cx, |v, _| v.repo.as_ref().unwrap().picker.shown));
    assert_eq!(shown(cx), 1);
    key(cx, &f, "escape", false, false, false);
    assert_eq!(shown(cx), 3, "Esc leaves file history");
    // a repository opened in Focus Mode has read neither the sidebar nor the log; file history loads both
    f.viewer.update(cx, |v, cx| {
        let session = crate::repository_session::RepositorySession::open_loaded(root.into(), false, None, None, v.set.clone(), v.spec.clone());
        v.with_session(session, cx);
        v.show_file_history("b.txt".into(), cx);
    });
    workflow_wait(cx, &f, |v| idle(v) && !v.repo.as_ref().unwrap().picker.loading && matches!(&v.spec, Spec::Git { paths, .. } if paths == &["b.txt"]));
    assert_eq!(f.viewer.read_with(cx, |v, _| v.repo.as_ref().unwrap().picker.log.len()), 3);
    assert_eq!(shown(cx), 1);
}

/// Pull requests run through the shared session: one opened before the window (as `gui.rs` does) is adopted
/// with its commits in the log and its review loaded; `v` marks a file viewed and moves on, `r` refetches.
#[gpui::test]
fn gui_pull_requests_run_through_the_shared_session(cx: &mut TestAppContext) {
    let f = repository_markdown_fixture(cx);
    let provider = crate::pr::test_support::markdown_provider(f.dir.path());
    let opened = crate::pr_session::open(f.dir.path(), Some("9")).unwrap();
    let range = opened.fetched.commit_range();
    f.viewer.update(cx, |v, cx| {
        v.show_change_set(opened.set, cx);
        let session = crate::repository_session::RepositorySession::open_loaded(f.dir.path().into(), true, None, None, v.set.clone(), opened.fetched.spec());
        v.with_session(session, cx);
        v.adopt_pr(opened.pr, opened.fetched, cx);
    });
    let reviewed = |v: &Viewer| v.repo.as_ref().is_some_and(|r| !r.busy() && r.pr.as_ref().is_some_and(|s| s.pr.number == 9 && s.review.is_some()));
    workflow_wait(cx, &f, |v| reviewed(v) && v.doc().is_some());
    f.viewer.read_with(cx, |v, _| {
        let r = v.repo.as_ref().unwrap();
        assert_eq!(r.picker.branches, crate::history::Branches::One(range.clone()), "the log walks the PR's commits");
        assert!(r.fetched_pr_refs(), "its fetch refs are removed on quit");
    });
    // Viewed (`v` in the diff): viewed, and on to the next unviewed file at its first change
    let first = f.viewer.read_with(cx, |v, _| v.current);
    f.window.update(cx, |v, _, cx| { v.pr_key("v", false, cx); }).unwrap();
    workflow_wait(cx, &f, |v| v.current != first && v.doc().is_some());
    assert_eq!(f.viewer.read_with(cx, |v, _| v.view_status(first)), Some(crate::pr::ViewStatus::Viewed));
    // Refresh (toolbar, or `r`): refetched through the session; the review is back and the viewed mark kept
    run(cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Refresh));
    assert_eq!(f.viewer.read_with(cx, |v, _| v.toast.as_ref().map(|t| t.0.clone())).as_deref(), Some("Fetching pull request…"));
    workflow_wait(cx, &f, reviewed);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.view_status(first)), Some(crate::pr::ViewStatus::Viewed));
    let calls = std::fs::read_to_string(f.dir.path().join(".git/markdown-provider/requests.log")).unwrap();
    assert!(calls.matches("pr view 9").count() >= 2, "the refresh fetched the PR again: {calls}");
    // a later comparison (Compare with Branch…) is the one the session marks and places threads in
    // the PR's revisions the other way round: the same files, a comparison the PR did not load
    let (head, base) = f.viewer.read_with(cx, |v, _| { let Spec::Git { revs, .. } = &v.spec else { panic!() }; (revs[0].clone(), revs[1].clone()) });
    let revs = [base.clone(), head.clone()];
    f.viewer.update(cx, |v, cx| v.repository_command(crate::repository_session::RepositoryCommand::CompareRevs(base, head), cx));
    workflow_wait(cx, &f, |v| matches!(&v.spec, Spec::Git { revs: shown, .. } if shown[..] == revs[..]) && v.doc().is_some());
    f.viewer.read_with(cx, |v, _| assert!(std::ptr::eq(v.repo.as_ref().unwrap().comparisons(), &v.set.comparisons[..])));
    drop(provider);
}

/// The log's context menu lists the session's shared commands with their disabled reasons and runs them on the
/// clicked row; the Repository menu offers the staged changes (Index).
#[gpui::test]
fn gui_log_context_menu_and_index_run_the_shared_commands(cx: &mut TestAppContext) {
    use crate::git::testing::{commit_all, git, repo, write};
    use crate::repository_session::{RepositoryCommand as C, RepositorySession};
    let f = fixture(cx, "old\n", "new\n");
    let history = repo();
    let root = history.path();
    write(root, "a.txt", "1\n");
    commit_all(root, "first");
    let first = git(root, &["rev-parse", "HEAD"]).trim().to_string();
    write(root, "a.txt", "2\n");
    commit_all(root, "second");
    write(root, "staged.txt", "staged\n");
    git(root, &["add", "staged.txt"]);
    let (set, spec, _) = crate::repository_session::initial_log_comparison(root, &crate::repository_session::local_spec()).unwrap();
    f.viewer.update(cx, |v, cx| {
        let set = Arc::new(set);
        let session = RepositorySession::open_loaded(root.into(), true, None, None, set.clone(), spec.clone());
        v.spec = spec;
        v.show_change_set(set, cx);
        v.with_session(session, cx);
    });
    workflow_wait(cx, &f, |v| v.repo.as_ref().is_some_and(|r| !r.busy() && r.picker.log.len() == 2));
    let selection = |cx: &mut TestAppContext| f.viewer.read_with(cx, |v, _| (v.repo.as_ref().unwrap().picker.anchor, v.repo.as_ref().unwrap().picker.second));
    let before = selection(cx);
    // the older commit: log row 2 (row 0 is Uncommitted changes)
    let row = f.viewer.read_with(cx, |v, _| { let p = &v.repo.as_ref().unwrap().picker; p.display_row(p.log.row_of(&first).unwrap()).unwrap() });
    let (x, y) = f.viewer.read_with(cx, |v, _| (v.picker_w() + 60.0, toolbar_h() + (row as f32 + 1.5) * line_h()));
    f.window.update(cx, |v, w, cx| v.on_right_down(&MouseDownEvent { position: point(px(x), px(y)), button: MouseButton::Right, ..Default::default() }, w, cx)).unwrap();
    let file = f.viewer.read_with(cx, |v, _| v.set.comparisons[v.comparison.current].path.clone());
    let items = f.viewer.read_with(cx, |v, _| v.menu.as_ref().unwrap().items.iter().map(|(label, action)| (label.clone(), matches!(action, MenuAction::RepositoryRow { .. }))).collect::<Vec<_>>());
    let shared: Vec<_> = RepositorySession::log_context_commands(row, &file).iter().map(|c| crate::repository_session::command_label(c).to_string()).collect();
    assert_eq!(items.iter().filter(|(_, row)| *row).map(|(label, _)| label.clone()).collect::<Vec<_>>(), shared, "the shared commands, enabled on a commit row");
    let at = items.iter().position(|(label, _)| *label == crate::repository_session::command_label(&C::CopyHash)).unwrap();
    key(cx, &f, "home", false, false, false);
    for _ in 0..at {
        key(cx, &f, "down", false, false, false);
    }
    key(cx, &f, "enter", false, false, false);
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some(first.as_str()));
    assert_eq!(selection(cx), before, "copying from the menu keeps the selection");
    // Repository ▸ Show Staged Changes
    run(cx,&f,crate::catalog::Action::Repository(crate::catalog::RepoAction::StagedChanges));
    workflow_wait(cx, &f, |v| v.spec == crate::repository_session::index_spec() && v.repo.as_ref().is_some_and(|r| !r.busy()));
    f.viewer.read_with(cx, |v, _| assert_eq!(v.set.comparisons.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["staged.txt"]));
    // right click ▸ Compare with Branch… ▸ the first branch: the commit against it, through the session
    let open_menu = |cx: &mut TestAppContext| f.window.update(cx, |v, w, cx| v.on_right_down(&MouseDownEvent { position: point(px(x), px(y)), button: MouseButton::Right, ..Default::default() }, w, cx)).unwrap();
    let choose = |cx: &mut TestAppContext, label: &str| {
        let at = f.viewer.read_with(cx, |v, _| v.menu.as_ref().unwrap().items.iter().position(|(l, _)| l == label).unwrap_or_else(|| panic!("{label} in the menu")));
        key(cx, &f, "home", false, false, false);
        for _ in 0..at {
            key(cx, &f, "down", false, false, false);
        }
        key(cx, &f, "enter", false, false, false);
    };
    open_menu(cx);
    choose(cx, "Compare with Branch…");
    let (label, target) = f.viewer.read_with(cx, |v, _| v.repo.as_ref().unwrap().branch_targets(row).remove(0));
    choose(cx, &label);
    let C::CompareRevs(left, right) = target else { panic!() };
    workflow_wait(cx, &f, |v| matches!(&v.spec, Spec::Git { revs, .. } if revs[..] == [left.clone(), right.clone()]) && v.repo.as_ref().is_some_and(|r| !r.busy()));
    // Repository ▸ Copy commit or PR URL, with a GitHub remote
    git(root, &["remote", "add", "origin", "https://github.com/example/lenz-fixture.git"]);
    f.viewer.update(cx, |v, _| { let p = &mut v.repo.as_mut().unwrap().picker; p.anchor = row; p.second = None; });
    run(cx,&f,crate::catalog::Action::Repository(crate::catalog::RepoAction::CopyUrl));
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()), Some(format!("https://github.com/example/lenz-fixture/commit/{first}")));
}

/// Mouse and pane keys in the repository run session commands: a click on a log row shows that commit, a double
/// click enters Focus Mode, Alt-2 brings the sidebar back with the keyboard on it.
#[gpui::test]
fn gui_log_clicks_and_pane_keys_run_the_shared_commands(cx: &mut TestAppContext) {
    use crate::git::testing::{commit_all, git, repo, write};
    let f = fixture(cx, "old\n", "new\n");
    let history = repo();
    let root = history.path();
    write(root, "a.txt", "1\n");
    commit_all(root, "first");
    let first = git(root, &["rev-parse", "HEAD"]).trim().to_string();
    write(root, "a.txt", "2\n");
    commit_all(root, "second");
    let (set, spec, _) = crate::repository_session::initial_log_comparison(root, &crate::repository_session::local_spec()).unwrap();
    f.viewer.update(cx, |v, cx| {
        let set = Arc::new(set);
        let session = crate::repository_session::RepositorySession::open_loaded(root.into(), true, None, None, set.clone(), spec.clone());
        v.spec = spec;
        v.show_change_set(set, cx);
        v.with_session(session, cx);
    });
    workflow_wait(cx, &f, |v| v.repo.as_ref().is_some_and(|r| !r.busy() && r.picker.log.len() == 2));
    let row = f.viewer.read_with(cx, |v, _| { let p = &v.repo.as_ref().unwrap().picker; p.display_row(p.log.row_of(&first).unwrap()).unwrap() });
    let (x, y) = f.viewer.read_with(cx, |v, _| (v.picker_w() + 60.0, toolbar_h() + (row as f32 + 1.5) * line_h()));
    click(cx, &f, x, y);
    workflow_wait(cx, &f, |v| matches!(&v.spec, Spec::Git { revs, .. } if revs.last() == Some(&first)) && v.repo.as_ref().is_some_and(|r| !r.busy()));
    f.window.update(cx, |v, w, cx| v.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: point(px(x), px(y)), modifiers: Default::default(), click_count: 2, first_mouse: false }, w, cx)).unwrap();
    assert!(f.viewer.read_with(cx, |v, _| !v.repo.as_ref().unwrap().picker.shown), "a double click enters Focus Mode");
    key(cx, &f, "2", false, false, true);
    f.viewer.read_with(cx, |v, _| {
        assert!(v.layout.focused(v.repo.as_ref()), "Alt-2 does nothing in Focus");
        assert!(v.toast.as_ref().is_some_and(|(m, _)| m.contains("Leave Focus")), "and says why");
    });
    key(cx, &f, "escape", false, false, false);
    f.viewer.read_with(cx, |v, _| {
        let p = &v.repo.as_ref().unwrap().picker;
        assert!(p.shown && p.sidebar, "Esc returns to the Overview with its panels");
    });
}

#[gpui::test]
fn gui_modal_reports_the_topmost_dialog(cx:&mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),None);
    key(cx,&f,"g",true,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::GoTo));
    key(cx,&f,"escape",false,false,false);
    key(cx,&f,"/",false,true,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::Help));
    key(cx,&f,"escape",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),None);
}

/// A repository window over two commits touching two files, its log loaded; the TempDir holds the repository.
pub(super) fn repository_fixture(cx:&mut TestAppContext)->(Fixture,tempfile::TempDir){
    use crate::git::testing::{commit_all, repo, write};
    let f=fixture(cx,"old\n","new\n");
    let history=repo();let root=history.path();
    write(root,"a.txt","1\n");write(root,"b.txt","1\n");commit_all(root,"first");
    write(root,"a.txt","2\n");write(root,"b.txt","2\n");commit_all(root,"second");
    let (set,spec,_)=crate::repository_session::initial_log_comparison(root,&crate::repository_session::local_spec()).unwrap();
    f.viewer.update(cx,|v,cx|{
        let set=Arc::new(set);
        let session=crate::repository_session::RepositorySession::open_loaded(root.into(),true,None,None,set.clone(),spec.clone());
        v.spec=spec;v.show_change_set(set,cx);v.with_session(session,cx);
    });
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()&&r.picker.log.len()==2));
    (f,history)
}

#[gpui::test]
fn gui_focus_hides_branches_and_history_and_restores_them(cx:&mut TestAppContext) {
    use crate::layout::{LayoutCommand,Panels};
    let (f,_repo)=repository_fixture(cx);
    let before=f.viewer.read_with(cx,|v,_|v.layout.panels(v.repo.as_ref()));
    assert!(before.history);
    f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(LayoutCommand::ToggleFocus),0.,w,cx)).unwrap();
    f.viewer.read_with(cx,|v,_|{assert!(v.layout.focused(v.repo.as_ref()));assert_eq!(v.layout.panels(v.repo.as_ref()),Panels{files:before.files,..Panels::default()});});
    key(cx,&f,"escape",false,false,false);
    f.viewer.read_with(cx,|v,_|{assert!(!v.layout.focused(v.repo.as_ref()));assert_eq!(v.layout.panels(v.repo.as_ref()),before);});
}

#[gpui::test]
fn gui_f_enters_focus_keeping_files(cx:&mut TestAppContext) {
    use crate::layout::{LayoutCommand,Panels};
    let (f,_repo)=repository_fixture(cx);
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();}
    assert!(f.viewer.read_with(cx,|v,_|v.layout.files()));
    f.viewer.update(cx,|v,_|{v.nav_focused=false;if let Some(r)=v.repo.as_mut(){r.picker.focus=Pane::Diff;}});
    key(cx,&f,"f",false,false,false);
    f.viewer.read_with(cx,|v,_|assert_eq!(v.layout.panels(v.repo.as_ref()),Panels{files:true,..Panels::default()}));
}

#[gpui::test]
fn gui_focus_survives_repository_refresh(cx:&mut TestAppContext) {
    use crate::layout::{LayoutCommand,Panels};
    let (f,_repo)=repository_fixture(cx);
    f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(LayoutCommand::ToggleFocus),0.,w,cx)).unwrap();
    f.viewer.update(cx,|v,cx|v.refresh_repository(cx));
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()));
    f.viewer.read_with(cx,|v,_|{assert!(v.layout.focused(v.repo.as_ref()));assert_eq!(v.layout.panels(v.repo.as_ref()),Panels::default());});
}

#[gpui::test]
fn gui_focus_is_unavailable_without_repository(cx:&mut TestAppContext) {
    use crate::layout::LayoutCommand;
    let f=files_fixture(cx);
    assert!(f.viewer.read_with(cx,|v,_|v.layout.files()));
    f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(LayoutCommand::ToggleFocus),0.,w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.layout.files()&&!v.layout.focused(v.repo.as_ref())));
}

/// Run `action` through the palette with the keyboard, from the diff's focus.
pub(super) fn run(cx:&mut TestAppContext,f:&Fixture,action:crate::catalog::Action){
    for _ in 0..8 {
        if f.viewer.read_with(cx,|v,_| !v.nav_focused && !v.panel_focused && v.repo.as_ref().map(|r| &r.picker).is_none_or(|p|!p.shown || p.focus==Pane::Diff)) {break;}
        key(cx,f,"tab",false,false,false);
    }
    key(cx,f,"p",true,true,false);
    let index=f.viewer.read_with(cx,|v,_|v.command_palette.as_ref().expect("palette opens").rows().position(|r|r.entry.action==action).unwrap_or_else(||panic!("{action:?} listed")));
    key(cx,f,"home",false,false,false);
    for _ in 0..index {key(cx,f,"down",false,false,false);}
    assert_eq!(f.viewer.read_with(cx,|v,_|v.command_palette.as_ref().unwrap().selected()),Some(index));
    key(cx,f,"enter",false,false,false);
}

#[gpui::test]
fn gui_palette_opens_from_every_route_and_runs_commands(cx:&mut TestAppContext) {
    use crate::catalog::Action;
    let f=fixture(cx,"old\n","new\n");
    for (name,ctrl,shift) in [("f10",false,false),(":",false,false),("p",true,true)] {
        key(cx,&f,name,ctrl,shift,false);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::Palette),"{name}");
        key(cx,&f,"escape",false,false,false);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),None);
    }
    run(cx,&f,Action::Edit(crate::edit_session::EditMenuCommand::ApplyRight));
    assert_eq!(text(&f),"new\n");
    run(cx,&f,Action::Edit(crate::edit_session::EditMenuCommand::Save));
    assert_eq!(text(&f),"old\n");
    run(cx,&f,Action::Comparison(Command::SetWhitespace(WhitespaceMode::Ignore)));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.options.whitespace),WhitespaceMode::Ignore);
}

#[gpui::test]
fn gui_palette_typing_filters_and_disabled_rows_explain(cx:&mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    key(cx,&f,"p",true,true,false);
    for c in "copy sel".chars() {key(cx,&f,&c.to_string(),false,false,false);}
    f.viewer.read_with(cx,|v,_|{let p=v.command_palette.as_ref().unwrap();assert_eq!(p.query(),"copy sel");assert_eq!(p.rows().next().unwrap().disabled.as_deref(),Some("No selected text"));});
    let bytes=text(&f);
    key(cx,&f,"enter",false,false,false);
    assert_eq!(text(&f),bytes);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()),"a disabled row keeps the palette open with its reason");
    for c in "zzzz".chars() {key(cx,&f,&c.to_string(),false,false,false);}
    key(cx,&f,"enter",false,false,false);
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_none()));
    assert_eq!(text(&f),bytes);
}

#[gpui::test]
fn gui_palette_and_diff_options_take_the_wheel_from_the_document(cx:&mut TestAppContext) {
    let long:String=(0..400).map(|i|format!("line {i}\n")).collect();
    let f=fixture(cx,&long,&long.replace("line 7\n","line seven\n"));
    let wheel=|cx:&mut TestAppContext,dy:f32|{f.window.update(cx,|v,w,cx|v.on_wheel(&ScrollWheelEvent{position:point(px(v.diff_x()+50.),px(v.top()+10.)),
        delta:ScrollDelta::Lines(point(0.,dy)),..Default::default()},w,cx)).unwrap();};
    key(cx,&f,"p",true,true,false);
    let (anchor,row)=f.viewer.read_with(cx,|v,_|(v.comparison.view.anchor_line(),v.command_palette.as_ref().unwrap().selected()));
    wheel(cx,-1.);
    f.viewer.read_with(cx,|v,_|{
        assert_eq!(v.comparison.view.anchor_line(),anchor,"the document stays put under the palette");
        assert_eq!(v.command_palette.as_ref().unwrap().selected(),row.map(|r|r+1),"the wheel moves the palette's row");
    });
    key(cx,&f,"escape",false,false,false);
    f.viewer.update(cx,|v,cx|v.toggle_diff_options(100.,cx));
    assert!(f.viewer.read_with(cx,|v,_|v.diff_options.is_some()));
    wheel(cx,-3.);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.view.anchor_line()),anchor,"nor under the Diff options popover");
}

#[gpui::test]
fn gui_palette_clamps_in_tiny_windows(cx:&mut TestAppContext) {
    let f=fixture(cx,"left\n","right\n");
    for [width,height] in [[900.,650.],[400.,240.],[240.,160.]] {
        cx.simulate_window_resize(f.window.into(),gpui::size(px(width),px(height)));cx.executor().run_until_parked();
        f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
        key(cx,&f,"f10",false,false,false);
        key(cx,&f,"end",false,false,false);key(cx,&f,"up",false,false,false);
        f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
        f.viewer.update(cx,|v,_|{
            let (x,y,w,rows)=v.palette_layout();
            assert!(x>=0.&&y>=0.&&x+w<=width&&y+(rows+1) as f32*line_h()+8.<=height,"{width}x{height}");
            let p=v.command_palette.as_mut().unwrap();let sel=p.selected().unwrap();assert!(p.window(rows).contains(&sel),"{width}x{height}");
        });
        key(cx,&f,"escape",false,false,false);assert_eq!(text(&f),"right\n");
    }
}

#[gpui::test]
fn gui_palette_closes_before_actions_that_replace_the_comparison(cx:&mut TestAppContext) {
    let f=files_fixture(cx);
    run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_none()));
    run(cx,&f,crate::catalog::Action::Comparison(Command::NextFile));
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_none()&&v.current==1));
}

#[gpui::test]
fn gui_new_modals_yield_to_questions(cx:&mut TestAppContext) {
    use crate::merge_session::Ask;
    let (dir,viewer)=merge_view::gui_tests::fixture(cx);
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
    let f=Fixture{dir,viewer,window};
    key(cx,&f,"enter",true,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.merge.as_ref().unwrap().ask),Some(Ask::DoneWithConflicts));
    for (name,ctrl,shift) in [("f10",false,false),(":",false,false),("p",true,true)] {
        key(cx,&f,name,ctrl,shift,false);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::Question),"{name}");
        assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_none()));
    }
}

fn toolbar_ids(cx:&mut TestAppContext,f:&Fixture,width:f32)->Vec<&'static str>{
    cx.simulate_window_resize(f.window.into(),gpui::size(px(width),px(600.)));cx.executor().run_until_parked();
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);v.toolbar_ids()}).unwrap()
}

#[gpui::test]
fn gui_toolbar_shows_frequent_actions_and_collapses_by_width(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    // Focus is leftmost, where Focus's Overview is
    assert_eq!(toolbar_ids(cx,&f,1200.),["app-menu","focus","nav-back","nav-forward","prev-change","next-change","prev-file","next-file","diff-options","help"]);
    assert_eq!(toolbar_ids(cx,&f,800.),["app-menu","focus","nav-back","nav-forward","prev-change","next-change","prev-file","next-file","diff-options","help"]);
    assert_eq!(toolbar_ids(cx,&f,500.),["app-menu","focus","nav-back","nav-forward","prev-change","next-change","help"]);
    assert_eq!(toolbar_ids(cx,&f,350.),["app-menu","focus","nav-back","nav-forward","prev-change","next-change","help"]);
}

#[gpui::test]
fn gui_editable_toolbar_hides_edit_buttons_even_with_a_dirty_draft(cx:&mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    assert_eq!(toolbar_ids(cx,&f,1200.),["app-menu","prev-change","next-change","prev-file","next-file","diff-options","help"]);
    assert_eq!(toolbar_ids(cx,&f,500.),["app-menu","prev-change","next-change","help"]);
    key(cx,&f,"right",false,true,true);
    assert!(f.viewer.read_with(cx,|v,_|v.edits.dirty(v.comparison.current)));
    for width in [1200.,500.] {assert!(!toolbar_ids(cx,&f,width).iter().any(|id|id.starts_with("edit-")));}
}

#[gpui::test]
fn gui_merge_and_picker_toolbars_have_no_find_and_no_palette_button(cx:&mut TestAppContext) {
    let (dir,viewer)=merge_view::gui_tests::fixture(cx);
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|viewer.clone())).unwrap();
    let f=Fixture{dir,viewer,window};
    let ids=toolbar_ids(cx,&f,1200.);
    assert!(!ids.iter().any(|id|id.contains("find")),"{ids:?}");
    for (id,label) in [("m-undo","↶"),("m-redo","↷"),("m-save","⤓")] {assert!(toolbar_labels(cx,&f,1200.).contains(&(id,label.to_string())));}
    assert_eq!(ids.first(),Some(&"app-menu"));
    assert!(!ids.contains(&"palette"),"no ⋯ button: the palette is on Ctrl/Cmd+Shift+P, F10 and :");
    let picker=merge_dialog_fixture(cx,true);
    for width in [1200.,400.] {
        for (f,prefix) in [(&f,"m"),(&picker,"p")] {
            let labels=toolbar_labels(cx,f,width);
            for (suffix,label) in [("undo","↶"),("redo","↷"),("save","⤓")] {
                assert!(labels.iter().any(|(id,text)|*id==format!("{prefix}-{suffix}")&&text==label),"{labels:?}");
            }
        }
    }
}

fn enter_focus(cx:&mut TestAppContext,f:&Fixture){
    f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFocus),0.,w,cx)).unwrap();
}

fn toolbar_labels(cx:&mut TestAppContext,f:&Fixture,width:f32)->Vec<(&'static str,String)>{
    toolbar_ids(cx,f,width);
    f.viewer.read_with(cx,|v,_|v.toolbar_slots().into_iter().map(|s|(s.id,s.label.into_owned())).collect())
}

#[gpui::test]
fn gui_focus_toolbar_slots(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    enter_focus(cx,&f);
    assert_eq!(toolbar_ids(cx,&f,1200.),["app-menu","overview","nav-back","nav-forward","prev-change","next-change","prev-file","next-file","diff-options","help"]);
    assert!(f.viewer.read_with(cx,|v,_|v.focus_title()).is_some_and(|t|!t.text.is_empty()));
}

#[gpui::test]
fn gui_focus_toolbar_widths(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    enter_focus(cx,&f);
    let labels=toolbar_labels(cx,&f,1200.);
    assert!(labels.contains(&("diff-options","⚙".to_string()))&&!labels.iter().any(|l|l.0=="unchanged"),"no ≡ Unchanged: the Diff menu has it: {labels:?}");
    assert_eq!(toolbar_ids(cx,&f,400.),["app-menu","overview","nav-back","nav-forward","prev-change","next-change","help"]);
}

#[gpui::test]
fn gui_plain_window_is_focus_without_overview(cx:&mut TestAppContext) {
    let f=files_fixture(cx);
    let ids=toolbar_ids(cx,&f,1200.);
    assert!(ids.contains(&"diff-options")&&!ids.contains(&"overview")&&!ids.contains(&"focus")&&ids.first()==Some(&"app-menu"),"{ids:?}");
    assert!(f.viewer.read_with(cx,|v,_|v.focus_title()).is_none());
    // Esc keeps its old meaning: no layout change
    let files=f.viewer.read_with(cx,|v,_|v.layout.files());
    key(cx,&f,"escape",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.layout.files()),files);
}

#[gpui::test]
fn gui_focus_diff_menu_has_the_layout_and_runs_its_commands(cx:&mut TestAppContext) {
    let f=fixture(cx,"a  b\nx\n","a b\ny\n");
    toolbar_ids(cx,&f,1200.);
    f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::DiffOptions,300.,w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::DiffOptions),"the Overview's Diff menu, in Focus too");
    // Layout is the first row: Split | Unified
    let (x,y)=f.viewer.read_with(cx,|v,_|v.diff_option_point(0,1));
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.view.unified()));
    let (x,y)=f.viewer.read_with(cx,|v,_|v.diff_option_point(1,1));
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.options.whitespace),WhitespaceMode::Trim);
    key(cx,&f,"escape",false,false,false);
    f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::Help,300.,w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.help),"? opens the shortcuts");
}

#[gpui::test]
fn gui_diff_popover_sets_options_and_blocks_input_underneath(cx:&mut TestAppContext) {
    let f=fixture(cx,"a  b\n","a b\n");
    f.window.update(cx,|v,w,cx|v.tool(Tool::DiffOptions,0.,w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::DiffOptions));
    // row 0 is Layout; Whitespace below it
    key(cx,&f,"down",false,false,false);key(cx,&f,"right",false,false,false);key(cx,&f,"right",false,false,false);key(cx,&f,"enter",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.options.whitespace),WhitespaceMode::Ignore);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::DiffOptions),"stays open for more changes");
    let bytes=text(&f);
    key(cx,&f,"right",false,true,true);
    key(cx,&f,"s",true,false,false);
    assert_eq!(text(&f),bytes,"document keys do not reach the comparison");
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.diff_options.is_none()));
    f.window.update(cx,|v,w,cx|v.tool(Tool::DiffOptions,0.,w,cx)).unwrap();
    key(cx,&f,"down",false,false,false);key(cx,&f,"down",false,false,false);key(cx,&f,"right",false,false,false);key(cx,&f,"enter",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.options.highlight),HighlightLevel::Char);
    f.window.update(cx,|v,w,cx|v.tool(Tool::DiffOptions,0.,w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.diff_options.is_none()),"the button toggles the popover");
}

#[gpui::test]
fn gui_diff_popover_clicks_choose_values_and_outside_closes(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\n","b\n");
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);v.tool(Tool::DiffOptions,0.,w,cx)}).unwrap();
    let (x,y)=f.viewer.read_with(cx,|v,_|v.diff_option_point(3,2));
    click(cx,&f,x,y);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.view.mode()),ScrollMode::Padded);
    click(cx,&f,1.,500.);
    assert!(f.viewer.read_with(cx,|v,_|v.diff_options.is_none()));
}

#[gpui::test]
fn gui_markdown_switch_lives_in_the_diff_header(cx:&mut TestAppContext) {
    let f=fixture_named(cx,"# a\n","# b\n","md");
    assert!(!toolbar_ids(cx,&f,1200.).iter().any(|id|id.starts_with("markdown-")));
    assert!(f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);!v.header_switch_labels().is_empty()}).unwrap());
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Comparison(Command::SetPresentation(crate::markdown::PresentationMode::Rendered)),w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.markdown_active()));
    let g=fixture(cx,"a\n","b\n");
    assert!(!g.window.update(cx,|v,w,cx|{let _=v.render(w,cx);!v.header_switch_labels().is_empty()}).unwrap());
}

#[gpui::test]
fn gui_panel_toggles_and_palette_button_dispatch(cx:&mut TestAppContext) {
    use crate::layout::LayoutCommand;
    let (f,_repo)=repository_fixture(cx);
    f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(LayoutCommand::ToggleHistory),0.,w,cx)).unwrap();
    assert!(!f.viewer.read_with(cx,|v,_|v.layout.panels(v.repo.as_ref()).history));
    f.viewer.update(cx,|v,cx|v.open_palette(cx));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.modal()),Some(Modal::Palette));
}

fn right_click_labels(cx:&mut TestAppContext,f:&Fixture,side:Side,line:u32)->Vec<String>{
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);let (x,y)=v.point_on_line(w,side,line);
        v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx);}).unwrap();
    let labels=f.viewer.read_with(cx,|v,_|v.menu.as_ref().map_or_else(Vec::new,|m|m.items.iter().map(|(l,_)|l.clone()).collect()));
    key(cx,f,"escape",false,false,false);labels
}

#[gpui::test]
fn gui_text_context_menu_lists_only_what_is_under_the_pointer(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\nb\nc\nkeep\n","a\nB\nC\nkeep\n");
    let labels=right_click_labels(cx,&f,Side::Right,1);
    assert!(labels.len()<=8,"{labels:?}");
    assert!(!labels.iter().any(|l|l.ends_with('›')),"no submenus: {labels:?}");
    for gone in ["Undo","Save","Copy file path",commands::Interaction::DefaultApp.label()] {assert!(!labels.iter().any(|l|l.starts_with(gone)),"{gone} in {labels:?}");}
    assert!(labels.iter().any(|l|l.starts_with("Fold Change Block")),"multiline block folds: {labels:?}");
    assert!(labels.iter().any(|l|l.starts_with("Move change")),"{labels:?}");
    assert!(labels.iter().any(|l|l==commands::Interaction::Editor.label()),"{labels:?}");
}

#[gpui::test]
fn gui_single_line_block_has_no_fold_item(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\nb\nkeep\n","a\nB\nkeep\n");
    let labels=right_click_labels(cx,&f,Side::Right,1);
    assert!(!labels.iter().any(|l|l.contains("Fold")),"{labels:?}");
}

#[gpui::test]
fn gui_image_header_switch_offers_and_sets_the_modes(cx:&mut TestAppContext) {
    let f=fixture(cx,"old","new");let lp=f.dir.path().join("left.png");let rp=f.dir.path().join("right.png");
    image::RgbaImage::from_pixel(2,2,image::Rgba([0,0,0,255])).save(&lp).unwrap();
    image::RgbaImage::from_pixel(2,2,image::Rgba([255,0,0,255])).save(&rp).unwrap();
    let spec=Spec::Plain{left:lp,right:rp};let set=ChangeSet::load(&spec,f.dir.path()).unwrap();
    f.viewer.update(cx,|v,cx|{v.spec=spec;v.show_change_set(set,cx);});
    workflow_wait(cx,&f,|v|v.image_doc().is_some());
    assert_eq!(f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);v.header_switch_labels()}).unwrap(),["2-up","Swipe","Onion skin","Difference"]);
    let (x,y)=f.viewer.read_with(cx,|v,_|{let cell=v.header_switch().into_iter().find(|c|c.2=="Difference").unwrap();(cell.0+cell.1/2.,v.top()-hdr_h()/2.)});
    click(cx,&f,x,y);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.iview.state().mode()),crate::image_session::Mode::Difference);
    assert!(f.viewer.read_with(cx,|v,_|v.header_switch().iter().any(|c|c.2=="Difference"&&c.3)));
}

#[gpui::test]
fn gui_annotations_can_be_hidden_from_the_line_menu(cx:&mut TestAppContext) {
    let f=discard_fixture(cx);
    f.viewer.update(cx,|v,cx|v.toggle_blame(cx));
    workflow_wait(cx,&f,|v|v.blame_key(Side::Right).and_then(|k|v.blames.get(&k)).is_some_and(|b|b.line(0).is_some()));
    let labels=right_click_labels(cx,&f,Side::Right,0);
    assert!(labels.iter().any(|l|l=="Hide annotations"),"{labels:?}");
    assert!(!labels.iter().any(|l|l.contains("Annotate")),"{labels:?}");
    f.window.update(cx,|v,w,cx|{let (x,y)=v.point_on_line(w,Side::Right,0);v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx);}).unwrap();
    let row=f.viewer.read_with(cx,|v,_|v.menu.as_ref().unwrap().items.iter().position(|(l,_)|l=="Hide annotations").unwrap());
    key(cx,&f,"home",false,false,false);for _ in 0..row{key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);
    assert!(!f.viewer.read_with(cx,|v,_|v.blame_on));
    assert!(right_click_labels(cx,&f,Side::Right,0).iter().any(|l|l=="Annotate with blame"));
}

#[gpui::test]
fn gui_app_menu_lists_settings_theme_keys_about_and_report(cx:&mut TestAppContext) {
    // ☰ is the Overview's; Focus has ⚙ and ?
    let (f,_repo)=repository_fixture(cx);
    f.window.update(cx,|v,w,cx|v.tool(Tool::AppMenu,900.,w,cx)).unwrap();
    let labels=f.viewer.read_with(cx,|v,_|v.menu.as_ref().unwrap().items.iter().map(|(l,_)|l.clone()).collect::<Vec<_>>());
    assert_eq!(labels,["Settings…",commands::Interaction::ChangeTheme.label(),"Keyboard shortcuts",commands::Interaction::About.label(),commands::Interaction::ReportLastError.label()]);
    assert!(f.viewer.read_with(cx,|v,_|v.toolbar_slots().iter().any(|s|s.id=="app-menu"&&s.active)),"the open menu marks its button");
    key(cx,&f,"home",false,false,false);key(cx,&f,"down",false,false,false);key(cx,&f,"down",false,false,false);key(cx,&f,"enter",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.help),"Keyboard shortcuts opens help");
    key(cx,&f,"escape",false,false,false);
    f.window.update(cx,|v,w,cx|v.tool(Tool::AppMenu,900.,w,cx)).unwrap();
    f.window.update(cx,|v,w,cx|v.tool(Tool::AppMenu,900.,w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.menu.is_none()),"the button toggles its menu");
}

#[gpui::test]
fn gui_header_switch_waits_for_the_document(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\n","b\n");
    f.viewer.update(cx,|v,_|v.loaded.clear());
    assert!(f.viewer.read_with(cx,|v,_|v.doc().is_none()&&v.header_switch_labels().is_empty()),"no Source | Preview flash while a file loads");
}

#[gpui::test]
fn gui_switching_branches_keeps_the_old_history_until_new_rows_arrive(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    let before=f.viewer.update(cx,|v,_|v.shown_log_frame().map(|frame|frame.rows.len()).unwrap_or(0));
    assert!(before>1);
    f.viewer.update(cx,|v,cx|v.repository_command(crate::repository_session::RepositoryCommand::Walk(crate::history::Branches::All),cx));
    f.viewer.update(cx,|v,_|{
        let p=&v.repo.as_ref().unwrap().picker;
        assert!(p.log.is_empty()&&p.loading,"the log restarts");
        let held=v.shown_log_frame().expect("a frame is shown");
        assert_eq!(held.rows.len(),before,"no empty history between the click and the first new rows");
    });
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()));
    let after=f.viewer.update(cx,|v,_|v.shown_log_frame().map(|frame|frame.rows.len()).unwrap_or(0));
    assert_eq!(after,before);
}

#[gpui::test]
fn gui_a_new_change_set_replaces_the_old_one_with_its_first_file_read(cx:&mut TestAppContext) {
    let (f,repo)=repository_fixture(cx);
    workflow_wait(cx,&f,|v|v.doc().is_some());
    let old=f.viewer.read_with(cx,|v,_|v.set.clone());
    let spec=Spec::Git{cached:false,revs:vec!["HEAD~1".into(),"HEAD".into()],paths:Vec::new(),untracked:false};
    let set=Arc::new(ChangeSet::load(&spec,repo.path()).unwrap());
    f.viewer.update(cx,|v,cx|{
        v.stage_change_set(set.clone(),spec.clone(),cx);
        assert!(Arc::ptr_eq(&v.set,&old)&&v.doc().is_some(),"the old comparison stays on screen while the new first file is read");
    });
    workflow_wait(cx,&f,|v|Arc::ptr_eq(&v.set,&set));
    f.viewer.read_with(cx,|v,_|{assert!(v.doc().is_some(),"the new comparison arrives with its first file");assert_eq!(v.spec,spec);});
}

#[gpui::test]
fn gui_open_popups_hide_the_pane_dividers(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    assert!(!f.viewer.read_with(cx,|v,_|v.resize_handles().is_empty()));
    for open in [Tool::DiffOptions,Tool::AppMenu] {
        f.window.update(cx,|v,w,cx|v.tool(open,300.,w,cx)).unwrap();
        assert!(f.viewer.read_with(cx,|v,_|v.resize_handles().is_empty()),"no divider drawn over an open popup");
        key(cx,&f,"escape",false,false,false);
    }
}

#[gpui::test]
fn gui_toolbar_clicks_close_an_open_dropdown_first(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    let open=|v:&Viewer|v.menu.is_some()||v.command_palette.is_some()||v.diff_options.is_some();
    for button in [Tool::DiffOptions,Tool::AppMenu] {
        f.window.update(cx,|v,w,cx|v.toolbar_click(button,300.,w,cx)).unwrap();
        assert!(f.viewer.read_with(cx,|v,_|open(v)),"{button:?} opens");
        f.window.update(cx,|v,w,cx|v.toolbar_click(button,300.,w,cx)).unwrap();
        assert!(f.viewer.read_with(cx,|v,_|!open(v)),"{button:?} again closes it");
        f.window.update(cx,|v,w,cx|v.toolbar_click(button,300.,w,cx)).unwrap();
        let focus_before=f.viewer.read_with(cx,|v,_|v.layout.focused(v.repo.as_ref()));
        f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::Layout(crate::layout::LayoutCommand::ToggleFocus),300.,w,cx)).unwrap();
        f.viewer.read_with(cx,|v,_|{assert!(!open(v),"another button closes the dropdown");assert_eq!(v.layout.focused(v.repo.as_ref()),focus_before,"and does nothing else");});
    }
}

#[gpui::test]
fn gui_unavailable_toolbar_buttons_look_it_and_do_nothing(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\n","b\n");
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    f.viewer.read_with(cx,|v,_|{
        let slots=v.toolbar_slots();let dim=|id:&str|slots.iter().find(|s|s.id==id).unwrap().dim;
        assert!(dim("prev-file")&&dim("next-file"),"one file: no previous or next file");
        assert!(!dim("help"));
    });
    let before=f.viewer.read_with(cx,|v,_|v.current);
    f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::NextFile,0.,w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.current),before);
}

#[gpui::test]
fn gui_pr_has_no_toolbar_menu_and_refreshes_from_its_sidebar_row(cx:&mut TestAppContext) {
    let f=fixture(cx,"old\n","new\n");
    f.viewer.update(cx,|v,_|{
        let pr:crate::pr::PullRequest=serde_json::from_str(include_str!("../../fixtures/pr/view.json")).unwrap();
        let number=pr.number;
        let repo=v.repo.get_or_insert_with(||crate::repository_session::RepositorySession::with_picker(Picker::new(f.dir.path().into(),true)));
        repo.pr=Some(crate::pr_session::PrState::new(pr,crate::pr::Fetched{number,merge_base:"base".into(),head:"head".into()}));
        repo.picker.sources.prs=vec![crate::pr::ListedPr{number,title:"The PR".into(),author:crate::pr::Account{login:"a".into()},head_ref_name:"h".into(),base_ref_name:"b".into(),is_draft:false,updated_at:String::new()}];
        repo.picker.sources.loaded=true;
    });
    let ids=toolbar_ids(cx,&f,1200.);
    assert!(!ids.contains(&"pr-menu")&&!ids.contains(&"pr-viewed"),"no PR ▾ and no Viewed: {ids:?}");
    // a right click on the open PR's row offers Refresh
    let (x,y)=f.viewer.read_with(cx,|v,_|{
        let p=&v.repo.as_ref().unwrap().picker;
        let row=p.sources.rows(&v.palette,&p.branches).iter().position(|r|matches!(r.action,Some(crate::repository_session::SourceAction::Pr(_)))).expect("a PR row");
        (v.picker_w()/2.,toolbar_h()+(row as f32+1.5-p.sources.scroll)*line_h())
    });
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    let labels=f.viewer.read_with(cx,|v,_|v.menu.as_ref().map_or_else(Vec::new,|m|m.items.iter().map(|(l,_)|l.clone()).collect::<Vec<_>>()));
    assert_eq!(labels,["Refresh pull request"]);
}

#[gpui::test]
fn gui_back_returns_from_file_history_and_forward_goes_again(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    let ids=toolbar_ids(cx,&f,1200.);
    assert_eq!(ids[..4],["app-menu","focus","nav-back","nav-forward"]);
    assert!(f.viewer.read_with(cx,|v,_|v.toolbar_slots().iter().filter(|s|s.id.starts_with("nav-")).all(|s|s.dim)),"nowhere to go yet");
    f.viewer.update(cx,|v,cx|v.repository_command(crate::repository_session::RepositoryCommand::FileHistory("a.txt".into()),cx));
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()&&r.picker.file_paths.is_some()));
    key(cx,&f,"left",false,false,true);
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()&&r.picker.file_paths.is_none()));
    f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::Forward,0.,w,cx)).unwrap();
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()&&r.picker.file_paths.is_some()));
    // Alt+Left/Right no longer switch files; [ and ] still do
    let file=f.viewer.read_with(cx,|v,_|v.current);
    key(cx,&f,"right",false,false,true);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.current),file);
}

#[gpui::test]
fn gui_palette_runs_repository_history_commands(cx:&mut TestAppContext){
    use crate::catalog::{Action,RepoAction};
    let (f,_dir)=repository_fixture(cx);
    run(cx,&f,Action::Repository(RepoAction::AllBranches));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().picker.branches.clone()),crate::history::Branches::All);
    run(cx,&f,Action::Repository(RepoAction::Filter));
    assert!(f.viewer.read_with(cx,|v,_|{let p=&v.repo.as_ref().unwrap().picker;p.filter_editing&&p.focus==crate::repository_session::Pane::Log}));
    // nothing selected in the sidebar: the compare says why instead of running
    run(cx,&f,Action::Repository(RepoAction::CompareBranch{merge_base:true}));
    assert!(f.viewer.read_with(cx,|v,_|v.toast.as_ref().is_some_and(|t|t.0.contains("Select a branch"))));
}

#[gpui::test]
fn gui_history_filter_from_focus_restores_the_panels_focus_hid(cx:&mut TestAppContext){
    use crate::catalog::{Action,RepoAction};
    let (f,_dir)=repository_fixture(cx);
    let before=f.viewer.read_with(cx,|v,_|v.layout.panels(v.repo.as_ref()));
    run(cx,&f,Action::Layout(crate::layout::LayoutCommand::ToggleFocus));
    assert!(f.viewer.read_with(cx,|v,_|v.layout.focused(v.repo.as_ref())));
    run(cx,&f,Action::Repository(RepoAction::Filter));
    let after=f.viewer.read_with(cx,|v,_|v.layout.panels(v.repo.as_ref()));
    assert_eq!((after.branches,after.history),(before.branches,true),"Focus ends: the panels it hid come back");
}

fn nav_file_point(cx:&mut TestAppContext,f:&Fixture)->(f32,f32){
    f.viewer.read_with(cx,|v,_|{
        let row=v.nav.iter().position(|r|matches!(r,NavRow::File{..})).expect("a file row");
        (v.nav_x()+40.,v.top()+(row as f32-v.nav_scroll+0.5)*line_h())
    })
}

#[gpui::test]
fn gui_files_enter_and_double_click_enter_focus(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();}
    toolbar_ids(cx,&f,1200.);
    f.viewer.update(cx,|v,_|v.nav_focused=true);
    key(cx,&f,"enter",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.layout.focused(v.repo.as_ref())),"Enter on Files enters Focus");
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|!v.layout.focused(v.repo.as_ref())));
    toolbar_ids(cx,&f,1200.);
    let (x,y)=nav_file_point(cx,&f);
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|!v.layout.focused(v.repo.as_ref())),"one click only opens the file");
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:2,first_mouse:false},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.layout.focused(v.repo.as_ref())),"double-click on a file enters Focus");
}

#[gpui::test]
fn gui_escape_closes_the_dropdown_before_leaving_focus(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    enter_focus(cx,&f);
    f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::DiffOptions,300.,w,cx)).unwrap();
    key(cx,&f,"escape",false,false,false);
    f.viewer.read_with(cx,|v,_|{assert!(v.diff_options.is_none());assert!(v.layout.focused(v.repo.as_ref()),"first Esc only closes the menu");});
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|!v.layout.focused(v.repo.as_ref())));
}

#[gpui::test]
fn gui_leaving_focus_keeps_file_and_selection(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    let before=f.viewer.read_with(cx,|v,_|(v.comparison.current,v.repo.as_ref().unwrap().picker.anchor));
    enter_focus(cx,&f);
    f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::Layout(crate::layout::LayoutCommand::LeaveFocus),10.,w,cx)).unwrap();
    f.viewer.read_with(cx,|v,_|{assert!(!v.layout.focused(v.repo.as_ref()));assert_eq!((v.comparison.current,v.repo.as_ref().unwrap().picker.anchor),before);});
}

#[gpui::test]
fn gui_titles_name_the_selection(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|r.picker.refs.is_some()));
    let (window,header,sel)=f.viewer.read_with(cx,|v,_|(v.title(),v.log_frame().unwrap().status,v.repo.as_ref().unwrap().selection_title().unwrap().text));
    assert!(window.starts_with(&sel),"{window}");
    assert!(header.contains(&sel),"{header}");
}

#[gpui::test]
fn gui_escape_closes_the_change_set_search_before_leaving_focus(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    enter_focus(cx,&f);
    key(cx,&f,"f",true,true,false);
    for c in ["a","enter"] {key(cx,&f,c,false,false,false);}
    assert!(f.viewer.read_with(cx,|v,_|v.search.as_ref().is_some_and(|s|!s.editing)),"search results are up");
    // the results panel may hold the keyboard: Esc hands it back, then closes the search, never leaving Focus
    for _ in 0..2 {
        if f.viewer.read_with(cx,|v,_|v.search.is_none()) {break;}
        key(cx,&f,"escape",false,false,false);
        assert!(f.viewer.read_with(cx,|v,_|v.layout.focused(v.repo.as_ref())),"Esc with the search open stays in Focus");
    }
    assert!(f.viewer.read_with(cx,|v,_|v.search.is_none()),"the search closed");
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|!v.layout.focused(v.repo.as_ref())),"then Esc leaves Focus");
}

fn pr_fixture(cx:&mut TestAppContext)->Fixture{
    let f=fixture(cx,"old\n","new\n");
    f.viewer.update(cx,|v,_|{
        let mut pr:crate::pr::PullRequest=serde_json::from_str(include_str!("../../fixtures/pr/view.json")).unwrap();
        pr.body="Description line\n".repeat(80)+"DESCRIPTION_END";
        let mut state=crate::pr_session::PrState::new(pr,crate::pr::Fetched{number:14130,merge_base:"base".into(),head:"head".into()});
        state.marks=None;
        v.repo.get_or_insert_with(||crate::repository_session::RepositorySession::with_picker(Picker::new(f.dir.path().into(),false))).pr=Some(state);
    });
    toolbar_ids(cx,&f,1200.);
    f
}

fn pr_tab(cx:&mut TestAppContext,f:&Fixture)->Option<crate::pr_session::DetailsTab>{
    f.viewer.read_with(cx,|v,_|v.repo.as_ref().and_then(|r|r.pr.as_ref()).and_then(|p|p.details()).map(|d|d.tab))
}

#[gpui::test]
fn gui_pr_tabs_show_in_focus_without_a_modal(cx:&mut TestAppContext) {
    use crate::pr_session::DetailsTab;
    let f=pr_fixture(cx);
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)),w,cx)).unwrap();
    toolbar_ids(cx,&f,1200.);
    f.viewer.read_with(cx,|v,_|{
        assert!(v.layout.focused(v.repo.as_ref()),"details enter Focus");
        assert_eq!(v.modal(),None,"the tab is not a modal");
        let tabs:Vec<String>=v.toolbar_slots().into_iter().filter(|s|s.center).map(|s|s.label.into_owned()).collect();
        assert_eq!(tabs,["Conversation","Commits","Files changed"],"the PR's tabs are the toolbar's middle");
        assert!(v.header_switch_labels().is_empty(),"not in the diff header");
        assert!(!v.toolbar_ids().contains(&"pr-menu"),"tabs replace PR ▾ in Focus");
        assert!(!v.toolbar_ids().contains(&"diff-layout"),"diff controls hide on a details tab");
    });
    // the palette opens over the tab and Esc closes only the palette
    key(cx,&f,"f10",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()));
    key(cx,&f,"escape",false,false,false);
    assert_eq!(pr_tab(cx,&f),Some(DetailsTab::Conversation));
    // a toolbar tab switches tabs
    let tool=f.viewer.read_with(cx,|v,_|v.toolbar_slots().into_iter().find(|s|s.id=="pr-tab-commits").unwrap().tool);
    f.window.update(cx,|v,w,cx|v.toolbar_click(tool,600.,w,cx)).unwrap();
    assert_eq!(pr_tab(cx,&f),Some(DetailsTab::Commits));
    assert!(f.viewer.read_with(cx,|v,_|v.toolbar_slots().into_iter().any(|s|s.id=="pr-tab-commits"&&s.active)),"the open tab shows as current");
    // Esc returns to the diff and stays in Focus
    key(cx,&f,"escape",false,false,false);
    f.viewer.read_with(cx,|v,_|{assert!(!v.pr_details_shown());assert!(v.layout.focused(v.repo.as_ref()));});
    assert_eq!(text(&f),"new\n");
}

#[gpui::test]
fn gui_pr_reading_tabs_hide_files_and_leaving_focus_returns_to_the_diff(cx:&mut TestAppContext) {
    use crate::pr_session::DetailsTab;
    let f=pr_fixture(cx);
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();}
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)),w,cx)).unwrap();
    toolbar_ids(cx,&f,1200.);
    // a reading tab has no file list; Files changed brings it back
    assert_eq!(f.viewer.read_with(cx,|v,_|v.sidebar_w()),0.);
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Pr(crate::catalog::PrAction::Diff),w,cx)).unwrap();
    toolbar_ids(cx,&f,1200.);
    let (x,y)=nav_file_point(cx,&f);
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert_eq!(pr_tab(cx,&f),None,"choosing a file stays on the diff");
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Commits)),w,cx)).unwrap();
    f.window.update(cx,|v,w,cx|v.toolbar_click(Tool::Layout(crate::layout::LayoutCommand::LeaveFocus),10.,w,cx)).unwrap();
    f.viewer.read_with(cx,|v,_|{assert!(!v.layout.focused(v.repo.as_ref()));assert!(!v.pr_details_shown(),"the Overview shows the diff");});
}

fn open_description(cx:&mut TestAppContext,f:&Fixture){
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Pr(crate::catalog::PrAction::Details(crate::pr_session::DetailsTab::Conversation)),w,cx)).unwrap();
    toolbar_ids(cx,f,1200.);
}

#[gpui::test]
fn gui_pr_tab_lets_palette_rows_be_clicked_over_it(cx:&mut TestAppContext) {
    let f=pr_fixture(cx);
    open_description(cx,&f);
    key(cx,&f,"f10",false,false,false);
    for c in "overview".chars() {key(cx,&f,&c.to_string(),false,false,false);}
    let (x,y)=f.viewer.read_with(cx,|v,_|{let (px_,py,w,_)=v.palette_layout();(px_+w/2.,py+4.+1.5*line_h())});
    assert!(f.viewer.read_with(cx,|v,_|{let (_,py,_,ph)=v.pr_pane();y>=py&&y<py+ph}),"the row lies over the details");
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|!v.layout.focused(v.repo.as_ref())),"the palette row ran");
}

#[gpui::test]
fn gui_pr_tab_right_click_never_reaches_the_hidden_diff(cx:&mut TestAppContext) {
    let f=pr_fixture(cx);
    open_description(cx,&f);
    let (x,y)=f.viewer.read_with(cx,|v,_|{let (px_,py,w,_)=v.pr_pane();(px_+w*0.75,py+4.)});
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{button:MouseButton::Right,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    f.viewer.read_with(cx,|v,_|{assert!(v.menu.is_none(),"no menu for the hidden diff");assert!(v.sel.is_none_or(|s|s.is_empty()));});
    key(cx,&f,"f10",false,true,false);
    assert!(f.viewer.read_with(cx,|v,_|v.menu.is_none()),"Shift-F10 neither");
    assert_eq!(text(&f),"new\n");
}

#[gpui::test]
fn gui_pr_tab_keeps_diff_keys_from_files_focus_and_diff_actions_return_to_diff(cx:&mut TestAppContext) {
    let f=pr_fixture(cx);
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();}
    open_description(cx,&f);
    f.viewer.update(cx,|v,_|v.nav_focused=true);
    key(cx,&f,"i",false,false,false);
    assert!(!f.viewer.read_with(cx,|v,_|v.comparison.options().ignore_case),"Files focus: i still cannot reach the hidden diff");
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Comparison(Command::NextChange),w,cx)).unwrap();
    assert!(!f.viewer.read_with(cx,|v,_|v.pr_details_shown()),"a diff action shows the diff");
}

#[gpui::test]
fn gui_pr_header_switch_stays_right_of_files(cx:&mut TestAppContext) {
    let f=fixture_named(cx,"# old\n","# new\n","md");
    f.viewer.update(cx,|v,_|{
        let pr:crate::pr::PullRequest=serde_json::from_str(include_str!("../../fixtures/pr/view.json")).unwrap();
        v.repo.get_or_insert_with(||crate::repository_session::RepositorySession::with_picker(Picker::new(f.dir.path().into(),false))).pr=Some(crate::pr_session::PrState::new(pr,crate::pr::Fetched{number:14130,merge_base:"base".into(),head:"head".into()}));
    });
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();}
    enter_focus(cx,&f);
    workflow_wait(cx,&f,|v|v.doc().is_some());
    for width in [700.,900.,1200.] {
        toolbar_ids(cx,&f,width);
        f.viewer.read_with(cx,|v,_|{
            let cells=v.header_switch();
            assert!(cells.iter().all(|c|c.0>=v.diff_x()),"{width}: cells start right of Files");
            assert!(cells.iter().any(|c|c.2=="Source"),"{width}: Markdown modes stay; PR tabs give way first");
        });
    }
}

fn unified(cx:&mut TestAppContext,f:&Fixture,on:bool){
    let layout=if on {crate::commands::DiffLayout::Unified} else {crate::commands::DiffLayout::Split};
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Comparison(Command::SetDiffLayout(layout)),w,cx)).unwrap();
    toolbar_ids(cx,f,1200.);
}

#[gpui::test]
fn gui_unified_is_one_read_only_column(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\nb\nc\n","a\nB\nc\n");
    let bytes=std::fs::read(f.dir.path().join("right.txt")).ok();
    unified(cx,&f,true);
    f.viewer.read_with(cx,|v,_|{
        assert!(v.comparison.view.unified());
        assert_eq!(v.comparison_widths(1000.)[0],0.,"one full-width pane");
        let slots=v.toolbar_slots();
        assert!(slots.iter().any(|s|s.id=="diff-options"));
        assert!(!slots.iter().any(|s|s.id=="edit-undo"),"no edit controls in Unified");
    });
    // edits are refused, bytes stay
    key(cx,&f,"right",false,true,true);
    f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::Undo),w,cx)).unwrap();
    assert_eq!(text(&f),"a\nB\nc\n");
    assert_eq!(std::fs::read(f.dir.path().join("right.txt")).ok(),bytes);
    // a single click only places the caret
    let (x,y)=f.viewer.read_with(cx,|v,_|(v.diff_x()+GUTTER_W+80.,v.top()+0.5*line_h()));
    click(cx,&f,x,y);
    assert!(f.viewer.read_with(cx,|v,_|v.sel.is_none_or(|s|s.is_empty())));
    unified(cx,&f,false);
    assert!(f.viewer.read_with(cx,|v,_|!v.comparison.view.unified()&&v.comparison_widths(1000.)[0]>0.));
}

#[gpui::test]
fn gui_markdown_preview_falls_back_to_split(cx:&mut TestAppContext) {
    let f=fixture_named(cx,"# old\n","# new\n","md");
    workflow_wait(cx,&f,|v|v.doc().is_some());
    unified(cx,&f,true);
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.view.unified()));
    run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
    markdown_ready(cx,&f);
    f.viewer.read_with(cx,|v,_|{assert!(!v.comparison.view.unified(),"the Preview has its own layout");assert!(v.unified_disabled().is_some());});
}

#[gpui::test]
fn gui_unified_selects_and_copies_either_side(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\nbeta\nc\n","a\nBETA\nc\n");
    unified(cx,&f,true);
    // rows: a, beta (deleted, left), BETA (inserted, right), c
    let at=|cx:&mut TestAppContext,row:f32|f.viewer.read_with(cx,|v,_|{let x=v.diff_x()+GUTTER_W+num_w()+8.+v.char_w*1.5;(x,v.top()+(row+0.5)*line_h())});
    for (row,side,word) in [(1.,Side::Left,"beta"),(2.,Side::Right,"BETA")] {
        let (x,y)=at(cx,row);
        f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:2,first_mouse:false},w,cx)).unwrap();
        f.window.update(cx,|v,w,cx|v.on_mouse_up(&MouseUpEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:2},w,cx)).unwrap();
        assert_eq!(f.viewer.read_with(cx,|v,_|v.sel.map(|s|s.side)),Some(side),"row {row}");
        key(cx,&f,"c",true,false,false);
        assert_eq!(cx.update(|cx|cx.read_from_clipboard().unwrap().text().unwrap()),word);
    }
    assert_eq!(text(&f),"a\nBETA\nc\n","copying changes nothing");
}

#[gpui::test]
fn gui_focus_names_the_selection_above_files(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|r.picker.refs.is_some()));
    let title=f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().selection_title().unwrap().text);
    assert!(f.viewer.read_with(cx,|v,_|v.nav_header()).ends_with("changed files"),"the Overview counts files");
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();}
    enter_focus(cx,&f);
    f.viewer.read_with(cx,|v,_|{assert_eq!(v.nav_header(),title,"Focus names the selection above Files");assert!(v.focus_title().is_none(),"not again in the toolbar");});
    f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.focus_title().map(|t|t.text)),Some(title),"Files hidden: the toolbar keeps it");
}

#[gpui::test]
fn gui_focus_and_overview_buttons_match_and_back_forward_stay(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    let width=|cx:&mut TestAppContext,id:&str|{toolbar_ids(cx,&f,1200.);f.viewer.read_with(cx,|v,_|v.toolbar_slots().into_iter().find(|s|s.id==id).and_then(|s|s.width))};
    let focus=width(cx,"focus");
    assert_eq!(toolbar_ids(cx,&f,1200.)[..4],["app-menu","focus","nav-back","nav-forward"]);
    enter_focus(cx,&f);
    assert_eq!(toolbar_ids(cx,&f,1200.)[..4],["app-menu","overview","nav-back","nav-forward"],"Back and Forward in Focus too");
    let overview=width(cx,"overview");
    assert!(focus.is_some()&&focus==overview,"Focus and Overview are one size, so ◀ stays put: {focus:?} {overview:?}");
}

#[gpui::test]
fn gui_panel_switches_sit_glued_at_the_bottom(cx:&mut TestAppContext) {
    use crate::layout::LayoutCommand as L;
    let (f,_repo)=repository_fixture(cx);
    let ids=toolbar_ids(cx,&f,1200.);
    assert!(!ids.iter().any(|id|id.starts_with("panel-")),"panels toggle at the bottom: {ids:?}");
    assert_eq!(ids.last(),Some(&"help"),"? at the top right");
    let cells=f.viewer.read_with(cx,|v,_|v.panel_switch());
    assert_eq!(cells.iter().map(|c|c.label).collect::<Vec<_>>(),["Branches","History","Files"]);
    assert!(cells.windows(2).all(|w|(w[0].x+w[0].w-w[1].x).abs()<0.5),"glued together");
    let (bottom,history)=f.viewer.read_with(cx,|v,_|(v.panes.viewport[1],v.layout.panels(v.repo.as_ref()).history));
    assert!(cells.iter().all(|c|c.y>=bottom),"below the content");
    let at=&cells[1];
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(at.x+at.w/2.),px(at.y+footer_h()/2.)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.layout.panels(v.repo.as_ref()).history),!history,"a click toggles History");
    enter_focus(cx,&f);
    let ids=toolbar_ids(cx,&f,1200.);
    assert!(!ids.contains(&"unchanged")&&ids.last()==Some(&"help"),"{ids:?}");
    let cells=f.viewer.read_with(cx,|v,_|v.panel_switch());
    assert_eq!(cells.iter().map(|c|(c.label,c.enabled)).collect::<Vec<_>>(),[("Branches",false),("History",false),("Files",true),("Commit",true)],"Focus refuses Branches and History");
    assert_eq!(cells[2].command,crate::catalog::Action::Layout(L::ToggleFiles));
}

#[gpui::test]
fn gui_right_click_with_nothing_to_offer_opens_no_menu(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    toolbar_ids(cx,&f,1200.);
    // the empty part of the Branches sidebar has no actions
    let (x,y)=f.viewer.read_with(cx,|v,_|(20.,v.panes.viewport[1]-20.));
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{button:MouseButton::Right,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    f.viewer.read_with(cx,|v,_|{assert!(v.menu.is_none(),"no empty popup");assert_eq!(v.modal(),None);});
}

#[gpui::test]
fn gui_theme_picker_previews_as_you_move(cx:&mut TestAppContext) {
    let f=fixture(cx,"a\n","b\n");
    let render=|cx:&mut TestAppContext|f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);v.palette.clone()}).unwrap();
    let original=render(cx);
    f.window.update(cx,|v,w,cx|v.tool(Tool::Theme,0.,w,cx)).unwrap();
    // move until a theme that looks different is highlighted
    let mut shown=original.clone();
    for _ in 0..40 {
        key(cx,&f,"down",false,false,false);
        shown=render(cx);
        if *shown!=*original {break;}
    }
    assert!(*shown!=*original,"moving through the picker shows each theme");
    let expected=f.viewer.read_with(cx,|v,cx|{let c=v.theme_chooser.as_ref().unwrap();cx.global::<themes::GuiThemes>().0.preview(&c.entries[c.selected].id).unwrap()});
    assert_eq!(*shown,*expected);
    key(cx,&f,"escape",false,false,false);
    assert_eq!(*render(cx),*original,"Esc puts the theme back");
}

#[gpui::test]
fn gui_branch_menu_compares_with_current(cx:&mut TestAppContext) {
    let (f,repo)=repository_fixture(cx);
    crate::git::testing::git(repo.path(),&["branch","side","HEAD~1"]);
    f.viewer.update(cx,|v,cx|v.refresh_repository(cx));
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()&&r.picker.sources.branches.len()==2));
    toolbar_ids(cx,&f,1200.);
    // the sidebar row naming `side`
    let row=|cx:&mut TestAppContext|f.viewer.read_with(cx,|v,_|{
        let p=&v.repo.as_ref().unwrap().picker;
        let rows=p.sources.rows(&v.palette,&p.branches);
        let i=rows.iter().position(|r|matches!(&r.action,Some(crate::repository_session::SourceAction::Branch(name,_)) if name.ends_with("side"))).expect("side listed");
        (20.,toolbar_h()+(i as f32+1.5-p.sources.scroll)*line_h())
    });
    // a click on the branch shows its log; the menu only adds the comparison
    let (x,y)=row(cx);
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{button:MouseButton::Right,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert_eq!(f.viewer.read_with(cx,|v,_|v.menu.as_ref().map(|m|m.items.iter().map(|i|i.0.clone()).collect::<Vec<_>>())),Some(vec!["Compare with Current".to_string()]));
    key(cx,&f,"escape",false,false,false);
    let (n,label)=(0,"Compare with Current");
    let (x,y)=row(cx);
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{button:MouseButton::Right,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    let items=f.viewer.read_with(cx,|v,_|v.menu.as_ref().map(|m|m.items.iter().map(|i|i.0.clone()).collect::<Vec<_>>()));
    assert_eq!(items.as_ref().and_then(|i|i.get(n)).map(String::as_str),Some(label),"{items:?}");
    // click the entry, as a person would
    let (mx,my)=f.viewer.read_with(cx,|v,_|{let m=v.menu.as_ref().unwrap();let (x,y,_,_)=m.layout(v.panes.viewport,v.char_w);(x+30.,y+4.+(n as f32+0.5)*line_h())});
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(mx),px(my)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.menu.is_none()),"the click chose an entry");
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()));
    f.viewer.read_with(cx,|v,_|{
        let p=&v.repo.as_ref().unwrap().picker;
        assert!(p.compare.as_ref().is_some_and(|(b,_)|b.ends_with("side")),"compares with side: {:?}",p.compare);
        assert!(matches!(&v.spec,crate::changeset::Spec::Git{revs,..} if revs==&vec!["refs/heads/side...HEAD".to_string()]),"the diff shows what this branch adds: {:?}",v.spec);
    });
}


#[gpui::test]
fn gui_menu_entries_over_a_pane_divider_still_run(cx:&mut TestAppContext) {
    let (f,repo)=repository_fixture(cx);
    crate::git::testing::git(repo.path(),&["branch","side","HEAD~1"]);
    f.viewer.update(cx,|v,cx|v.refresh_repository(cx));
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()&&r.picker.sources.branches.len()==2));
    toolbar_ids(cx,&f,1200.);
    let y=f.viewer.read_with(cx,|v,_|{
        let p=&v.repo.as_ref().unwrap().picker;
        let i=p.sources.rows(&v.palette,&p.branches).iter().position(|r|matches!(&r.action,Some(crate::repository_session::SourceAction::Branch(n,_)) if n.ends_with("side"))).unwrap();
        toolbar_ids_y(v,i)
    });
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{button:MouseButton::Right,position:point(px(150.),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    // the first entry, clicked right where the sidebar's divider runs under the menu
    let (mx,my)=f.viewer.read_with(cx,|v,_|{let m=v.menu.as_ref().unwrap();let (_,my,_,_)=m.layout(v.panes.viewport,v.char_w);(v.picker_w(),my+4.+0.5*line_h())});
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(mx),px(my)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()));
    f.viewer.read_with(cx,|v,_|{assert!(v.menu.is_none());assert!(v.panes.drag.is_none(),"no resize starts under a menu");assert!(v.repo.as_ref().unwrap().picker.compare.is_some(),"Compare with Current ran");});
}

fn toolbar_ids_y(v:&Viewer,row:usize)->f32{toolbar_h()+(row as f32+1.5-v.repo.as_ref().unwrap().picker.sources.scroll)*line_h()}

#[gpui::test]
fn gui_palette_keys_work_while_a_list_searches(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    toolbar_ids(cx,&f,1200.);
    // History has the keyboard: letters and ':' are search text there
    f.viewer.update(cx,|v,_|{v.nav_focused=false;if let Some(r)=v.repo.as_mut(){r.picker.focus=Pane::Log;}});
    key(cx,&f,":",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_none()),"':' searches the list");
    key(cx,&f,"f",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|!v.layout.focused(v.repo.as_ref())),"'f' is search text in a list, not Focus");
    for (k,ctrl,shift) in [("f10",false,false),("p",true,true)] {
        key(cx,&f,"escape",false,false,false);
        f.viewer.update(cx,|v,_|{v.command_palette=None;if let Some(r)=v.repo.as_mut(){r.picker.focus=Pane::Log;}});
        key(cx,&f,k,ctrl,shift,false);
        assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()),"{k} opens the palette from a list");
    }
    // Files has the keyboard
    f.viewer.update(cx,|v,_|{v.command_palette=None;v.nav_focused=true;});
    key(cx,&f,"f10",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()),"F10 from Files");
}

#[gpui::test]
fn gui_bottom_bar_names_the_palette_key_and_opens_it(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    toolbar_ids(cx,&f,1200.);
    let (x,w,label)=f.viewer.read_with(cx,|v,_|v.palette_hint());
    assert!(label.contains(if cfg!(target_os="macos") {"Cmd+Shift+P"} else {"Ctrl+Shift+P"}),"{label}");
    assert!(x+w<=1200.&&x>600.,"at the right end: {x} {w}");
    let y=f.viewer.read_with(cx,|v,_|v.panes.viewport[1]+footer_h()/2.);
    f.window.update(cx,|v,w_,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x+w/2.),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w_,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()),"a click opens the palette");
}

#[gpui::test]
fn gui_a_click_outside_closes_any_popup(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    toolbar_ids(cx,&f,1200.);
    let outside=|cx:&mut TestAppContext|{let (x,y)=f.viewer.read_with(cx,|v,_|(v.panes.viewport[0]-20.,v.panes.viewport[1]-20.));
        f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();};
    type Open=Box<dyn Fn(&mut Viewer,&mut Window,&mut Context<Viewer>)>;
    type Case=(&'static str,Open,fn(&Viewer)->bool);
    let mut open_cases:Vec<Case>=vec![
        ("menu",Box::new(|v,w,cx|v.tool(Tool::AppMenu,10.,w,cx)),|v|v.menu.is_some()),
        ("palette",Box::new(|v,_,cx|v.open_palette(cx)),|v|v.command_palette.is_some()),
        ("diff options",Box::new(|v,w,cx|v.tool(Tool::DiffOptions,300.,w,cx)),|v|v.diff_options.is_some()),
        ("help",Box::new(|v,w,cx|v.tool(Tool::Help,0.,w,cx)),|v|v.help),
        ("theme picker",Box::new(|v,w,cx|v.tool(Tool::Theme,0.,w,cx)),|v|v.theme_chooser.is_some()),
        ("about",Box::new(|v,w,cx|v.tool(Tool::About,0.,w,cx)),|v|v.about.is_some()),
        ("go to file",Box::new(|v,_,_|v.switcher=Some(Switcher{query:String::new(),selected:0,hits:Vec::new()})),|v|v.switcher.is_some()),
        ("go to line",Box::new(|v,_,_|v.goto=Some(String::new())),|v|v.goto.is_some()),
    ];
    let mut stuck=Vec::new();
    for (name,open,shown) in open_cases.drain(..) {
        f.window.update(cx,|v,w,cx|open(v,w,cx)).unwrap();
        assert!(f.viewer.read_with(cx,|v,_|shown(v)),"{name} opened");
        outside(cx);
        if f.viewer.read_with(cx,|v,_|shown(v)) {stuck.push(name);}
        f.viewer.update(cx,|v,_|{v.menu=None;v.command_palette=None;v.diff_options=None;v.help=false;v.theme_chooser=None;v.switcher=None;v.goto=None;});
        f.window.update(cx,|v,w,_|{if v.about.is_some(){v.close_about(w);}}).unwrap();
    }
    assert!(stuck.is_empty(),"a click outside leaves these open: {stuck:?}");
    // a click inside the box keeps it open
    f.viewer.update(cx,|v,_|v.goto=Some("12".into()));
    let (x,y)=f.viewer.read_with(cx,|v,_|{let prompt=v.goto_prompt().unwrap();let w=prompt.chars().count() as f32*7.8+24.;let x0=v.diff_x();(x0+(v.panes.viewport[0]-x0-w)/2.+w-6.,toolbar_h()+v.log_h()+66.)});
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.goto.is_some()),"inside the box: still open");
}

#[gpui::test]
fn gui_a_click_outside_closes_find_and_find_in_files(cx:&mut TestAppContext) {
    let f=fixture(cx,"alpha\nbeta\n","alpha\nBETA\n");
    toolbar_ids(cx,&f,1200.);
    let click_at=|cx:&mut TestAppContext,x:f32,y:f32|f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    let far=f.viewer.read_with(cx,|v,_|(v.diff_x()+40.,v.panes.viewport[1]-30.));
    // Ctrl+F: a click on the bar keeps it, a click elsewhere closes it
    key(cx,&f,"f",true,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.find.is_some()));
    let (bx,by,bw,_)=f.viewer.read_with(cx,|v,_|v.find_bar_box().unwrap());
    click_at(cx,bx+bw/2.,by+4.);
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.find.is_some()),"a click on the bar keeps it");
    click_at(cx,far.0,far.1);
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.find.is_none()),"a click outside closes Find");
    // Ctrl+Shift+F while typing the query
    key(cx,&f,"f",true,true,false);
    assert!(f.viewer.read_with(cx,|v,_|v.search.as_ref().is_some_and(|s|s.editing)));
    click_at(cx,far.0,far.1);
    assert!(f.viewer.read_with(cx,|v,_|v.search.is_none()),"a click outside closes Find in files");
}

#[gpui::test]
fn gui_a_click_on_history_closes_find(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    toolbar_ids(cx,&f,1200.);
    f.viewer.update(cx,|v,_|{v.nav_focused=false;if let Some(r)=v.repo.as_mut(){r.picker.focus=Pane::Diff;}});
    key(cx,&f,"f",true,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.find.is_some()));
    let (x,y)=f.viewer.read_with(cx,|v,_|(v.picker_w()+200.,toolbar_h()+line_h()*2.5));
    f.window.update(cx,|v,w,cx|v.on_mouse_down(&MouseDownEvent{button:MouseButton::Left,position:point(px(x),px(y)),modifiers:Default::default(),click_count:1,first_mouse:false},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.find.is_none()),"a click on History closes Find");
}

/// What a key or a command changed, for comparing the two.
fn fingerprint(v:&Viewer)->String{
    let r=v.repo.as_ref();
    format!("{:?}",(
        (v.comparison.options,v.comparison.collapse,v.comparison.view.mode(),v.comparison.view.current_change(),v.comparison.view.unified(),v.comparison.current),
        (v.sel,v.help,v.command_palette.is_some(),v.menu.as_ref().map(|m|m.items.iter().map(|i|i.0.clone()).collect::<Vec<_>>()),v.diff_options.is_some()),
        (v.layout.panels(r),v.layout.focused(r),v.comparison.find.is_some(),v.search.as_ref().map(|s|s.editing),v.goto.is_some(),v.switcher.is_some(),v.blame_on),
        (v.edits.dirty(v.comparison.current),v.doc().map(|d|d.text_right.to_string())),
        r.map(|r|(r.picker.branches.clone(),r.picker.compare.clone(),r.picker.anchor,r.picker.filter.clone(),r.pr.as_ref().and_then(|p|p.details().map(|d|d.tab)))),
    ))
}

fn press(cx:&mut TestAppContext,f:&Fixture,shortcut:&str){
    let parts:Vec<&str>=shortcut.split('-').collect();
    let (mods,key_name)=parts.split_at(parts.len()-1);
    let name=match key_name[0] {""=>"-",k=>k};
    key(cx,f,name,mods.contains(&"ctrl"),mods.contains(&"shift"),mods.contains(&"alt"));
}

fn check_shortcuts(cx:&mut TestAppContext,make:&dyn Fn(&mut TestAppContext)->Fixture,wrong:&mut Vec<String>){
    let probe=make(cx);
    let entries:Vec<crate::catalog::Entry>=probe.viewer.read_with(cx,|v,_|crate::catalog::entries(&v.scope())).into_iter().filter(|e|e.shortcut.is_some()).collect();
    for e in entries {
        let shortcut=e.shortcut.unwrap();
        let settle=|cx:&mut TestAppContext,f:&Fixture|{for _ in 0..20 {cx.executor().advance_clock(std::time::Duration::from_millis(20));cx.executor().run_until_parked();}let _=f;};
        let prep=|cx:&mut TestAppContext,f:&Fixture|{toolbar_ids(cx,f,1200.);f.viewer.update(cx,|v,_|{v.nav_focused=false;v.panel_focused=false;if let Some(r)=v.repo.as_mut(){r.picker.focus=Pane::Diff;}});};
        let by_key=make(cx);prep(cx,&by_key);press(cx,&by_key,shortcut);settle(cx,&by_key);
        let by_cmd=make(cx);prep(cx,&by_cmd);by_cmd.window.update(cx,|v,w,cx|v.run_action(e.action,w,cx)).unwrap();settle(cx,&by_cmd);
        let (k,c)=(by_key.viewer.read_with(cx,|v,_|fingerprint(v)),by_cmd.viewer.read_with(cx,|v,_|fingerprint(v)));
        let untouched=probe.viewer.read_with(cx,|v,_|fingerprint(v));
        if k!=c {wrong.push(format!("{shortcut} for {:?} ({}) — key changed {}",e.action,e.label,if k==untouched {"nothing"} else {"something else"}));}
    }
}

#[gpui::test]
fn gui_palette_shortcuts_do_what_they_say(cx:&mut TestAppContext) {
    let mut wrong=Vec::new();
    check_shortcuts(cx,&|cx|fixture(cx,"a\nold one\nb\nc\nd\ne\nf\ng\nh\ni\nj\nold two\nk\n","a\nnew one\nb\nc\nd\ne\nf\ng\nh\ni\nj\nnew two\nk\n"),&mut wrong);
    check_shortcuts(cx,&|cx|repository_fixture(cx).0,&mut wrong);
    check_shortcuts(cx,&|cx|pr_fixture(cx),&mut wrong);
    check_shortcuts(cx,&|cx|{let f=repository_fixture(cx).0;enter_focus(cx,&f);f},&mut wrong);
    check_shortcuts(cx,&|cx|{let f=pr_fixture(cx);enter_focus(cx,&f);f},&mut wrong);
    assert!(wrong.is_empty(),"shortcuts that do not match their entry:\n{}",wrong.join("\n"));
}

#[gpui::test]
fn gui_file_keys_work_while_files_has_the_keyboard(cx:&mut TestAppContext) {
    let f=files_fixture(cx);
    toolbar_ids(cx,&f,1200.);
    let shown=f.viewer.read_with(cx,|v,_|crate::catalog::entries(&v.scope()).into_iter().filter(|e|matches!(e.action,crate::catalog::Action::Comparison(Command::NextFile|Command::PreviousFile))).map(|e|e.shortcut).collect::<Vec<_>>());
    assert_eq!(shown,[Some("ctrl-["),Some("ctrl-]")],"the palette names the keys that work everywhere");
    f.viewer.update(cx,|v,_|v.nav_focused=true);
    let before=f.viewer.read_with(cx,|v,_|v.comparison.current);
    key(cx,&f,"]",true,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.current),before+1,"Ctrl+] from Files");
    key(cx,&f,"[",true,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.current),before,"Ctrl+[ from Files");
}

#[gpui::test]
fn gui_file_keys_work_while_history_or_branches_has_the_keyboard(cx:&mut TestAppContext) {
    let (f,_repo)=repository_fixture(cx);
    toolbar_ids(cx,&f,1200.);
    assert!(f.viewer.read_with(cx,|v,_|v.set.comparisons.len())>=2,"two files to step between");
    for pane in [Pane::Log,Pane::Sidebar] {
        f.viewer.update(cx,|v,_|{v.nav_focused=false;if let Some(r)=v.repo.as_mut(){r.picker.focus=pane;}});
        let before=f.viewer.read_with(cx,|v,_|v.comparison.current);
        key(cx,&f,if before==0 {"]"} else {"["},true,false,false);
        assert_ne!(f.viewer.read_with(cx,|v,_|v.comparison.current),before,"Ctrl+[ / Ctrl+] from {pane:?}");
    }
}

fn drag(cx:&mut TestAppContext,f:&Fixture,from:(f32,f32),to:(f32,f32)){
    click(cx,f,from.0,from.1);
    f.window.update(cx,|v,w,cx|{
        if from!=to {v.on_mouse_move(&MouseMoveEvent{position:point(px(to.0),px(to.1)),pressed_button:Some(MouseButton::Left),modifiers:Default::default()},w,cx);}
        v.on_mouse_up(&MouseUpEvent{button:MouseButton::Left,position:point(px(to.0),px(to.1)),modifiers:Default::default(),click_count:1},w,cx);
    }).unwrap();
    cx.executor().run_until_parked();
}

#[gpui::test]
fn gui_focus_shows_the_commit_message_below_files_and_diff(cx:&mut TestAppContext) {
        let (f,repo)=repository_fixture(cx);
    if !f.viewer.read_with(cx,|v,_|v.layout.files()){f.window.update(cx,|v,w,cx|v.tool(Tool::Layout(crate::layout::LayoutCommand::ToggleFiles),0.,w,cx)).unwrap();}
    assert!(f.viewer.read_with(cx,|v,_|v.message_lines().is_empty()),"the Overview has no panel");
    enter_focus(cx,&f);
    workflow_wait(cx,&f,|v|v.message_lines().first().is_some_and(|l|l.text=="▾ second"));
    // the test window shapes no text: give the panel a character width to place columns with
    f.viewer.update(cx,|v,_|v.char_w=8.);
    let (x,top,nav_top,char_w)=f.viewer.read_with(cx,|v,_|(v.diff_x()+20.,v.panes.viewport[1]-v.message_h(),v.nav_top(),v.char_w));
    assert!(top>nav_top+line_h(),"the panel sits below the files and diff");
    assert_eq!(nav_top,f.viewer.read_with(cx,|v,_|v.top()),"Files start directly below their header");
    // a file row is still where it is drawn
    let (row,index)=f.viewer.read_with(cx,|v,_|v.nav.iter().enumerate().filter_map(|(i,r)|match r{NavRow::File{index,..}=>Some((i,*index)),_=>None}).next_back().unwrap());
    let scroll=f.viewer.read_with(cx,|v,_|v.nav_scroll);
    click(cx,&f,20.,nav_top+(row as f32-scroll+0.5)*line_h());
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.current),index);
    // right click: copy what is under the pointer
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(top+line_h()*0.5)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    let labels=f.viewer.read_with(cx,|v,_|v.menu.as_ref().map_or_else(Vec::new,|m|m.items.iter().map(|(l,_)|l.clone()).collect::<Vec<_>>()));
    assert_eq!(labels,["Copy commit message","Copy commit hash"]);
    key(cx,&f,"escape",false,false,false);
    // with the menu open, a click on the panel only closes the menu
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(top+line_h()*1.5)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.menu.is_some()));
    drag(cx,&f,(x,top+line_h()*0.5),(x,top+line_h()*0.5));
    f.viewer.read_with(cx,|v,_|{assert!(v.menu.is_none());assert_eq!(v.message_lines()[0].text,"▾ second");});
    // a drag selects text without collapsing, and Ctrl/Cmd+C copies it
    let text_x=f.viewer.read_with(cx,|v,_|v.nav_x()+8.);
    drag(cx,&f,(text_x+2.*char_w,top+line_h()*0.5),(text_x+6.*char_w,top+line_h()*0.5));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.message_lines()[0].text.clone()),"▾ second");
    key(cx,&f,"c",true,false,false);
    assert_eq!(cx.read_from_clipboard().and_then(|c|c.text()).as_deref(),Some("seco"));
    // a click on its first line collapses it, and the palette brings it back
    drag(cx,&f,(x,top+line_h()*0.5),(x,top+line_h()*0.5));
    f.viewer.read_with(cx,|v,_|{assert!(v.message_lines().is_empty());assert_eq!(v.message_h(),0.);});
    let toggle=f.viewer.read_with(cx,|v,_|v.panel_switch().into_iter().find(|c|c.label=="Commit").expect("Commit switch remains available"));
    click(cx,&f,toggle.x+toggle.w/2.,toggle.y+footer_h()/2.);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.message_lines()[0].text.clone()),"▾ second");

    // Hiding Files does not hide the panel; hiding the panel gives the diff its height back.
    let rows=f.viewer.read_with(cx,|v,_|v.comparison.view.viewport_rows());
    run(cx,&f,crate::catalog::Action::Layout(crate::layout::LayoutCommand::ToggleFiles));
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    assert!(!f.viewer.read_with(cx,|v,_|v.message_lines().is_empty()));
    run(cx,&f,crate::catalog::Action::Repository(crate::catalog::RepoAction::ToggleMessage));
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.view.viewport_rows())>rows);
    run(cx,&f,crate::catalog::Action::Repository(crate::catalog::RepoAction::ToggleMessage));
    run(cx,&f,crate::catalog::Action::Layout(crate::layout::LayoutCommand::ToggleFiles));

    // a range: ↓ from the last file enters the panel, ↓ walks the commits, Enter opens one, Esc gives Files back
    use crate::git::testing::{commit_all,write};
    write(repo.path(),"a.txt","3\n");commit_all(repo.path(),"third");
    let spec=crate::repository_session::branch_comparison("HEAD~2","HEAD",false);
    let set=crate::changeset::ChangeSet::load(&spec,repo.path()).unwrap();
    f.viewer.update(cx,|v,_|v.repo.as_mut().unwrap().showing(Arc::new(set),spec));
    workflow_wait(cx,&f,|v|v.message_lines().first().is_some_and(|l|l.text=="▾ 2 commits"));
    f.window.update(cx,|v,w,cx|{v.nav_focused=true;v.open(0,Place::Top,cx);let _=w;}).unwrap();
    key(cx,&f,"end",false,false,false);
    key(cx,&f,"down",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.message_focused&&!v.nav_focused));
    key(cx,&f,"down",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().message_cursor()),1);
    key(cx,&f,"enter",false,false,false);
    f.viewer.read_with(cx,|v,_|assert!(v.message_lines().iter().any(|l|l.text.trim()=="body"||l.kind==crate::commit_message::LineKind::Meta),"{:?}",v.message_lines()));
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|!v.message_focused&&v.nav_focused));
    // The commit panel keeps its keys while Folder Summary is open above it.
    run(cx,&f,crate::catalog::Action::Folder(crate::folders::FolderAction::Summary));
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    let (mx,my)=f.viewer.read_with(cx,|v,_|{
        let row=v.message_lines().iter().position(|l|l.kind==crate::commit_message::LineKind::Meta).unwrap();
        (v.nav_x()+12.,v.content_bottom()+1.+(row as f32+0.5)*line_h())
    });
    drag(cx,&f,(mx,my),(mx,my));
    assert!(f.viewer.read_with(cx,|v,_|v.message_focused));
    key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|!v.message_focused&&v.comparison.folders.summary().is_some()));
    let bytes=std::fs::read(repo.path().join("a.txt")).unwrap();
    // Current geometry clamps stale scroll after resize; the bottom panel never owns a diff divider.
    for (width,height) in [(800.,600.),(400.,240.),(120.,90.),(1.,1.),(1200.,800.)] {
        cx.simulate_window_resize(f.window.into(),size(px(width),px(height)));
        f.window.update(cx,|v,w,cx|{v.message_scroll=10000.;let _=v.render(w,cx);
            assert!(v.content_bottom()>=0. && v.message_h()<=v.panes.viewport[1]);
            assert!(v.message_scroll<=v.message_lines().len() as f32);
            if v.message_h()>0. {assert!(!v.begin_resize(v.diff_x()+v.comparison_widths(width-v.diff_x())[0],v.content_bottom()+line_h()/2.));}
        }).unwrap();
    }
    // Narrow Focus keeps the active panel switches clear of the Commands hint.
    cx.simulate_window_resize(f.window.into(),size(px(400.),px(240.)));
    f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    let toggle=f.viewer.read_with(cx,|v,_|{
        let c=v.panel_switch().into_iter().find(|c|c.label=="Commit").expect("visible narrow toggle");
        assert!(c.x+c.w<=v.palette_hint().0);c
    });
    click(cx,&f,toggle.x+toggle.w/2.,toggle.y+footer_h()/2.);
    assert!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().message_collapsed()&&v.command_palette.is_none()));
    click(cx,&f,toggle.x+toggle.w/2.,toggle.y+footer_h()/2.);
    assert!(!f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().message_collapsed()));
    // A modal consumes the footer toggle, preserving both the panel state and file bytes.
    let toggle=f.viewer.read_with(cx,|v,_|v.panel_switch().into_iter().find(|c|c.label=="Commit").unwrap());
    let hidden=f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().message_collapsed());
    f.viewer.update(cx,|v,_|v.help=true);
    click(cx,&f,toggle.x+toggle.w/2.,toggle.y+footer_h()/2.);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().message_collapsed()),hidden);
    assert_eq!(std::fs::read(repo.path().join("a.txt")).unwrap(),bytes);
}

#[gpui::test]
fn gui_pr_conversation_clicks_fold_jump_open_commits_and_copy(cx:&mut TestAppContext) {
    use crate::pr_session::DetailsTab;
    use crate::rich::Target;
    let f=pr_fixture(cx);
    let path=f.viewer.read_with(cx,|v,_|v.set.comparisons[0].path.clone());
    f.viewer.update(cx,|v,_|{
        v.char_w=8.;
        let thread=crate::pr::Thread{path:path.clone(),diff_side:Side::Right,line:Some(1),start_line:None,original_line:Some(1),is_outdated:false,is_resolved:true,
            comments:vec![crate::pr::Comment{author:Some(crate::pr::Account{login:"bob".into()}),body:"THREAD_BODY".into(),created_at:"2026-10-05T00:00:00Z".into(),url:"https://x/c1".into(),..Default::default()}],subject_type:String::new()};
        let mut review=crate::pr::Review{placed:[((path.clone(),Side::Right,1),vec![thread])].into(),..Default::default()};
        review.overview.commits=vec![crate::pr::PrCommit{sha:"abc1234567".into(),message:"Commit subject\n\nCommit body".into(),author:"octocat".into(),date:"2026-10-05T00:00:00Z".into()}];
        review.overview.commit_total=1;
        let pr=v.repo.as_mut().unwrap().pr.as_mut().unwrap();pr.seed_review(review,&[]).unwrap();
    });
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)));
    let page=|cx:&mut TestAppContext|{f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().page.clone())};
    // the window point of a page target: its row, at its first column
    let point_of=|cx:&mut TestAppContext,target:&Target|f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);
        let rect=v.pr_pane();let char_w=v.char_w;
        let d=v.pr_dialog.as_mut().unwrap();let row=d.page.row_of(target).unwrap();let r=&d.page.rows[row];
        let mut col=r.indent;for s in &r.spans{if s.target.as_ref()==Some(target){break;}col+=crate::rich::width(&s.text);}
        let (_,y)=d.point_of(row,col,rect,char_w);
        if y<rect.1||y+line_h()>rect.1+rect.3 {d.scroll=row.saturating_sub(2).min(d.max_scroll(rect));}
        let (x,y)=d.point_of(row,col,rect,char_w);(x+1.,y+2.)
    }).unwrap();
    let texts=|cx:&mut TestAppContext|page(cx).rows.iter().map(|r|r.text()).collect::<Vec<_>>().join("\n");
    assert!(!texts(cx).contains("THREAD_BODY"),"a resolved thread starts folded");
    let fold=Target::Toggle(crate::pr_conversation::thread_key(&f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().pr.as_ref().unwrap().review.as_ref().unwrap().placed.values().next().unwrap()[0].clone())));
    let (x,y)=point_of(cx,&fold);click(cx,&f,x,y);
    assert!(texts(cx).contains("THREAD_BODY"),"a click unfolds it");
    // right click on the card: copy its comment or link
    let (x,y)=point_of(cx,&fold);
    f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
    let labels=f.viewer.read_with(cx,|v,_|v.menu.as_ref().map_or_else(Vec::new,|m|m.items.iter().map(|(l,_)|l.clone()).collect::<Vec<_>>()));
    assert_eq!(labels,["Copy comment","Copy link to comment"]);
    key(cx,&f,"escape",false,false,false);
    // path:line jumps to Files changed at that line
    let (x,y)=point_of(cx,&Target::Line{path:path.clone(),line:1,left:false});click(cx,&f,x,y);
    assert_eq!(pr_tab(cx,&f),None,"Files changed");
    // Commits: a commit's hash opens Files changed for it alone
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Commits)));
    assert!(texts(cx).contains("Commit subject"));
    let (x,y)=point_of(cx,&Target::Commit(Some("abc1234567".into())));click(cx,&f,x,y);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().pr.as_ref().unwrap().shown_commit.clone()),Some("abc1234567".into()));
    assert_eq!(pr_tab(cx,&f),None,"the commit's files show");
    assert_eq!(text(&f),"new\n");
}

#[gpui::test]
fn gui_pr_tabs_copy_text_hide_files_keep_their_place_and_sit_in_the_middle(cx:&mut TestAppContext) {
    use crate::pr_session::DetailsTab;
    let f=pr_fixture(cx);
    f.viewer.update(cx,|v,_|{
        v.char_w=8.;
        let mut review=crate::pr::Review::default();
        review.overview.commits=(0..80).map(|i|crate::pr::PrCommit{sha:format!("{i:07}abc"),message:format!("Commit {i}"),author:"octocat".into(),date:"2026-10-05T00:00:00Z".into()}).collect();
        review.overview.commit_total=80;
        v.repo.as_mut().unwrap().pr.as_mut().unwrap().seed_review(review,&[]).unwrap();
        if !v.layout.files(){v.layout.execute(crate::layout::LayoutCommand::ToggleFiles,v.repo.as_mut());}
    });
    let paint=|cx:&mut TestAppContext|f.window.update(cx,|v,w,cx|{let _=v.render(w,cx);}).unwrap();
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)));paint(cx);
    // Files only on Files changed
    assert_eq!(f.viewer.read_with(cx,|v,_|v.sidebar_w()),0.,"no file list on Conversation");
    assert_eq!(f.viewer.read_with(cx,|v,_|v.pr_pane().1),f.viewer.read_with(cx,|v,_|v.top()-hdr_h()),"over the empty header row");
    // the tabs sit in the middle of the header
    f.viewer.read_with(cx,|v,_|assert_eq!(v.toolbar_slots().into_iter().filter(|s|s.center).map(|s|s.id).collect::<Vec<_>>(),["pr-tab-conversation","pr-tab-commits","pr-tab-files"],"the tabs sit in the toolbar's middle"));
    // a drag selects text and Ctrl/Cmd+C copies it
    let (x,y,end_x)=f.viewer.read_with(cx,|v,_|{let d=v.pr_dialog.as_ref().unwrap();let row=d.page.rows.iter().position(|r|r.text().starts_with("Description line")).unwrap();let col=d.page.rows[row].indent;let (x,y)=d.point_of(row,col,v.pr_pane(),v.char_w);let end_x=d.point_of(row,col+11,v.pr_pane(),v.char_w).0;(x,y+2.,end_x)});
    click(cx,&f,x,y);
    f.window.update(cx,|v,w,cx|{v.on_mouse_move(&MouseMoveEvent{position:point(px(end_x),px(y)),pressed_button:Some(MouseButton::Left),modifiers:Default::default()},w,cx);v.on_mouse_up(&MouseUpEvent{button:MouseButton::Left,position:point(px(end_x),px(y)),modifiers:Default::default(),click_count:1},w,cx);}).unwrap();
    key(cx,&f,"c",true,false,false);
    assert_eq!(cx.read_from_clipboard().and_then(|c|c.text()).as_deref(),Some("Description"));
    // Commits keeps its place across Files changed
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Commits)));paint(cx);
    key(cx,&f,"pagedown",false,false,false);
    let scrolled=f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().scroll);assert!(scrolled>0);
    key(cx,&f,"escape",false,false,false);paint(cx);
    assert!(f.viewer.read_with(cx,|v,_|v.sidebar_w())>0.,"Files changed has the file list");
    run(cx,&f,crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Commits)));paint(cx);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().scroll),scrolled,"back where it was");
    // Back from one commit returns to the whole PR's Commits
    f.window.update(cx,|v,_,cx|v.repository_command(crate::repository_session::RepositoryCommand::PrCommit(Some("0000001abc".into())),cx)).unwrap();
    assert!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().can_go_back()));
    f.viewer.update(cx,|v,_|v.repo.as_mut().unwrap().go_back());
    f.viewer.read_with(cx,|v,_|{let pr=v.repo.as_ref().unwrap().pr.as_ref().unwrap();assert_eq!(pr.shown_commit,None);assert_eq!(pr.details().map(|d|d.tab),Some(DetailsTab::Commits));});
    paint(cx);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.pr_dialog.as_ref().unwrap().scroll),scrolled,"the list as it was");
}

fn type_keys(cx: &mut TestAppContext, f: &Fixture, text: &str) {
    for ch in text.chars() { key(cx, f, &ch.to_string(), false, false, false); }
}
fn find_hits(cx: &mut TestAppContext, f: &Fixture) -> Vec<(u32, std::ops::Range<usize>)> {
    f.viewer.read_with(cx, |v, _| v.comparison.find.as_ref().map(|x| x.hits.clone()).unwrap_or_default())
}

#[gpui::test]
fn gui_find_options_toggle_by_key_and_click_and_report_bad_expressions(cx: &mut TestAppContext) {
    let f = fixture(cx, "Foo foo food\n", "x\n");
    f.viewer.update(cx, |v, _| v.diff_focus = Side::Left);
    key(cx, &f, "f", true, false, false);
    type_keys(cx, &f, "foo");
    assert_eq!(find_hits(cx, &f).len(), 3);
    key(cx, &f, "c", false, false, true);
    assert_eq!(find_hits(cx, &f), [(0, 4..7), (0, 8..11)]);
    key(cx, &f, "w", false, false, true);
    assert_eq!(find_hits(cx, &f), [(0, 4..7)]);
    // A click on the regex toggle, then an unclosed group.
    let (_, (x, y, w, h)) = f.viewer.read_with(cx, |v, _| v.find_toggles()[2]);
    click(cx, &f, x + w / 2.0, y + h / 2.0);
    assert!(f.viewer.read_with(cx, |v, _| v.comparison.find_options.regex));
    key(cx, &f, "backspace", false, false, false);
    key(cx, &f, "backspace", false, false, false);
    key(cx, &f, "backspace", false, false, false);
    key(cx, &f, "(", false, false, false);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.find.as_ref().and_then(|x| x.error().map(String::from))), Some("unclosed group".into()));
    assert!(f.viewer.read_with(cx, |v, _| v.find_control().unwrap().counter.ends_with("unclosed group")), "the bar shows the reason instead of the count");
    key(cx, &f, "f3", false, false, false);
    assert!(find_hits(cx, &f).is_empty());
}

#[gpui::test]
fn gui_change_set_search_takes_options_and_changed_only_moved_to_alt_o(cx: &mut TestAppContext) {
    let f = fixture(cx, "needle Needle\n", "needle\n");
    let encoded=std::iter::once(0xfeff).chain("Needle\n".encode_utf16()).flat_map(u16::to_le_bytes).collect::<Vec<_>>();
    std::fs::write(f.dir.path().join("right.txt"),encoded).unwrap();
    key(cx, &f, "f", true, true, false);
    type_keys(cx, &f, "Needle");
    key(cx, &f, "c", false, false, true);
    assert!(f.viewer.read_with(cx, |v, _| v.search.as_ref().unwrap().options().case && v.comparison.find_options.case));
    assert!(f.viewer.read_with(cx, |v, _| v.search_prompt().unwrap().contains("alt-o changed lines only")));
    let (_, (x, y, w, h)) = f.viewer.read_with(cx, |v, _| v.search_toggles()[1]);
    click(cx, &f, x + w / 2.0, y + h / 2.0);
    assert!(f.viewer.read_with(cx, |v, _| v.search.as_ref().unwrap().options().word), "a click on W");
    click(cx, &f, x + w / 2.0, y + h / 2.0);
    key(cx, &f, "o", false, false, true);
    assert!(f.viewer.read_with(cx, |v, _| v.search.as_ref().unwrap().changed_only));
    key(cx, &f, "o", false, false, true);
    key(cx, &f, "enter", false, false, false);
    let until = Instant::now() + std::time::Duration::from_secs(10);
    while !f.viewer.read_with(cx, |v, _| v.search.as_ref().is_some_and(|s| s.ended.is_some())) {
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        cx.executor().run_until_parked();
        assert!(Instant::now() < until, "search did not finish");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(f.viewer.read_with(cx, |v, _| v.search.as_ref().unwrap().hits.len()), 2, "case-sensitive Needle in UTF-8 and UTF-16");
}

#[gpui::test]
fn gui_go_to_accepts_a_comma_column(cx: &mut TestAppContext) {
    let f = fixture(cx, "a\nbcdef\n", "a\nbcdeX\n");
    key(cx, &f, "g", true, false, false);
    type_keys(cx, &f, "2,3");
    key(cx, &f, "enter", false, false, false);
    let s = f.viewer.read_with(cx, |v, _| v.sel.unwrap());
    assert_eq!((s.head.line, s.head.col), (1, 2));
}

#[gpui::test]
fn gui_find_toggle_boxes_follow_the_measured_character_width(cx: &mut TestAppContext) {
    let f = fixture(cx, "Foo foo food\n", "x\n");
    f.viewer.update(cx, |v, _| { v.diff_focus = Side::Left; v.char_w = 9.6; }); // a 16 px font
    key(cx, &f, "f", true, false, false);
    type_keys(cx, &f, "a long query here x");
    let control = f.viewer.read_with(cx, |v, _| v.find_control().unwrap());
    for (option, bounds) in control.toggles {
        assert!(bounds.origin.x >= control.bounds.origin.x);
        assert!(bounds.right() <= control.bounds.right());
        assert_eq!(control.option_at(bounds.center()), Some(option));
    }
    let (_, aa) = control.toggles[0];
    assert!((f32::from(aa.size.width) - (3.0 * 9.6 + 2.0)).abs() < 0.01);

}

#[gpui::test]
fn gui_find_shared_control_stays_in_tiny_panes(cx: &mut TestAppContext) {
    let f = fixture(cx, "foo\n", "Foo\n");
    key(cx, &f, "f", true, false, false);
    type_keys(cx, &f, "a query much wider than the pane");
    for width in [1.0, 16.0, 80.0, 180.0, 600.0] {
        f.viewer.update(cx, |v, _| v.panes.viewport = [width, 400.0]);
        let b = f.viewer.read_with(cx, |v, _| v.find_control().unwrap());
        assert!(b.bounds.origin.x >= px(0.0));
        assert!(b.bounds.right() <= px(width));
        if width <= 180.0 { assert!(b.toggles.iter().all(|(_, t)| t.size.width == px(0.0))); }
    }
}

#[gpui::test]
fn gui_an_open_dialog_takes_clicks_over_the_find_toggles(cx: &mut TestAppContext) {
    let f = fixture(cx, "Foo foo food\n", "x\n");
    f.viewer.update(cx, |v, _| v.diff_focus = Side::Left);
    key(cx, &f, "f", true, false, false);
    type_keys(cx, &f, "foo");
    let (_, (x, y, w, h)) = f.viewer.read_with(cx, |v, _| v.find_toggles()[0]);
    f.viewer.update(cx, |v, _| v.help = true); // as `?` with the Files list focused opens it
    assert!(f.viewer.read_with(cx, |v, _| v.modal() == Some(Modal::Help)), "help is open over the find bar");
    click(cx, &f, x + w / 2.0, y + h / 2.0);
    assert!(!f.viewer.read_with(cx, |v, _| v.comparison.find_options.case), "the click went to the help dialog");
}

/// A Rust file whose `fn f` body is taller than the window, inside `mod m`.
fn nested_rust() -> String {
    format!("mod m {{\n    fn f() {{\n{}    }}\n}}\n", "        a;\n".repeat(80))
}

fn sticky_fixture(cx: &mut TestAppContext) -> Fixture {
    let src = nested_rust();
    let f = fixture_named(cx, &src, &src.replacen("        a;\n", "        b;\n", 1), "rs");
    f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap(); // the first render starts the parse
    cx.executor().run_until_parked();
    f.viewer.update(cx, |v, _| v.comparison.view.scroll_by(10.0, Side::Right));
    f
}

#[gpui::test]
fn gui_sticky_headers_pin_and_jump(cx: &mut TestAppContext) {
    let f = sticky_fixture(cx);
    let lines = f.viewer.read_with(cx, |v, _| v.sticky_lines());
    assert_eq!(lines[1].iter().map(|s| (s.line, s.row)).collect::<Vec<_>>(), vec![(0, 0), (1, 1)], "mod m, then fn f");
    assert_eq!(lines[0].iter().map(|s| s.line).collect::<Vec<_>>(), vec![0, 1], "the left side too");
    let (x, y) = f.window.update(cx, |v, w, cx| {
        let _ = v.render(w, cx);
        (v.diff_x() + v.comparison_widths(f32::from(w.viewport_size().width) - v.diff_x())[0] + GUTTER_W + 80.0, v.top() + 1.5 * line_h())
    }).unwrap();
    click(cx, &f, x, y);
    let top = f.viewer.read_with(cx, |v, _| v.comparison.view.frame().right_top);
    assert!(top <= 1.0, "the click on fn f's sticky line shows fn f: top {top}");
}

#[gpui::test]
fn gui_sticky_scroll_toggles_off_and_on(cx: &mut TestAppContext) {
    let f = sticky_fixture(cx);
    run(cx, &f, crate::catalog::Action::Interaction(crate::commands::Interaction::StickyScroll));
    assert!(f.viewer.read_with(cx, |v, _| v.sticky_lines()).iter().all(Vec::is_empty), "off");
    run(cx, &f, crate::catalog::Action::Interaction(crate::commands::Interaction::StickyScroll));
    cx.executor().run_until_parked();
    assert_eq!(f.viewer.read_with(cx, |v, _| v.sticky_lines())[1].len(), 2, "on again");
}

#[gpui::test]
fn gui_sticky_scroll_turned_on_after_a_parse_without_declarations(cx: &mut TestAppContext) {
    let src = nested_rust();
    let f = fixture_named(cx, &src, &src.replacen("        a;\n", "        b;\n", 1), "rs");
    f.viewer.update(cx, |v, _| v.sticky_on = false); // as `[view] sticky_scroll = false` starts it
    f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
    cx.executor().run_until_parked();
    f.viewer.update(cx, |v, _| v.comparison.view.scroll_by(10.0, Side::Right));
    run(cx, &f, crate::catalog::Action::Interaction(crate::commands::Interaction::StickyScroll));
    f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
    cx.executor().run_until_parked();
    assert_eq!(f.viewer.read_with(cx, |v, _| v.sticky_lines())[1].len(), 2, "the declarations are parsed when it turns on");
}

#[gpui::test]
fn gui_a_click_in_the_find_bar_over_sticky_lines_does_not_jump(cx: &mut TestAppContext) {
    let f = sticky_fixture(cx);
    key(cx, &f, "f", true, false, false);
    let (x, y, top) = f.window.update(cx, |v, w, cx| {
        let _ = v.render(w, cx);
        let (bx, by, bw, bh) = v.find_bar_box().expect("the find bar is open");
        (bx + bw / 2.0, by + bh / 2.0, v.comparison.view.frame().right_top)
    }).unwrap();
    assert!(f.viewer.read_with(cx, |v, _| !v.sticky_lines()[1].is_empty()), "the bar sits over the sticky band");
    click(cx, &f, x, y);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.view.frame().right_top), top, "no jump");
}

#[gpui::test]
fn gui_committed_zip_opens_as_a_folder_whose_files_show_their_changes_read_only(cx:&mut TestAppContext) {
    use crate::archive::testing::zip;
    let f=fixture(cx,"old\n","new\n");let root=f.dir.path();workflow_git(root,&["init","-q","-b","main"]);
    std::fs::write(root.join("bundle.zip"),zip(&[("src/main.rs",b"fn main() {\n    old();\n}\n"),("gone.txt",b"bye\n")])).unwrap();
    workflow_git(root,&["add","bundle.zip"]);workflow_git(root,&["commit","-qm","Add bundle"]);
    let v2=zip(&[("src/main.rs",b"fn main() {\n    new_call();\n}\n"),("added.txt",b"hi\n")]);
    std::fs::write(root.join("bundle.zip"),&v2).unwrap();workflow_git(root,&["commit","-qam","Change bundle"]);
    let spec=Spec::Git{cached:false,revs:vec!["HEAD~".into(),"HEAD".into()],paths:Vec::new(),untracked:false};
    let set=Arc::new(ChangeSet::load(&spec,root).unwrap());let c=&set.comparisons[0];
    let doc=Arc::new(Doc::load(&set,0,Path::new(&c.old_path),Path::new(&c.path),false).unwrap());let view=doc.view(&doc.line_diff(&Options::default()),30.);
    f.viewer.update(cx,|v,cx|{*v=Viewer::new(set,spec,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);v.layout.set_files(true);});
    let dirs=|cx:&mut TestAppContext|f.viewer.read_with(cx,|v,_|v.nav.iter().filter_map(|r|if let NavRow::Dir{path,collapsed,files,..}=r{Some((path.clone(),*collapsed,*files))}else{None}).collect::<Vec<_>>());
    // the archive starts collapsed, and with nothing else changed its summary covers the first file
    assert_eq!(dirs(cx),[("bundle.zip".to_string(),true,3)]);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().map(|s|(s.folder.clone(),s.files.len()))),Some(("bundle.zip".into(),3)));
    // a click on the heading's arrow expands it: the Files list groups the archive's members under it, as folders
    let y=f.viewer.read_with(cx,|v,_|v.nav_top()+2.);
    click(cx,&f,f.viewer.read_with(cx,|v,_|v.nav_x()+12.),y);
    assert_eq!(dirs(cx),[("bundle.zip".to_string(),false,0),("bundle.zip/src".to_string(),false,0)]);
    // a click on the member opens its comparison: both versions of the file inside the archive
    let y=f.viewer.read_with(cx,|v,_|v.top()+v.nav.iter().position(|r|matches!(r,NavRow::File{index,..}if v.set.comparisons[*index].path=="bundle.zip/src/main.rs")).unwrap()as f32*line_h()+2.);
    click(cx,&f,f.viewer.read_with(cx,|v,_|v.picker_w()+80.),y);
    workflow_wait(cx,&f,|v|v.set.comparisons.get(v.current).is_some_and(|c|c.path=="bundle.zip/src/main.rs")&&v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
    f.viewer.read_with(cx,|v,_|{let d=v.doc().unwrap();assert!(d.text(Side::Left).contains("old();")&&d.text(Side::Right).contains("new_call();"));});
    // Apply is refused with the reason, and nothing is written
    for _ in 0..8 {if f.viewer.read_with(cx,|v,_|!v.nav_focused&&!v.panel_focused){break;}key(cx,&f,"tab",false,false,false);}
    key(cx,&f,"right",false,true,true);
    assert!(f.viewer.read_with(cx,|v,_|v.write_error.as_ref().is_some_and(|e|e.contains("inside an archive"))),"{:?}",f.viewer.read_with(cx,|v,_|v.write_error.clone()));
    assert_eq!(std::fs::read(root.join("bundle.zip")).unwrap(),v2);
    // Show file history follows the archive git knows
    run(cx,&f,crate::catalog::Action::File(crate::file_actions::FileAction::History));
    workflow_wait(cx,&f,|v|v.repo.as_ref().is_some_and(|r|!r.busy()&&r.picker.file_paths.is_some()));
    assert_eq!(f.viewer.read_with(cx,|v,_|v.repo.as_ref().unwrap().picker.file_paths.as_ref().unwrap().0.clone()),"bundle.zip");
}

/// A repository whose last commit changes `file.rs` and the members of `bundle.zip`, open on `HEAD~ HEAD`.
fn archive_folders_fixture(cx:&mut TestAppContext)->Fixture {
    use crate::archive::testing::zip;
    let f=fixture(cx,"old\n","new\n");let root=f.dir.path();workflow_git(root,&["init","-q","-b","main"]);
    std::fs::write(root.join("file.rs"),"first\n").unwrap();
    std::fs::write(root.join("bundle.zip"),zip(&[("src/main.rs",b"fn main() {\n    old();\n}\n"),("gone.txt",b"bye\n")])).unwrap();
    workflow_git(root,&["add","file.rs","bundle.zip"]);workflow_git(root,&["commit","-qm","Add bundle"]);
    std::fs::write(root.join("bundle.zip"),zip(&[("src/main.rs",b"fn main() {\n    new_call();\n    more();\n}\n"),("added.txt",b"hi\n")])).unwrap();
    std::fs::write(root.join("file.rs"),"third\n").unwrap();workflow_git(root,&["commit","-qam","Change bundle"]);
    let spec=Spec::Git{cached:false,revs:vec!["HEAD~".into(),"HEAD".into()],paths:Vec::new(),untracked:false};
    let set=Arc::new(ChangeSet::load(&spec,root).unwrap());let c=&set.comparisons[0];
    let doc=Arc::new(Doc::load(&set,0,Path::new(&c.old_path),Path::new(&c.path),false).unwrap());let view=doc.view(&doc.line_diff(&Options::default()),30.);
    f.viewer.update(cx,|v,cx|{*v=Viewer::new(set,spec,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);v.layout.set_files(true);});
    workflow_wait(cx,&f,|v|v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
    f
}

fn nav_labels(cx:&mut TestAppContext,f:&Fixture)->Vec<String> {
    f.viewer.read_with(cx,|v,_|v.sidebar_rows().iter().map(|r|r.parts.iter().map(|p|p.0.as_str()).collect::<String>().trim().to_string()).collect())
}

/// The y of Files row `row`, for a click.
fn nav_row_y(cx:&mut TestAppContext,f:&Fixture,row:usize)->f32 { f.viewer.read_with(cx,|v,_|v.nav_top()+(row as f32-v.nav_scroll+0.5)*line_h()) }

#[gpui::test]
fn gui_archive_folders_start_collapsed_and_show_a_summary_of_their_changed_files(cx:&mut TestAppContext) {
    use crate::folders::{FolderAction,Stat};
    let f=archive_folders_fixture(cx);
    // the archive is one collapsed heading with its file count; the plain file is shown
    assert_eq!(nav_labels(cx,&f),["▾ .","M file.rs","▸ bundle.zip · 3 files"]);
    // the header counts the files in collapsed folders too
    assert_eq!(f.viewer.read_with(cx,|v,_|v.nav_header()),"4 changed files");
    assert_eq!(f.viewer.read_with(cx,|v,_|v.set.comparisons[v.comparison.current].path.clone()),"file.rs");
    // ] skips the hidden files
    key(cx,&f,"]",false,false,false);
    f.viewer.read_with(cx,|v,_|{assert_eq!(v.set.comparisons[v.comparison.current].path,"file.rs");assert!(v.toast.as_ref().is_some_and(|t|t.0.contains("No later visible file")));});
    // a click on the heading's name shows the summary, its counts filled in the background
    let y=nav_row_y(cx,&f,2);
    click(cx,&f,f.viewer.read_with(cx,|v,_|v.nav_x()+80.),y);
    let summary=f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().cloned()).expect("the summary shows");
    assert_eq!(summary.folder,"bundle.zip");
    workflow_wait(cx,&f,|v|summary.files.iter().all(|&i|v.comparison.folders.stat(i).is_some()));
    let main=f.viewer.read_with(cx,|v,_|summary.files.iter().copied().find(|&i|v.set.comparisons[i].path.ends_with("src/main.rs")).unwrap());
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.folders.stat(main).cloned()),Some(Stat::Lines{added:2,removed:1}));
    // the heading is the Files row marked, and the summary covers the comparison: no placeholder or file labels
    f.viewer.update(cx,|v,_|{assert!(v.sidebar_rows()[2].current&&!v.sidebar_rows()[1].current);let frame=v.summary_frame().unwrap();assert_eq!(frame.rows_text(),[("A added.txt".to_string(),"+1".to_string()),("D gone.txt".into(),"−1".into()),("M src/main.rs".into(),"+2 −1".into())]);});
    // a click on the summary's heading gives it the keyboard; down to main.rs, Enter opens it, and the archive expands
    let (rx,ry,_,_)=f.viewer.read_with(cx,|v,_|v.pr_pane());
    click(cx,&f,rx+40.,ry+4.);
    assert!(!f.viewer.read_with(cx,|v,_|v.nav_focused));
    for _ in 0..summary.files.len() { if f.viewer.read_with(cx,|v,_|v.comparison.folders.selected_file())==Some(main){break;} key(cx,&f,"down",false,false,false); }
    key(cx,&f,"enter",false,false,false);
    workflow_wait(cx,&f,|v|v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
    f.viewer.read_with(cx,|v,_|{assert!(v.comparison.folders.summary().is_none());assert_eq!(v.set.comparisons[v.comparison.current].path,"bundle.zip/src/main.rs");});
    assert!(nav_labels(cx,&f).contains(&"▾ bundle.zip".to_string()));
    // Collapse all archives from the palette hides the file on screen, so its folder's summary takes its place
    run(cx,&f,crate::catalog::Action::Folder(FolderAction::CollapseArchives));
    f.viewer.read_with(cx,|v,_|{assert!(v.comparison.folders.collapsed("bundle.zip"));assert_eq!(v.comparison.folders.summary().map(|s|s.folder.as_str()),Some("bundle.zip/src"));});
    // Esc goes back to the file, its folders expanded again
    for _ in 0..8 {if f.viewer.read_with(cx,|v,_|!v.nav_focused&&!v.panel_focused){break;}key(cx,&f,"tab",false,false,false);}
    key(cx,&f,"escape",false,false,false);
    f.viewer.read_with(cx,|v,_|{assert!(v.comparison.folders.summary().is_none()&&!v.comparison.folders.collapsed("bundle.zip"));assert_eq!(v.set.comparisons[v.comparison.current].path,"bundle.zip/src/main.rs");});
    // with nothing collapsed, the palette says why
    use crate::catalog::Availability;
    assert!(f.viewer.read_with(cx,|v,_|v.disabled(crate::catalog::Action::Folder(FolderAction::ExpandAll))).is_some_and(|why|why.contains("No folder is collapsed")));
}

#[gpui::test]
fn gui_files_keys_walk_folder_headings_as_a_tree(cx:&mut TestAppContext) {
    let f=archive_folders_fixture(cx);
    // a click on the file gives Files the keyboard; Down reaches the archive's heading: its summary shows
    let y=nav_row_y(cx,&f,1);
    click(cx,&f,f.viewer.read_with(cx,|v,_|v.nav_x()+80.),y);
    assert!(f.viewer.read_with(cx,|v,_|v.nav_focused));
    key(cx,&f,"down",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().map(|s|s.folder.clone())),Some("bundle.zip".into()));
    // Right expands it, Down walks into it and opens its first file
    key(cx,&f,"right",false,false,false);
    assert!(!f.viewer.read_with(cx,|v,_|v.comparison.folders.collapsed("bundle.zip")));
    key(cx,&f,"down",false,false,false);
    workflow_wait(cx,&f,|v|v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
    f.viewer.read_with(cx,|v,_|{assert_eq!(v.set.comparisons[v.comparison.current].path,"bundle.zip/added.txt");assert!(v.comparison.folders.summary().is_none());});
    // Left on a file goes to its folder's heading; Left there collapses it
    key(cx,&f,"left",false,false,false);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().map(|s|s.folder.clone())),Some("bundle.zip".into()));
    key(cx,&f,"left",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.comparison.folders.collapsed("bundle.zip")));
    assert_eq!(nav_labels(cx,&f).last().map(String::as_str),Some("▸ bundle.zip · 3 files"));
}

/// A repository whose last commit changes all 60 files inside `big.zip`, open on `HEAD~ HEAD` (on the zip's summary).
fn big_archive_fixture(cx:&mut TestAppContext)->Fixture {
    use crate::archive::testing::zip;
    let f=fixture(cx,"old\n","new\n");let root=f.dir.path();workflow_git(root,&["init","-q","-b","main"]);
    let files=|v:&str|(0..60).map(|i|(format!("dir/f{i:02}.txt"),format!("{v} {i}\n"))).collect::<Vec<_>>();
    let write=|v:&str|{let files=files(v);let pairs:Vec<(&str,&[u8])>=files.iter().map(|(p,c)|(p.as_str(),c.as_bytes())).collect();std::fs::write(root.join("big.zip"),zip(&pairs)).unwrap();};
    write("old");workflow_git(root,&["add","big.zip"]);workflow_git(root,&["commit","-qm","Add big"]);
    write("new");workflow_git(root,&["commit","-qam","Change big"]);
    let spec=Spec::Git{cached:false,revs:vec!["HEAD~".into(),"HEAD".into()],paths:Vec::new(),untracked:false};
    let set=Arc::new(ChangeSet::load(&spec,root).unwrap());let c=&set.comparisons[0];
    let doc=Arc::new(Doc::load(&set,0,Path::new(&c.old_path),Path::new(&c.path),false).unwrap());let view=doc.view(&doc.line_diff(&Options::default()),30.);
    f.viewer.update(cx,|v,cx|{*v=Viewer::new(set,spec,false,None,doc,view,false,Options::default(),gpui::font("monospace"),Instant::now(),false,cx);v.layout.set_files(true);});
    f
}

/// The summary rows painted now, and which of them has the keyboard.
fn summary_rows(cx:&mut TestAppContext,f:&Fixture)->(Vec<String>,Option<String>) {
    f.viewer.update(cx,|v,_|{let frame=v.summary_frame().expect("the summary shows");let rows=frame.rows_text();(rows.iter().map(|r|r.0.clone()).collect(),frame.selected_text())})
}

#[gpui::test]
fn gui_a_long_summary_scrolls_with_its_keyboard_row_at_every_size_and_esc_leaves_it(cx:&mut TestAppContext) {
    let f=big_archive_fixture(cx);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().map(|s|s.files.len())),Some(60));
    cx.simulate_window_resize(f.window.into(),size(px(1200.0),px(400.0)));cx.executor().run_until_parked();
    let (rx,ry,_,_)=f.viewer.read_with(cx,|v,_|v.pr_pane());
    click(cx,&f,rx+40.,ry+4.);
    // End then Up: the keyboard row is always painted
    key(cx,&f,"end",false,false,false);
    assert_eq!(summary_rows(cx,&f).1.as_deref(),Some("M dir/f59.txt"));
    key(cx,&f,"up",false,false,false);
    let (rows,selected)=summary_rows(cx,&f);
    assert_eq!(selected.as_deref(),Some("M dir/f58.txt"));
    assert!(!rows.contains(&"M dir/f00.txt".to_string()),"{rows:?}");
    // the wheel scrolls away from it, and the next key brings it back
    f.window.update(cx,|v,w,cx|v.on_wheel(&ScrollWheelEvent{position:point(px(rx+60.),px(ry+80.)),delta:ScrollDelta::Lines(point(0.,30.)),..Default::default()},w,cx)).unwrap();
    let (rows,selected)=summary_rows(cx,&f);
    assert!(rows.contains(&"M dir/f00.txt".to_string())&&selected.is_none(),"{rows:?}");
    key(cx,&f,"up",false,false,false);
    assert_eq!(summary_rows(cx,&f).1.as_deref(),Some("M dir/f57.txt"));
    // small windows keep the selection painted
    for (w,h) in [(400.,240.),(160.,120.),(60.,60.),(1200.,400.)] {
        cx.simulate_window_resize(f.window.into(),size(px(w),px(h)));cx.executor().run_until_parked();
        key(cx,&f,"end",false,false,false);key(cx,&f,"up",false,false,false);
        assert_eq!(summary_rows(cx,&f).1.as_deref(),Some("M dir/f58.txt"),"{w}x{h}");
    }
    // Esc from the scrolled summary returns to the file behind it, its folders expanded
    key(cx,&f,"escape",false,false,false);
    f.viewer.read_with(cx,|v,_|{assert!(v.comparison.folders.summary().is_none()&&!v.comparison.folders.collapsed("big.zip"));assert_eq!(v.set.comparisons[v.comparison.current].path,"big.zip/dir/f00.txt");});
}

#[gpui::test]
fn gui_dialogs_over_the_summary_take_its_keys(cx:&mut TestAppContext) {
    let f=big_archive_fixture(cx);
    let (rx,ry,_,_)=f.viewer.read_with(cx,|v,_|v.pr_pane());
    click(cx,&f,rx+40.,ry+4.);
    key(cx,&f,"down",false,false,false);
    let selected=f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().map(|s|s.selected));
    assert_eq!(selected,Some(1));
    // the palette and help consume the arrows and Enter; the summary keeps its row and stays open
    key(cx,&f,"p",true,true,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_some()));
    key(cx,&f,"down",false,false,false);key(cx,&f,"escape",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.command_palette.is_none()));
    key(cx,&f,"?",false,false,false);
    assert!(f.viewer.read_with(cx,|v,_|v.help));
    key(cx,&f,"down",false,false,false);key(cx,&f,"end",false,false,false);key(cx,&f,"escape",false,false,false);
    f.viewer.read_with(cx,|v,_|{assert!(!v.help);assert_eq!(v.comparison.folders.summary().map(|s|s.selected),selected);assert_eq!(v.comparison.current,0);});
}

#[gpui::test]
fn gui_a_summary_and_a_pr_details_tab_never_share_the_diff_area(cx:&mut TestAppContext) {
    use crate::{catalog::{Action,PrAction},folders::FolderAction,pr_session::DetailsTab};
    let f=big_archive_fixture(cx);
    f.viewer.update(cx,|v,_|{
        let pr:crate::pr::PullRequest=serde_json::from_str(include_str!("../../fixtures/pr/view.json")).unwrap();
        let mut state=crate::pr_session::PrState::new(pr,crate::pr::Fetched{number:14130,merge_base:"base".into(),head:"head".into()});
        state.marks=None;
        v.repo.get_or_insert_with(||crate::repository_session::RepositorySession::with_picker(Picker::new(f.dir.path().into(),false))).pr=Some(state);
    });
    assert!(f.viewer.read_with(cx,|v,_|v.summary_shown()));
    // a details tab, however it opens, closes the summary
    run(cx,&f,Action::Pr(PrAction::Details(DetailsTab::Conversation)));
    assert_eq!(pr_tab(cx,&f),Some(DetailsTab::Conversation));
    assert!(!f.viewer.read_with(cx,|v,_|v.summary_shown()));
    // and the summary closes the tab
    run(cx,&f,Action::Folder(FolderAction::Summary));
    assert_eq!(pr_tab(cx,&f),None);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().map(|s|s.folder.clone())),Some("big.zip/dir".into()));
}

#[gpui::test]
fn gui_a_heading_in_each_group_shows_that_group_only(cx:&mut TestAppContext) {
    use crate::archive::testing::zip;
    let f=fixture(cx,"old\n","new\n");let root=f.dir.path();workflow_git(root,&["init","-q","-b","main"]);
    std::fs::write(root.join("a.zip"),zip(&[("f.txt",b"1\n")])).unwrap();workflow_git(root,&["add","a.zip"]);workflow_git(root,&["commit","-qm","Add a.zip"]);
    std::fs::write(root.join("a.zip"),zip(&[("f.txt",b"2\n")])).unwrap();workflow_git(root,&["add","a.zip"]);
    std::fs::write(root.join("a.zip"),zip(&[("f.txt",b"3\n"),("g.txt",b"g\n")])).unwrap();
    let set=crate::repo::local_changes(root,false).unwrap();
    f.viewer.update(cx,|v,cx|{v.spec=crate::repository_session::local_spec();v.show_change_set(set,cx);v.layout.set_files(true);});
    assert_eq!(nav_labels(cx,&f)[..4],["STAGED · 1","▸ a.zip · 1 file","UNSTAGED · 2","▸ a.zip · 2 files"]);
    // a click on the Unstaged heading's name summarizes only that group
    let y=nav_row_y(cx,&f,3);
    click(cx,&f,f.viewer.read_with(cx,|v,_|v.nav_x()+80.),y);
    f.viewer.read_with(cx,|v,_|{let s=v.comparison.folders.summary().expect("the summary shows");
        assert_eq!(s.files.iter().map(|&i|v.set.comparisons[i].path.as_str()).collect::<Vec<_>>(),["a.zip/f.txt","a.zip/g.txt"]);assert_eq!(s.range,v.set.groups[1].1);});
}

#[gpui::test]
fn gui_archive_grouped_summary_cursor_uses_selected_group(cx:&mut TestAppContext) {
    use crate::archive::testing::zip;
    let f=fixture(cx,"old\n","new\n");let root=f.dir.path();workflow_git(root,&["init","-q","-b","main"]);
    std::fs::write(root.join("a.zip"),zip(&[("f.txt",b"1\n")])).unwrap();workflow_git(root,&["add","a.zip"]);workflow_git(root,&["commit","-qm","Add a.zip"]);
    std::fs::write(root.join("a.zip"),zip(&[("f.txt",b"2\n")])).unwrap();workflow_git(root,&["add","a.zip"]);
    std::fs::write(root.join("a.zip"),zip(&[("f.txt",b"3\n"),("g.txt",b"g\n")])).unwrap();
    let set=crate::repo::local_changes(root,false).unwrap();
    f.viewer.update(cx,|v,cx|{v.spec=crate::repository_session::local_spec();v.show_change_set(set,cx);v.layout.set_files(true);});
    let y=nav_row_y(cx,&f,3);
    click(cx,&f,f.viewer.read_with(cx,|v,_|v.nav_x()+80.),y);
    let expected=f.viewer.read_with(cx,|v,_|v.set.groups[1].1.start);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.folders.summary().unwrap().range.start),expected);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.sidebar_rows().iter().filter(|r| r.current).count()), 1, "only the selected group heading is highlighted");
    key(cx,&f,"right",false,false,false);key(cx,&f,"down",false,false,false);
    workflow_wait(cx,&f,|v|v.doc().is_some()&&v.comparison.diffed==v.comparison.generation);
    assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.current),expected,"Down should open first Unstaged member, not Staged");
}

#[gpui::test]
fn gui_archive_plain_reload_reads_changed_tar_members(cx: &mut TestAppContext) {
    let f = fixture(cx, "old\n", "new\n");
    let (left, right) = (f.dir.path().join("left.tgz"), f.dir.path().join("right.tgz"));
    let tar = |text: &[u8]| crate::archive::testing::tar(&[("f.txt", text)], crate::archive::Compression::Gzip);
    std::fs::write(&left, tar(b"old\n")).unwrap(); std::fs::write(&right, tar(b"new\n")).unwrap();
    let spec = Spec::Plain { left, right: right.clone() };
    let set = Arc::new(ChangeSet::load(&spec, f.dir.path()).unwrap());
    let c = &set.comparisons[0];
    let doc = Arc::new(Doc::load(&set, 0, Path::new(&c.old_path), Path::new(&c.path), false).unwrap());
    let view = doc.view(&doc.line_diff(&Options::default()), 30.);
    f.viewer.update(cx, |v, cx| *v = Viewer::new(set, spec, false, None, doc, view, false, Options::default(), gpui::font("monospace"), Instant::now(), false, cx));
    std::fs::write(&right, tar(b"updated\n")).unwrap();
    run(cx, &f, crate::catalog::Action::Interaction(crate::commands::Interaction::Reload));
    workflow_wait(cx, &f, |v| v.doc().is_some() && v.comparison.diffed == v.comparison.generation);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.doc().unwrap().text(Side::Right).to_string()), "updated\n");
}

#[gpui::test]
fn gui_summary_reload_refreshes_counts(cx: &mut TestAppContext) {
    use crate::{catalog::Action, commands::Interaction, folders::{FolderAction, Stat}};
    let f = fixture(cx, "old\n", "new\n");
    run(cx, &f, Action::Folder(FolderAction::Summary));
    workflow_wait(cx, &f, |v| v.comparison.folders.stat(0).is_some());
    assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.folders.stat(0).cloned()), Some(Stat::Lines { added: 1, removed: 1 }));
    std::fs::write(f.dir.path().join("right.txt"), "new\nmore\nlast\n").unwrap();
    run(cx, &f, Action::Interaction(Interaction::Reload));
    workflow_wait(cx, &f, |v| v.doc().is_some() && v.comparison.diffed == v.comparison.generation && !v.comparison.folders.busy());
    assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.folders.stat(0).cloned()), Some(Stat::Lines { added: 3, removed: 1 }));
}

#[gpui::test]
fn gui_summary_scrolled_header_click_only_focuses(cx: &mut TestAppContext) {
    let f = big_archive_fixture(cx);
    cx.simulate_window_resize(f.window.into(), size(px(1200.), px(400.))); cx.executor().run_until_parked();
    let (x, y, _, _) = f.viewer.read_with(cx, |v, _| v.pr_pane());
    click(cx, &f, x + 40., y + 4.);
    key(cx, &f, "end", false, false, false);
    summary_rows(cx, &f);
    assert!(f.viewer.read_with(cx, |v, _| v.summary_scroll > 2.));
    for row in [0., 1.] {
        click(cx, &f, x + 40., y + (row + 0.5) * line_h());
        assert!(f.viewer.read_with(cx, |v, _| v.summary_shown()), "header and gap must not open a file");
        assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.folders.summary().unwrap().selected), 59);
    }
}
