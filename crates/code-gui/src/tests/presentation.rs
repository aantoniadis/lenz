use super::*;

#[gpui::test]
fn markdown_source_tokens_and_preview_keep_the_same_unicode_source(cx: &mut TestAppContext) {
    use lenz_content::highlight::Capture;
    let source = "# Heading\r\n\r\nPlain *café* **strong** [site](https://example.test) and `code`.\r\n\r\n```rust\r\nfn main() {}\r\n```\r\n";
    let f = fixture(cx, Some(("tokens.md", source.as_bytes())));
    settle(cx, &f, |r| r.session.state().active().is_some_and(|t| r.session.state().view(t.id()).is_some_and(|v| v.style_at(2, 8).0.is_some())));
    let mut v = visual(cx, &f);
    f.view.read_with(&v, |r, _| {
        let tab = r.session.state().active().unwrap();
        let view = r.session.state().view(tab.id()).unwrap();
        for (needle, expected) in [("café", Capture::Emphasis), ("strong", Capture::Strong), ("site", Capture::Link), ("code", Capture::String)] {
            let line = source.lines().nth(2).unwrap();
            assert_eq!(view.style_at(2, line.find(needle).unwrap()).0, Some(expected));
        }
    });
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::MarkdownSource, w, cx));
    drop(v);
    settle(cx, &f, |r| matches!(r.session.state().rendering(), Some(Rendering::Preview(_))));
    let mut v = visual(cx, &f);
    for id in ["darcula", "light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(id).unwrap())), cx));
        for width in [900.0, 240.0] {
            v.simulate_resize(size(px(width), px(300.0))); show(&mut v);
            f.view.read_with(&v, |r, _| {
                let layout = r.markdown[0].as_ref().unwrap().layout.as_ref().unwrap();
                assert_eq!(layout.preview.doc.source.as_ref(), source);
                assert!(layout.preview.doc.text.contains("Plain café strong site and code."));
                assert!(layout.rows.iter().flat_map(|row| &row.pieces).all(|p| layout.preview.doc.text.get(p.range.clone()).is_some()));
            });
        }
    }
    assert_eq!(std::fs::read(f.dir.path().join("tokens.md")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn command_dropdowns_keep_help_out_of_action_rows(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    f.view.update(&mut v, |r, cx| {
        let rows = r.session.menu(Some(CommandGroup::Metadata));
        let at = rows.iter().position(|row| row.target == MenuTarget::Run(CommandId::ConnectSqlMetadata)).unwrap();
        r.modal = Some(Modal::CommandGroup { group: CommandGroup::Metadata, at }); cx.notify();
    });
    draw(&mut v);
    f.view.read_with(&v, |r, _| {
        assert!(r.modal_lines().iter().all(|(_, line, _)| !line.contains("select its dialect first")));
        assert_eq!(r.modal_layout().unwrap().footer.join(" "), r.session.disabled(CommandId::ConnectSqlMetadata).unwrap());
    });
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::CommandGroup { .. }))));
}

#[gpui::test]
fn annotation_edge_drag_resizes_the_gutter_without_selecting_or_editing_text(cx: &mut TestAppContext) {
    let (f, _) = blame_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1000.0), px(400.0))); draw(&mut v);
    v.simulate_keystrokes("alt-b"); settle(cx, &f, annotated); draw(&mut v);
    let (g, cw, selection) = f.view.read_with(&v, |r, _| (r.panes[0], r.char_w, r.session().state().selection()));
    let x = f32::from(g.text.origin.x);
    let y = f32::from(g.text.origin.y) + line_h() * 1.5;
    drag(&mut v, (x, y), (x + 10.0 * cw, y));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().layout().annotation_cols()), 33);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), selection);
    assert!(f.view.read_with(&v, |r, _| r.session().state().commit_popup().is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"one\ntwo\n");
}

#[gpui::test]
fn markdown_preview_uses_the_editor_pane_instead_of_a_fixed_reading_column(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("wide.md", b"# A wide preview\n\nA paragraph to render.\n")));
    settle(cx, &f, active_is("wide.md"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1500.0), px(500.0)));
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::MarkdownSource, w, cx));
    drop(v);
    settle(cx, &f, |r| matches!(r.session().state().rendering(), Some(Rendering::Preview(_))));
    let mut v = visual(cx, &f);
    show(&mut v);
    f.view.read_with(&v, |r, _| {
        let layout = r.markdown[0].as_ref().unwrap().layout.as_ref().unwrap();
        assert!(layout.column > 1100.0, "the editor preview should use its available width");
        assert!((layout.column + 2.0 * preview::PAD - layout.width).abs() < 1.0);
    });
    v.simulate_resize(size(px(320.0), px(240.0)));
    show(&mut v);
    f.view.read_with(&v, |r, _| {
        let layout = r.markdown[0].as_ref().unwrap().layout.as_ref().unwrap();
        assert!(layout.left + layout.column <= layout.width);
    });
    assert_eq!(std::fs::read(f.dir.path().join("wide.md")).unwrap(), b"# A wide preview\n\nA paragraph to render.\n");
}

#[gpui::test]
fn source_caret_blinks_and_restarts_visible_after_input(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("caret.txt", b"abc\n")));
    settle(cx, &f, active_is("caret.txt"));
    let mut v = visual(cx, &f);
    v.update(|w, _| w.activate_window()); cx.run_until_parked();
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("right"); draw(&mut v);
    let carets = |v: &VisualTestContext| f.view.read_with(v, |r, _| r.visible_rows().iter().filter(|row| row.caret.is_some()).count());
    assert_eq!(carets(&v), 1);
    cx.executor().advance_clock(std::time::Duration::from_millis(500)); draw(&mut v);
    assert_eq!(carets(&v), 0, "the idle caret hides for the second half of its cycle");
    cx.executor().advance_clock(std::time::Duration::from_millis(500)); draw(&mut v);
    assert_eq!(carets(&v), 1);
    cx.executor().advance_clock(std::time::Duration::from_millis(500)); draw(&mut v);
    v.simulate_keystrokes("x"); draw(&mut v);
    assert_eq!(carets(&v), 1, "typing immediately restores the caret");
    cx.executor().advance_clock(std::time::Duration::from_millis(400)); draw(&mut v);
    assert_eq!(carets(&v), 1, "typing starts a fresh visible interval");
    cx.executor().advance_clock(std::time::Duration::from_millis(100)); draw(&mut v);
    assert_eq!(carets(&v), 0);
    assert_eq!(std::fs::read(f.dir.path().join("caret.txt")).unwrap(), b"abc\n");
}

#[gpui::test]
fn source_caret_pauses_when_a_dialog_or_another_window_has_focus(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("caret.txt", b"abc\n")));
    settle(cx, &f, active_is("caret.txt"));
    let mut v = visual(cx, &f);
    v.update(|w, _| w.activate_window()); cx.run_until_parked();
    v.simulate_keystrokes("right"); draw(&mut v);
    let caret = |v: &VisualTestContext| f.view.read_with(v, |r, _| r.visible_rows().iter().any(|row| row.caret.is_some()));
    v.simulate_keystrokes("f2"); draw(&mut v);
    cx.executor().advance_clock(std::time::Duration::from_secs(2)); draw(&mut v);
    assert!(caret(&v));
    assert!(f.view.read_with(&v, |r, _| r.caret_blink.is_none()), "dialogs do not keep a blink timer alive");
    v.simulate_keystrokes("escape"); draw(&mut v);
    cx.executor().advance_clock(std::time::Duration::from_millis(500)); draw(&mut v);
    assert!(!caret(&v), "closing a dialog resumes blinking");
    v.deactivate_window(); draw(&mut v);
    cx.executor().advance_clock(std::time::Duration::from_millis(500)); draw(&mut v);
    assert!(caret(&v), "an inactive window keeps a steady caret");
    assert!(f.view.read_with(&v, |r, _| r.caret_blink.is_none()), "inactive windows do not keep a blink timer alive");
    v.update(|w, _| w.activate_window()); cx.run_until_parked(); draw(&mut v);
    cx.executor().advance_clock(std::time::Duration::from_millis(500)); draw(&mut v);
    assert!(!caret(&v), "activation resumes blinking");
    assert_eq!(std::fs::read(f.dir.path().join("caret.txt")).unwrap(), b"abc\n");
}

#[gpui::test]
fn the_completion_popup_opens_in_the_focused_split_pane(cx: &mut TestAppContext) {
    let text = "fn main() { hel }\nfn hello() {}\nfn helper() {}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    let mut v = visual(cx, &f); v.simulate_resize(size(px(1400.0),px(600.0))); draw(&mut v);
    v.simulate_keystrokes("ctrl-\\"); draw(&mut v);
    let focused = f.view.read_with(&v, |r, _| r.session.state().focused_pane());
    let at = text.find("hel }").unwrap() + 3;
    f.view.update(&mut v, |r, cx| { r.enable_draft_input_for_test(); r.dispatch(Command::Select(Selection {anchor:at,head:at}), cx); r.dispatch(Command::Complete, cx); });
    settle(cx, &f, |r| r.session.state().completion().is_some_and(|c| c.rows().len() > 0));
    let mut v = visual(cx, &f); draw(&mut v);
    let pane = f.view.read_with(&v, |r, _| r.panes.iter().find(|g| g.pane == focused).unwrap().text);
    let popup = v.update(|window,cx|f.view.update(cx,|r,cx|r.completion_frame(window,cx))).unwrap().0;
    assert!(popup.origin.x >= pane.origin.x, "{popup:?} outside the focused pane {pane:?}");
}

#[gpui::test]
fn typing_that_filters_every_suggestion_away_draws_no_popup(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let server = tempfile::tempdir().unwrap(); let launcher = server.path().join("server");
    std::fs::write(&launcher, format!("#!/bin/sh\nexport FAKE_LSP_INCOMPLETE=1\nexec '{}' \"$@\"\n", fake_lsp().display())).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
    let text = "fn main() { hel }\nfn hello() {}\nfn helper() {}\n";
    let f = fixture_with(cx, None, &launcher);
    fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    let at = text.find("hel }").unwrap() + 3;
    f.view.update(cx, |r, cx| { r.enable_draft_input_for_test(); r.dispatch(Command::Select(Selection {anchor:at,head:at}), cx); r.dispatch(Command::Complete, cx); });
    settle(cx, &f, |r| r.session.state().completion().is_some_and(|c| c.rows().len() > 0));
    let mut v = visual(cx, &f); v.simulate_resize(size(px(900.0),px(500.0))); draw(&mut v);
    // The incomplete list stays open with nothing left in it until the server answers again.
    v.simulate_keystrokes("q");
    assert!(f.view.read_with(&v, |r, _| r.session.state().completion().is_some_and(|c| c.rows().len() == 0)));
    draw(&mut v);
    assert!(v.update(|window,cx|f.view.update(cx,|r,cx|r.completion_frame(window,cx))).is_none());
    assert!(f.view.read_with(&v, |r, _| r.session.state().current_source().unwrap().copy_range(0..r.session.state().current_source().unwrap().len_bytes()).unwrap().contains("helq")));
}

#[gpui::test]
fn completion_documentation_keeps_full_markdown_and_scrolls_without_editing(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let server = tempfile::tempdir().unwrap(); let launcher = server.path().join("server");
    std::fs::write(&launcher, format!("#!/bin/sh\nexport FAKE_LSP_COMPLETION_DOC=1\nexec '{}' \"$@\"\n", fake_lsp().display())).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
    let text = "fn main() { hel }\nfn hello() {}\nfn helper() {}\nstruct hello_type;\n";
    let f = fixture_with(cx, None, &launcher);
    let pkg = fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    let at = text.find("hel }").unwrap() + 3;
    f.view.update(cx, |r, cx| { r.enable_draft_input_for_test(); r.dispatch(Command::Select(Selection {anchor:at,head:at}), cx); r.dispatch(Command::Complete, cx); });
    settle(cx, &f, |r| r.session.state().completion().is_some_and(|c| c.documentation_preview().is_some()));
    let mut v = visual(cx, &f); v.simulate_resize(size(px(900.0),px(500.0))); draw(&mut v);
    settle(cx,&f,|r|r.completion_doc.is_some()&&r.completion_doc_building.is_none()); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.completion_documentation_lines().unwrap().join("\n").contains("FINAL DOCUMENTATION LINE")), "documentation is retained beyond twelve lines");
    let frame = v.update(|window,cx|f.view.update(cx,|r,cx|r.completion_frame(window,cx).unwrap()));
    assert_eq!(frame.1[0].2,Some("Function")); assert!(frame.1[0].0.2); assert_eq!(frame.1[0].1.first(),Some(&(0..3)));
    assert_eq!(frame.1[1].2,Some("Struct")); assert!(frame.1[1].3,"kind boundaries are visible without changing server order");
    let selection=f.view.read_with(&v,|r,_|r.session.state().selection());
    for theme in ["darcula","github-light"] {
        f.view.update(&mut v,|r,cx|r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())),cx));
        for (w,h) in [(900.0,500.0),(320.0,240.0),(180.0,160.0)] {
            v.simulate_resize(size(px(w),px(h))); draw(&mut v);
            settle(cx,&f,|r|r.completion_doc.is_some()&&r.completion_doc_building.is_none()); draw(&mut v);
            let (bounds,layout)=f.view.read_with(&v,|r,_|{let d=r.completion_doc.as_ref().unwrap(); (d.bounds,d.layout.clone())});
            assert!(f32::from(bounds.right())<=w+0.1 && f32::from(bounds.bottom())<=h+0.1,"docs fit: {bounds:?}");
            draw(&mut v); assert!(f.view.read_with(&v,|r,_|Arc::ptr_eq(&layout,&r.completion_doc.as_ref().unwrap().layout)),"idle redraw retains shaped Markdown");
            v.simulate_event(ScrollWheelEvent{position:bounds.center(),delta:ScrollDelta::Lines(point(0.0,-3.0)),..Default::default()}); draw(&mut v);
            assert!(f.view.read_with(&v,|r,_|r.completion_doc.as_ref().unwrap().top)>0.0);
            v.simulate_event(MouseDownEvent{position:bounds.center(),button:MouseButton::Left,..Default::default()});
            assert!(f.view.read_with(&v,|r,_|r.session.state().completion().is_some()));
            v.simulate_keystrokes("alt-end"); draw(&mut v);
            f.view.read_with(&v,|r,_|{let d=r.completion_doc.as_ref().unwrap();
                let visible=d.layout.visible(d.top,f32::from(d.bounds.size.height)).iter().flat_map(|row|row.pieces.iter()).map(|piece|&d.layout.preview.doc.text[piece.range.clone()]).collect::<String>();
                assert!(visible.contains("FINAL DOCUMENTATION LINE"),"end of documentation is painted: {visible}");
                assert_eq!(r.session.state().selection(),selection);
                assert!(d.layout.rows.iter().filter(|row|row.kind==preview::RowKind::Code).count()>0);
                for row in &d.layout.rows { for piece in &row.pieces {
                    assert!(piece.x+f32::from(piece.shaped.width)<=d.layout.left+d.layout.column+0.5,"documentation piece is horizontally reachable: {}",&d.layout.preview.doc.text[piece.range.clone()]);
                }}
                assert!(d.layout.rows.iter().flat_map(|row|row.pieces.iter()).map(|piece|&d.layout.preview.doc.text[piece.range.clone()]).collect::<String>().contains("FINAL TABLE CELL MARKER"));
            });
            v.simulate_keystrokes("alt-home"); draw(&mut v); assert_eq!(f.view.read_with(&v,|r,_|r.completion_doc.as_ref().unwrap().top),0.0);
            v.simulate_keystrokes("alt-pagedown"); draw(&mut v); assert!(f.view.read_with(&v,|r,_|r.completion_doc.as_ref().unwrap().top)>0.0);
        }
    }
    v.simulate_keystrokes("down"); settle(cx,&f,|r|r.session.state().completion().is_some_and(|c|c.documentation_preview().is_some())); draw(&mut v);
    settle(cx,&f,|r|r.completion_doc.is_some()&&r.completion_doc_building.is_none()); draw(&mut v);
    assert_eq!(f.view.read_with(&v,|r,_|r.completion_doc.as_ref().unwrap().top),0.0);
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), text);
}


#[gpui::test]
fn completion_documentation_layout_does_not_block_input_and_rejects_cancelled_work(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let server=tempfile::tempdir().unwrap(); let launcher=server.path().join("server");
    std::fs::write(&launcher,format!("#!/bin/sh\nexport FAKE_LSP_COMPLETION_DOC=1\nexport FAKE_LSP_COMPLETION_DOC_LONG=1\nexec '{}' \"$@\"\n",fake_lsp().display())).unwrap();
    std::fs::set_permissions(&launcher,std::fs::Permissions::from_mode(0o755)).unwrap();
    let text="fn main() { hel }\nfn hello() {}\nfn helper() {}\n"; let f=fixture_with(cx,None,&launcher);
    let pkg=fake_package(cx,&f,&[("src/main.rs",text)],"src/main.rs"); let at=text.find("hel }").unwrap()+3;
    f.view.update(cx,|r,cx|{r.enable_draft_input_for_test();r.dispatch(Command::Select(Selection{anchor:at,head:at}),cx);r.dispatch(Command::Complete,cx);});
    settle(cx,&f,|r|r.session.state().completion().is_some_and(|c|c.documentation_preview().is_some()));
    let mut v=visual(cx,&f); v.simulate_resize(size(px(180.0),px(240.0)));
    f.view.update(&mut v,|r,_|{r.completion_doc=None;r.completion_doc_building=None;});
    let started=std::time::Instant::now();
    v.update(|window,cx|f.view.update(cx,|r,cx|{let _=r.completion_frame(window,cx);assert!(r.completion_doc.is_none(),"first frame queues shaping off the input thread");assert!(r.completion_doc_building.is_some());}));
    let feedback=started.elapsed(); assert!(feedback<std::time::Duration::from_millis(100),"first feedback blocks for {feedback:?}");
    for w in [320.0,180.0,260.0,180.0] {v.simulate_resize(size(px(w),px(240.0)));draw(&mut v);}
    f.view.update(&mut v,|r,cx|r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id("github-light").unwrap())),cx));draw(&mut v);
    v.simulate_keystrokes("escape");draw(&mut v);cx.run_until_parked();
    assert!(f.view.read_with(&v,|r,_|r.completion_doc.is_none()&&r.completion_doc_building.is_none()&&r.completion_doc_drawn.is_none()),"cancelled work cannot restore a popup");
    v.simulate_keystrokes("ctrl-space");settle(cx,&f,|r|r.session.state().completion().is_some_and(|c|c.documentation_preview().is_some()));draw(&mut v);
    let layout_started=std::time::Instant::now();settle(cx,&f,|r|r.completion_doc.is_some()&&r.completion_doc_building.is_none());draw(&mut v);
    let completed=layout_started.elapsed();assert!(completed<std::time::Duration::from_secs(5),"100 KiB narrow documentation layout: {completed:?}");
    v.simulate_keystrokes("alt-end");draw(&mut v);
    let rows=f.view.read_with(&v,|r,_|{let d=r.completion_doc.as_ref().unwrap();assert!(d.layout.width<=180.0);assert!(Arc::ptr_eq(&d.layout.palette,r.session.state().palette()));
        let visible=d.layout.visible(d.top,f32::from(d.bounds.size.height)).iter().flat_map(|row|row.pieces.iter()).map(|piece|&d.layout.preview.doc.text[piece.range.clone()]).collect::<String>();
        assert!(visible.contains("FINAL LONG DOCUMENTATION MARKER"),"full long documentation remains readable: {visible}");d.layout.rows.len()});
    eprintln!("completion-layout chars=100000 width=180 feedback_ms={:.3} completed_layout_ms={:.3} rows={rows}",feedback.as_secs_f64()*1000.0,completed.as_secs_f64()*1000.0);
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(),text);
}


#[gpui::test]
fn completion_popup_wraps_unicode_without_hidden_text_and_cancels_between_chunks(cx: &mut TestAppContext) {
    let f=fixture(cx,None);let mut v=visual(cx,&f);
    use unicode_segmentation::UnicodeSegmentation;
    let text=format!("```java\n{}e\u{301}👩🏽‍💻{}e{}\n```", "x".repeat(4095), "String café = \"office e\u{301} 👩🏽‍💻 אבג\"; ".repeat(200), "\u{301}".repeat(4096));
    let mut budget=lenz_content::markdown::Limits::DEFAULT.segments_per_pair;
    let doc=lenz_content::markdown::parse(text.into(),lenz_content::markdown::Limits::DEFAULT,&mut budget,&||true).unwrap();
    let syntax=lenz_content::markdown::code_syntax(&doc,||true);
    let preview=Arc::new(Preview{version:0,doc:Arc::new(doc),syntax:Arc::new(syntax),images:None});
    v.update(|window,cx|f.view.update(cx,|r,_|{
        let style=r.reading_style(window);let palette=r.session.state().palette().clone();
        let boundaries:Vec<_>=preview.doc.text.grapheme_indices(true).map(|(at,_)|at).chain(std::iter::once(preview.doc.text.len())).collect();
        for width in [64.0,180.0,320.0] {
            let layout=preview::Layout::popup(preview.clone(),width,style.clone(),palette.clone(),window.text_system(),&||true);
            let expected=preview.doc.blocks.iter().find(|b|matches!(b.kind,lenz_content::markdown::BlockKind::Code{..})).map(|b|preview.doc.text[b.text.clone()].trim_end_matches('\n')).unwrap();
            let actual=layout.rows.iter().flat_map(|row|row.pieces.iter()).map(|p|preview.doc.text.get(p.range.clone()).expect("all wraps keep UTF-8 boundaries")).collect::<String>();
            assert_eq!(actual,expected);
            for piece in layout.rows.iter().flat_map(|row|&row.pieces) {
                assert!(boundaries.binary_search(&piece.range.start).is_ok()&&boundaries.binary_search(&piece.range.end).is_ok(),"wrap preserves whole graphemes: {:?}",preview.doc.text.get(piece.range.clone()));
                assert!(piece.x+f32::from(piece.shaped.width)<=layout.left+layout.column+0.5||preview.doc.text[piece.range.clone()].graphemes(true).count()==1,"Unicode wrap exceeds popup: {}",&preview.doc.text[piece.range.clone()]);
            }
        }
        for limit in [30,9000] {
            let calls=std::cell::Cell::new(0);
            let partial=preview::Layout::popup(preview.clone(),180.0,style.clone(),palette.clone(),window.text_system(),&||{calls.set(calls.get()+1);calls.get()<limit});
            let bytes=partial.rows.iter().flat_map(|row|&row.pieces).map(|piece|piece.range.len()).sum::<usize>();
            assert!(bytes<4096,"cancellation stops within a bounded shaping chunk: {bytes}");
            assert!(limit==30||bytes>0,"later cancellation also interrupts after making progress");
        }
    }));
}

#[gpui::test]
fn java_member_semantic_colour_stays_painted_during_typing_and_lexical_arrival(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let server = tempfile::tempdir().unwrap();
    let launcher = server.path().join("server");
    let gate = server.path().join("token-gate");
    std::fs::write(&launcher, format!("#!/bin/sh\nexport FAKE_LSP_TOKEN_GATE='{}'\nexec '{}' \"$@\"\n", gate.display(), fake_lsp().display())).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
    let source = b"public static void main(String[] args) {\n    System.out.println(\"Hello Lenz!\");\n    \n}\n";
    let f = fixture_providers(cx, Some(("Main.java", source)), &[("jdtls", &launcher, None, None)]);
    settle(cx, &f, |r| r.session.state().active().and_then(|tab| r.session.state().view(tab.id())).is_some_and(|view| view.tokens().is_some() && view.highlights().is_some()));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    let colour = |v: &mut VisualTestContext| {
        draw(v);
        f.view.read_with(v, |r, _| {
            let rows = r.visible_rows();
            let row = rows.iter().find(|row| row.number == 2).unwrap();
            let at = row.text.find("out").unwrap();
            row.fg.iter().find(|(range, _)| range.contains(&at)).map(|(_, colour)| *colour)
        })
    };
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())), cx));
        let expected = f.view.read_with(&v, |r, _| r.session.state().palette().syntax_color(lenz_content::highlight::Capture::Property));
        assert_eq!(colour(&mut v), Some(expected));
        std::fs::write(&gate, b"hold fresh semantic responses").unwrap();
        f.view.update(&mut v, |r, cx| {
            let at = r.session.state().current_source().unwrap().line_range(2).unwrap().start + 4;
            r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx);
        });
        v.simulate_keystrokes("x");
        assert_eq!(colour(&mut v), Some(expected), "typing must keep out's actual GUI foreground");
        drop(v);
        settle(cx, &f, |r| r.session.state().highlights().is_some());
        v = visual(cx, &f);
        assert_eq!(colour(&mut v), Some(expected), "fresh syntax must keep semantic colour while tokens wait");
        std::fs::remove_file(&gate).unwrap();
        drop(v);
        settle(cx, &f, |r| r.session.state().active().and_then(|tab| r.session.state().view(tab.id())).is_some_and(|view| view.tokens().is_some()));
        v = visual(cx, &f);
    }
    assert_eq!(std::fs::read(f.dir.path().join("Main.java")).unwrap(), source);
}
