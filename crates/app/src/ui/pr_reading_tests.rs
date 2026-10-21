use super::*;

#[gpui::test]
fn gui_pr_reading_wheel_preserves_small_pixel_and_horizontal_input(cx: &mut TestAppContext) {
    let f = pr_fixture(cx);
    f.viewer.update(cx, |v, _| v.repo.as_mut().unwrap().pr.as_mut().unwrap().pr.body = "Paragraph.\n\n".repeat(80));
    cx.simulate_window_resize(f.window.into(), size(px(1200.), px(800.)));
    cx.executor().run_until_parked();
    run(cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(crate::pr_session::DetailsTab::Conversation)));
    f.window.update(cx, |v, w, cx| {
        let _ = v.render(w, cx);
        let (x, y, _, _) = v.pr_pane();
        for delta in [point(px(-10.), px(0.)), point(px(0.), px(-0.25))] {
            assert!(v.pr_dialog_wheel(&ScrollWheelEvent { position: point(px(x + 40.), px(y + 60.)), delta: ScrollDelta::Pixels(delta), ..Default::default() }, cx));
            assert_eq!(v.pr_dialog.as_ref().unwrap().scroll, 0, "horizontal and subpixel wheel events must not jump three rows");
        }
        for (lines, expected) in [(-3.,1),(3.,0)] {
            assert!(v.pr_dialog_wheel(&ScrollWheelEvent { position:point(px(x+40.),px(y+60.)),delta:ScrollDelta::Lines(point(0.,lines)),..Default::default() },cx));
            assert_eq!(v.pr_dialog.as_ref().unwrap().scroll,expected,"one wheel event scrolls one row even when the platform reports three lines");
        }
    }).unwrap();
}

#[test]
fn gui_pr_trackpad_overscroll_does_not_delay_reversal() {
    let mut cx = TestAppContext::single();
    let f = pr_fixture(&mut cx);
    f.viewer.update(&mut cx, |v, _| v.repo.as_mut().unwrap().pr.as_mut().unwrap().pr.body = "Paragraph.\n\n".repeat(80));
    run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(crate::pr_session::DetailsTab::Conversation)));
    f.window.update(&mut cx, |v, w, cx| {
        let _ = v.render(w, cx);
        let rect = v.pr_pane();
        let max = v.pr_dialog.as_ref().unwrap().max_scroll(rect);
        assert!(max > 1);
        for (start, outward, expected) in [(max, -0.25, max - 1), (0, 0.25, 1)] {
            let d = v.pr_dialog.as_mut().unwrap();
            d.scroll = start; d.wheel = 0.;
            for delta in [outward, outward, outward, -outward, -outward, -outward, -outward] {
                v.on_wheel(&ScrollWheelEvent { position: point(px(rect.0 + 40.), px(rect.1 + 60.)), delta: ScrollDelta::Pixels(point(px(0.), px(delta * line_h()))), ..Default::default() }, w, cx);
            }
            assert_eq!(v.pr_dialog.as_ref().unwrap().scroll, expected);
        }
    }).unwrap();
}

#[test]
fn gui_pr_native_reading_reuses_markdown_typography_and_measured_targets() {
    let mut cx = TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0), None, native_text_system());
    let f = pr_fixture(&mut cx);
    let body = "# A heading\n\nWide WWW and narrow iii with [the guide](https://example.org/guide).\n\n```rust\n\tlet 界 = 1;\n```\n\n".repeat(12) + "PR_READING_TAIL";
    f.viewer.update(&mut cx, |v, _| { let pr=v.repo.as_mut().unwrap().pr.as_mut().unwrap();pr.pr.body=body;pr.seed_review(crate::pr::Review::default(), &[]).unwrap(); });
    run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(crate::pr_session::DetailsTab::Conversation)));
    for &(width, height) in &[(1200., 800.), (340., 300.), (180., 120.)] {
        cx.simulate_window_resize(f.window.into(), size(px(width), px(height)));
        cx.executor().run_until_parked();
        key(&mut cx, &f, "home", false, false, false);
        f.window.update(&mut cx, |v, w, cx| { let _=v.render(w,cx); }).unwrap();
        workflow_wait(&mut cx, &f, |v| v.pr_dialog.as_ref().is_some_and(|d| (d.reading.is_some() || d.reading_ready.is_some()) && d.reading_task.is_none()));
        f.window.update(&mut cx, |v, w, cx| {
            let _ = v.render(w, cx);
            let dialog = v.pr_dialog.as_ref().unwrap();
            let layout = dialog.reading.as_ref().unwrap();
            let heading = layout.rows.iter().flatten().find(|r| r.line.text.as_ref() == "A heading").unwrap();
            assert_eq!(f32::from(heading.line.font_size), layout.style.heading[0]);
            let prose = layout.rows.iter().flatten().find(|r| r.line.text.starts_with("Wide")).unwrap();
            assert_eq!(f32::from(prose.line.font_size), layout.style.size);
            assert!(heading.height > prose.height);
            let (index, code) = layout.rows.iter().enumerate().filter_map(|(i, r)| Some((i, r.as_ref()?))).find(|(_, r)| r.line.text.contains("let 界")).unwrap();
            assert_eq!(layout.page.rows[index].text(), code.line.text.as_ref(), "code copy spans and native glyph text preserve the same tabs");
            let row = &layout.page.rows[index];
            assert_eq!(layout.page.text((index, row.indent), (index, row.indent + crate::rich::width(&row.text()))), code.line.text.as_ref());
            assert!(layout.style.column * 1.3 > layout.style.column, "PR descriptions use a wider reading column");
            let target = crate::rich::Target::Url("https://example.org/guide".into());
            let row = dialog.page.row_of(&target).unwrap();
            let r = &dialog.page.rows[row];
            let mut column = r.indent;
            for span in &r.spans { if span.target.as_ref() == Some(&target) { break; } column += crate::rich::width(&span.text); }
            let (x, y) = dialog.point_of(row, column + 1, v.pr_pane(), v.char_w);
            let (found_row, found_column) = dialog.cell(v.pr_pane(), v.char_w, x + 0.2, y + 2.).unwrap();
            assert_eq!(dialog.page.target_at(found_row, found_column), Some(&target));
            let native = layout.rows[row].as_ref().unwrap();
            assert!(dialog.cell(v.pr_pane(), v.char_w, v.pr_pane().0 + 12. + native.x - 0.5, y + 2.).is_none(), "padding beside a native row is not a text target");
            if let Some((index, row)) = dialog.page.rows.iter().enumerate().find(|(i, row)| row.spans.iter().any(|s| s.target.as_ref() == Some(&target)) && layout.top(i + 1) > layout.top(*i) + layout.height(*i) + 2.) {
                let column = row.indent + row.spans.iter().take_while(|s| s.target.as_ref() != Some(&target)).map(|s| crate::rich::width(&s.text)).sum::<usize>() + 1;
                let (x, y) = dialog.point_of(index, column, v.pr_pane(), v.char_w);
                assert!(dialog.cell(v.pr_pane(), v.char_w, x, y + layout.height(index) + 1.).is_none(), "the blank gap above the next heading must not activate a link");
            }
        }).unwrap();
        key(&mut cx, &f, "end", false, false, false);
        f.window.update(&mut cx, |v, w, cx| {
            let _ = v.render(w, cx);
            let d = v.pr_dialog.as_ref().unwrap();
            let row = d.page.rows.iter().position(|r| r.text().contains("PR_READING_TAIL")).unwrap();
            let (_, y) = d.point_of(row, 0, v.pr_pane(), v.char_w);
            assert!(y >= v.pr_pane().1 && y < v.pr_pane().1 + v.pr_pane().3, "width={width} y={y} pane={:?} scroll={} row={} max={} rows={}",v.pr_pane(),d.scroll,row,d.max_scroll(v.pr_pane()),d.page.rows.len());
        }).unwrap();
    }
}

#[test]
#[ignore]
fn gui_pr_reading_release_perf() {
    let mut cx = TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0), None, native_text_system());
    let f = pr_fixture(&mut cx);
    let body = (0..180).map(|i| format!("## Heading {i}\n\nParagraph **bold** and [link](https://example.org/{i}) with wide WWW and narrow iii. This paragraph wraps with native font measurements and keeps its links.\n\n- item one\n- item two\n\n")).collect::<String>() + "PR_READING_TAIL";
    let bytes = body.len();
    let mut layout_ms = Vec::new();
    let mut first_frame_ms = Vec::new();
    let mut cached_ms = Vec::new();
    for _ in 0..4 {
        f.viewer.update(&mut cx, |v, _| {
            v.repo.as_mut().unwrap().pr.as_mut().unwrap().pr.body = body.clone();
            v.pr_dialog = None;
        });
        let start = std::time::Instant::now();
        run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(crate::pr_session::DetailsTab::Conversation)));
        f.window.update(&mut cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
        first_frame_ms.push(start.elapsed().as_secs_f64() * 1000.);
        workflow_wait(&mut cx, &f, |v| v.pr_dialog.as_ref().is_some_and(|d| (d.reading.is_some() || d.reading_ready.is_some()) && d.reading_task.is_none()));
        f.window.update(&mut cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
        layout_ms.push(start.elapsed().as_secs_f64() * 1000.);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            f.window.update(&mut cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
        }
        cached_ms.push(start.elapsed().as_secs_f64() * 10.);
        key(&mut cx, &f, "end", false, false, false);
        f.viewer.read_with(&cx, |v, _| assert!(v.pr_dialog.as_ref().unwrap().page.rows.iter().any(|r| r.text().contains("PR_READING_TAIL"))));
    }
    println!("LENZ_PR_READING {}", serde_json::json!({"source_bytes": bytes, "native_platform_text_system":true,"layout_ms":layout_ms,"render_preparation_ms":first_frame_ms,"cached_render_preparation_ms":cached_ms,"rss_mib":crate::perf::rss_mib()}));
}

#[gpui::test]
fn gui_pr_reading_rejects_a_superseded_layout_and_keeps_input_available(cx: &mut TestAppContext) {
    let f = pr_fixture(cx);
    run(cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(crate::pr_session::DetailsTab::Conversation)));
    let obsolete = f.window.update(cx, |v, w, cx| {
        let _ = v.render(w, cx);
        let d = v.pr_dialog.as_ref().unwrap();
        assert!(d.reading.is_none(), "native layout is not shaped synchronously in render");
        Arc::downgrade(d.reading_request.as_ref().unwrap())
    }).unwrap();
    f.window.update(cx, |v, w, cx| {
        let pr = v.repo.as_mut().unwrap().pr.as_mut().unwrap();
        pr.pr.body = "Replacement description".into();
        pr.seed_review(crate::pr::Review::default(), &[]).unwrap();
        let _ = v.render(w, cx);
        assert_eq!(obsolete.strong_count(), 0, "refresh cancels the obsolete shaping request");
    }).unwrap();
    workflow_wait(cx, &f, |v| v.pr_dialog.as_ref().is_some_and(|d| d.reading_ready.is_some() && d.reading_task.is_none()));
    f.window.update(cx, |v, w, cx| {
        let _ = v.render(w, cx);
        let text = v.pr_dialog.as_ref().unwrap().page.rows.iter().map(|r| r.text()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("Replacement description") && !text.contains("DESCRIPTION_END"));
    }).unwrap();
}

fn native_pr_reading_ready(cx: &mut TestAppContext, f: &Fixture) {
    f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
    workflow_wait(cx, f, |v| v.pr_dialog.as_ref().is_some_and(|d| (d.reading.is_some() || d.reading_ready.is_some()) && d.reading_task.is_none()));
    f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
}

#[test]
fn gui_pr_native_scroll_is_restored_after_a_tab_reflows() {
    use crate::pr_session::DetailsTab;
    let mut cx = TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0), None, native_text_system());
    let f = pr_fixture(&mut cx);
    f.viewer.update(&mut cx, |v, _| {
        let pr = v.repo.as_mut().unwrap().pr.as_mut().unwrap();
        pr.pr.body = "Paragraph.\n\n".repeat(80);
        pr.seed_review(crate::pr::Review::default(), &[]).unwrap();
    });
    cx.simulate_window_resize(f.window.into(), size(px(1200.), px(600.)));
    run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)));
    native_pr_reading_ready(&mut cx, &f);
    key(&mut cx, &f, "end", false, false, false);
    let scroll = f.viewer.read_with(&cx, |v, _| v.pr_dialog.as_ref().unwrap().scroll);
    run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Commits)));
    run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)));
    native_pr_reading_ready(&mut cx, &f);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.pr_dialog.as_ref().unwrap().scroll), scroll, "the provisional page must not reduce the saved native scroll");
    for drag in [false, true] {
        key(&mut cx, &f, "end", false, false, false);
        run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Commits)));
        run(&mut cx, &f, crate::catalog::Action::Pr(crate::catalog::PrAction::Details(DetailsTab::Conversation)));
        let chosen = f.window.update(&mut cx, |v, w, cx| {
            let _ = v.render(w, cx);
            let (x, y, _, _) = v.pr_pane();
            if drag {
                let event = MouseDownEvent { button: MouseButton::Left, position: point(px(x + 40.), px(y + 60.)), ..Default::default() };
                assert!(v.pr_dialog_click(&event, cx));
                assert!(v.pr_dialog_drag(x + 60., y + 60., cx));
                v.pr_dialog_release();
                // A second click clears the text selection before the completed layout is adopted.
                assert!(v.pr_dialog_click(&event, cx));
                v.pr_dialog_release();
            } else {
                assert!(v.pr_dialog_wheel(&ScrollWheelEvent { position: point(px(x + 40.), px(y + 60.)), delta: ScrollDelta::Lines(point(0., 1.)), ..Default::default() }, cx));
            }
            v.pr_dialog.as_ref().unwrap().scroll
        }).unwrap();
        native_pr_reading_ready(&mut cx, &f);
        assert_eq!(f.viewer.read_with(&cx, |v, _| v.pr_dialog.as_ref().unwrap().scroll), chosen, "user wheel/drag input overrides a pending restore");
    }
}

#[test]
fn gui_pr_native_thread_reveal_survives_tabs_reflow_and_home_overrides_it() {
    use crate::pr_session::PrCommand;
    let mut cx = TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0), None, native_text_system());
    let f = pr_fixture(&mut cx);
    let thread = crate::pr::Thread { path: "a.rs".into(), diff_side: Side::Right, line: Some(1), start_line: None, original_line: Some(1), is_outdated: false, is_resolved: false,
        comments: vec![crate::pr::Comment { body: "THREAD_REVEAL".into(), created_at: "2026-10-05T00:00:00Z".into(), ..Default::default() }], subject_type: String::new() };
    f.viewer.update(&mut cx, |v, _| {
        let pr = v.repo.as_mut().unwrap().pr.as_mut().unwrap();
        pr.pr.body = ("WWW ".repeat(25) + "\n\n").repeat(80);
        pr.seed_review(crate::pr::Review { placed: [(("a.rs".into(), Side::Right, 1), vec![thread.clone()])].into(), ..Default::default() }, &[]).unwrap();
    });
    cx.simulate_window_resize(f.window.into(), size(px(1200.), px(600.)));
    f.window.update(&mut cx, |v, w, cx| {
        v.pr_command(PrCommand::ShowThreads(vec![thread.clone()]), cx);
        let _ = v.render(w, cx);
        assert!(v.pr_dialog.as_ref().unwrap().reading.is_none(), "the reveal is still awaiting native formatting");
        v.pr_command(PrCommand::Show(crate::pr_session::DetailsTab::Commits), cx);
        let _ = v.render(w, cx);
        v.pr_command(PrCommand::Show(crate::pr_session::DetailsTab::Conversation), cx);
        let _ = v.render(w, cx);
    }).unwrap();
    native_pr_reading_ready(&mut cx, &f);
    f.viewer.read_with(&cx, |v, _| {
        let d = v.pr_dialog.as_ref().unwrap();
        let row = d.page.row_of(&crate::rich::Target::Line { path: "a.rs".into(), line: 1, left: false }).unwrap();
        assert_eq!(d.scroll, row.saturating_sub(2).min(d.max_scroll(v.pr_pane())), "the semantic target must be resolved against native rows");
    });
    f.viewer.update(&mut cx, |v, cx| v.pr_command(PrCommand::ShowThreads(vec![thread]), cx));
    f.window.update(&mut cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
    key(&mut cx, &f, "home", false, false, false);
    native_pr_reading_ready(&mut cx, &f);
    assert_eq!(f.viewer.read_with(&cx, |v, _| v.pr_dialog.as_ref().unwrap().scroll), 0, "a user scroll overrides the pending reveal");
}
