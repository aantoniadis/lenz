//! Owned-display acceptance tool only. No application defaults or shipped input gates change.
use gpui::{App, Entity, WindowHandle};
use reader_core::session::Config;
use reader_gui::ReaderView;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

fn config(root: &Path, dark: bool) -> Config {
    Config {
        themes: lenz_content::theme::ThemePaths::under(
            &root.join("config"),
            &root.join("state"),
            "lenz-reader",
        ),
        recent: root.join("recent.json"),
        settings: root.join("settings.json"),
        providers: root.join("providers.json"),
        cache: root.join("cache"),
        idle: Duration::ZERO,
        clock: Arc::new(Instant::now),
        overrides: Default::default(),
        dark,
    }
}
fn snapshot(view: &ReaderView) -> Value {
    let session = view.session();
    let state = session.state();
    let source = state.active().and_then(|t| state.view(t.id())).map(|view| {
        let source = view.snapshot(); let end = source.floor_char_boundary(source.len_bytes().min(4096));
        json!({"buffer": source.key().buffer.0, "version": source.key().version, "bytes": source.len_bytes(),
            "prefix": source.copy_range(0..end), "prefix_complete": end == source.len_bytes(),
            "dirty": view.dirty(), "can_undo": view.can_undo(), "can_redo": view.can_redo(),
            "selection": view.tab().selection().map(|s| [s.anchor,s.head]),
            "top_line": view.tab().top_line(), "column": view.tab().column()})
    });
    json!({"source":source,"closed":session.closed(),"close_prompt":session.close_prompt().map(|p|json!({
        "ticket":p.ticket,"buffer":p.buffer.0,"waiting":p.waiting,
        "save_disabled":p.save_disabled,"discard_disabled":p.discard_disabled}))})
}
fn record(trace: &Rc<RefCell<File>>, label: &str, value: &Value, started: Instant) {
    // This observer is acceptance instrumentation on small disposable fixtures, not a typing latency endpoint.
    let result = writeln!(
        trace.borrow_mut(),
        "{}",
        json!({"schema":1,"seconds":started.elapsed().as_secs_f64(),"label":label,"state":value})
    );
    if let Err(error) = result {
        eprintln!("native acceptance trace: {error}");
    }
}
fn observe(
    handle: WindowHandle<ReaderView>,
    label: String,
    trace: Rc<RefCell<File>>,
    started: Instant,
    cx: &mut App,
) {
    let view: Entity<ReaderView> = handle.root(cx).expect("created production window");
    let mut prior = snapshot(view.read(cx));
    record(&trace, &label, &prior, started);
    cx.observe(&view, move |view, cx| {
        let current = snapshot(view.read(cx));
        if current != prior {
            record(&trace, &label, &current, started);
            prior = current;
        }
    })
    .detach();
}
fn prepare(root: &Path) -> io::Result<()> {
    fs::create_dir(root)?; // Never reuse or overwrite an existing directory.
    for label in ["a", "b"] {
        fs::write(
            root.join(format!("{label}.txt")),
            format!("saved {label}\r\nsecond {label}\r\n"),
        )?;
        let state = root.join(format!("session-{label}"));
        fs::create_dir(&state)?;
        let providers: serde_json::Map<String, Value> = reader_core::semantics::provider::PROVIDERS
            .iter()
            .map(|p| {
                (
                    p.id.into(),
                    json!({"executable":format!("/nonexistent/{}",p.id)}),
                )
            })
            .collect();
        fs::write(
            state.join("providers.json"),
            json!({"providers":providers}).to_string(),
        )?;
    }
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: foundation-native <NEW disposable directory>".into());
    }
    let root = PathBuf::from(&args[0]);
    prepare(&root)?;
    let trace = Rc::new(RefCell::new(File::create(root.join("state.jsonl"))?));
    let started = Instant::now();
    gpui_platform::application().run(move |cx: &mut App| {
        for label in ["a", "b"] {
            let state = root.join(format!("session-{label}"));
            let make = Rc::new(move |dark| config(&state, dark));
            let handle = reader_gui::open_acceptance_window_for_test(
                cx,
                make,
                Some(root.join(format!("{label}.txt"))),
            )
            .expect("production constructor must open the acceptance window");
            handle
                .update(cx, |view, window, cx| {
                    view.enable_draft_input_for_test();
                    window.set_window_title(&format!("Lenz foundation {label}"));
                    view.focus_handle().focus(window, cx);
                    cx.notify();
                })
                .expect("new window is live");
            observe(handle, label.into(), trace.clone(), started, cx);
        }
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn acceptance_constructor_keeps_default_gate_and_installs_production_close_veto(
        cx: &mut gpui::TestAppContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("runtime");
        prepare(&root).unwrap();
        let base = root.join("session-a");
        let make = Rc::new(move |dark| config(&base, dark));
        let window = cx
            .update(|cx| {
                reader_gui::open_acceptance_window_for_test(cx, make, Some(root.join("a.txt")))
            })
            .unwrap();
        let view = window.root(cx).unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while view.read_with(&visual, |v, _| {
            v.session().state().current_source().is_none()
        }) {
            view.update(&mut visual, |v, cx| v.poll(cx));
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(view.read_with(&visual, |v, _| {
            v.session()
                .disabled(reader_core::session::CommandId::Paste)
                .unwrap()
                .contains("gated")
        }));
        view.update(&mut visual, |v, cx| {
            v.enable_draft_input_for_test();
            cx.notify();
        });
        let focus = view.read_with(&visual, |v, _| v.focus_handle());
        visual.update(|window, cx| {
            focus.focus(window, cx);
            let _ = window.draw(cx);
        });
        visual.simulate_keystrokes("ctrl-a x");
        assert!(view.read_with(&visual, |v, _| {
            v.session()
                .state()
                .view(v.session().state().active().unwrap().id())
                .unwrap()
                .dirty()
        }));
        assert!(!visual.simulate_close());
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.session().close_prompt().is_some()));
        assert_eq!(
            fs::read(root.join("a.txt")).unwrap(),
            b"saved a\r\nsecond a\r\n"
        );
    }
}
