use std::{
    ffi::OsString,
    future::Future,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    pin::Pin,
    process::ExitCode,
    task::Poll,
};

use base64::Engine;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, EventStream, KeyCode, KeyEventKind,
        KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
};
use futures_lite::{StreamExt, future::block_on};
use code_core::session::{Completion, Config};

use crate::app::App;

const HELP: &str = "Usage: lenz --tui [FILE|FOLDER]
       lenz --tui --help | --version
lenz-code-tui takes the same arguments without the --tui prefix.

F10 or : commands; F1 help; Ctrl-K search everywhere; Ctrl-P find file;
Ctrl-Alt-Shift-N symbols; Ctrl-Shift-A actions; Tab switches search scope;
Ctrl-O open; Ctrl-F find;
Ctrl-Shift-F project search; Ctrl-G line; F5 reload; Ctrl-F4 close tab;
Tab focus panes; Ctrl-Tab next tab; F8/Shift-F8 independent views;
Ctrl-Q close view; Ctrl-Shift-Q quit all. Type to edit on supported input backends;
Ctrl-S saves the draft. Closing unsaved work asks Save, Discard or Cancel.
Icons: Nerd Font glyphs by default (configure a Nerd Font Mono in your terminal);
View > File Icons turns them off, and LENZ_ICONS=plain|nerd overrides it for one run.";

/// An owned terminal's input modes are restored on success, I/O failure and panic.
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        leave();
    }
}

/// SIGTERM or SIGHUP: give the terminal back, as quitting does, instead of leaving it in the alternate screen with
/// mouse reporting on. One idle thread; a handler can't take the stdout lock.
fn restore_on_signal() {
    use signal_hook::consts::{SIGHUP, SIGTERM};
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGTERM, SIGHUP]) else { return };
    std::thread::spawn(move || {
        if let Some(signal) = signals.forever().next() {
            // held to exit: no frame is drawn after the terminal is restored
            let _stdout = io::stdout().lock();
            leave();
            std::process::exit(128 + signal);
        }
    });
}

/// The alternate screen in raw mode, with mouse, paste, focus and full key reporting.
fn enter() -> io::Result<ratatui::DefaultTerminal> {
    let terminal = ratatui::try_init()?;
    // Destructors do not run with panic=abort: give the terminal back before the report, as lenz-tui does. Each entry
    // wraps the hook again (as Ratatui's does); only the newest layer restores, on the screen it entered.
    static RESTORED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !RESTORED.swap(true, std::sync::atomic::Ordering::Relaxed) { leave(); }
        previous(info);
    }));
    execute!(
        io::stdout(),
        EnableMouseCapture,
        EnableBracketedPaste,
        EnableFocusChange,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
        )
    )?;
    Ok(terminal)
}

/// OSC 22: the terminal's pointer shape by its CSS name (foot, kitty, Ghostty and others; the rest ignore it).
fn pointer(output: &mut impl io::Write, text: bool) -> io::Result<()> {
    write!(output, "\x1b]22;{}\x1b\\", if text { "text" } else { "default" })?;
    output.flush()
}

fn leave() {
    let _ = pointer(&mut io::stdout(), false);
    let _ = execute!(
        io::stdout(),
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange
    );
    ratatui::restore();
}

/// Runs `launch` in this terminal until it exits, then takes the terminal back. Input reading must have stopped.
fn run_in_terminal(terminal: &mut ratatui::DefaultTerminal, launch: &lenz_content::open::Launch) -> io::Result<Option<String>> {
    leave();
    let mut command = std::process::Command::new(&launch.program);
    command.args(&launch.args);
    if let Some(dir) = &launch.cwd {
        command.current_dir(dir);
    }
    let status = command.status();
    *terminal = enter()?;
    Ok(match status {
        Ok(s) if s.success() => None,
        Ok(s) => Some(format!("Lenz Diff ended: {s}")),
        Err(e) => Some(format!("Cannot start {}: {e}", launch.program)),
    })
}

fn clipboard(out: &mut impl Write, text: &str) -> io::Result<()> {
    write!(
        out,
        "\x1b]52;c;{}\x07",
        base64::engine::general_purpose::STANDARD.encode(text)
    )?;
    out.flush()
}

#[allow(clippy::large_enum_variant)] // One received completion; no per-keystroke allocation.
enum Wake {
    Input(io::Result<Event>),
    Completion(usize, Completion),
    Theme(usize, lenz_content::theme::ThemeUpdate),
    Closed,
    Tick,
}

/// Await real input or a core completion. Idle views do not poll or repaint on a timer.
async fn wait(input: &mut EventStream, views: &[App]) -> Wake {
    let mut pending: Vec<Pin<Box<dyn Future<Output = Wake> + '_>>> =
        Vec::with_capacity(1 + views.len() * 2);
    pending.push(Box::pin(async {
        input.next().await.map_or(Wake::Closed, Wake::Input)
    }));
    for (i, view) in views.iter().enumerate() {
        let completions = view.session.completions();
        pending.push(Box::pin(async move {
            match completions.recv().await {
                Ok(c) => Wake::Completion(i, c),
                Err(_) => std::future::pending().await,
            }
        }));
        let themes = view.session.theme_updates();
        pending.push(Box::pin(async move {
            match themes.recv().await {
                Ok(t) => Wake::Theme(i, t),
                Err(_) => std::future::pending().await,
            }
        }));
    }
    if let Some(deadline) = views.iter().filter_map(App::next_deadline).min() {
        pending.push(Box::pin(async move {
            async_io::Timer::at(deadline).await;
            Wake::Tick
        }));
    }
    std::future::poll_fn(|cx| {
        for task in &mut pending {
            if let Poll::Ready(wake) = task.as_mut().poll(cx) {
                return Poll::Ready(wake);
            }
        }
        Poll::Pending
    })
    .await
}

/// Inactive views can finish a clipboard barrier; validate at the terminal output sink before closing any view.
pub(crate) fn flush_clipboards(views: &mut [App], output: &mut impl std::io::Write) -> std::io::Result<()> {
    for view in views {
        if let Some(text) = view.copy.take() {
            let ticket = view.copy_ticket.take();
            if ticket.is_none_or(|ticket| view.session.clipboard_text_current(ticket)) { clipboard(output, &text)?; }
            if let Some(ticket) = ticket { view.dispatch(code_core::session::Command::ClipboardTextPublished(ticket)); }
        }
    }
    Ok(())
}

fn progress_closes(views: &mut Vec<App>, active: &mut usize, coordinator: &mut code_core::close::CloseCoordinator) -> bool {
    use code_core::close::Progress;
    if !coordinator.pending() {return false;}
    for view in views.iter_mut() {view.remember_explorer();}
    for view in views.iter() { coordinator.register(view.session.id()); }
    let result = { let mut sessions: Vec<_> = views.iter_mut().map(|v| &mut v.session).collect(); coordinator.progress(&mut sessions) };
    match result {
        Progress::Pending(id) => {
            let before=*active;
            if let Some(index)=views.iter().position(|v|v.session.id()==id) { *active=index; }
            // Changed only when the clamp brings the close question forward: while another dialog (a terminal's)
            // stays in front, reporting a change every pass would spin the loop without reading the keys.
            let hidden=|views:&Vec<App>|views.iter().filter(|view| view.session.close_prompt().is_some_and(|p| !view.modal.as_ref().is_some_and(|m| matches!(&m.kind,crate::app::Prompt::Details {kind:crate::app::Details::Close,body,..} if *body==p.text())))).count();
            let waiting=hidden(views);
            for view in views.iter_mut() { view.clamp(); }
            before!=*active || hidden(views)<waiting
        }
        Progress::Committed(ids) => {
            let kept: Vec<bool> = views.iter().map(|v| !ids.contains(&v.session.id())).collect();
            let mut set = code_core::projects::Projects::with(views.len(), *active, code_core::projects::Layout::Tabs);
            set.retain(&kept);
            views.retain(|v| !ids.contains(&v.session.id()));
            for id in ids { coordinator.unregister(id); }
            *active=set.active().min(views.len().saturating_sub(1)); true
        }
        Progress::Cancelled => { for view in views.iter_mut() { view.clamp(); } true }
        Progress::Idle => false,
    }
}
/// A view asked for a new one (New Project Tab, Open Folder in New Project Tab, New Window, a worktree): it opens
/// at the end, on its folder when one was given, and takes the keys.
fn open_new_view(views: &mut Vec<App>, active: &mut usize, make: impl FnOnce(Option<PathBuf>) -> App) -> bool {
    if !views.get_mut(*active).is_some_and(|v| std::mem::take(&mut v.new_view)) { return false; }
    views[*active].handle(Event::FocusLost);
    let root = views[*active].new_view_root.take();
    views.push(make(root));
    *active = views.len() - 1;
    true
}

/// Begin closing project `index` (its unsaved work is asked about first), as its close mark on the strip asks.
fn close_project(views: &mut Vec<App>, active: &mut usize, coordinator: &mut code_core::close::CloseCoordinator, index: usize) -> bool {
    if index >= views.len() { return false; }
    for view in views.iter() { coordinator.register(view.session.id()); }
    coordinator.begin(&mut [&mut views[index].session]);
    progress_closes(views, active, coordinator) | true
}

/// Give the keys to project `index`: the one losing them ends its pointer and modifier state, the one gaining them
/// rechecks its files against the disk, as switching with F8 always has.
fn switch_project(views: &mut [App], active: &mut usize, index: usize) -> bool {
    if index == *active || index >= views.len() { return false; }
    views[*active].handle(Event::FocusLost);
    *active = index;
    views[*active].dispatch(code_core::session::Command::Recheck);
    true
}

/// A project request a view made of the runtime (Next / Previous Project Tab, Projects Side by Side).
fn project_request(views: &mut [App], active: &mut usize, layout: &mut code_core::projects::Layout) -> bool {
    use code_core::projects::{Action, Projects};
    let Some(request) = views.get_mut(*active).and_then(|v| v.project_request.take()) else { return false };
    let mut set = Projects::with(views.len(), *active, *layout);
    match request {
        Action::Next | Action::Previous => { let to = set.step(request == Action::Next); switch_project(views, active, to); }
        Action::ToggleSideBySide => *layout = set.toggle_layout(),
        // The view acts on these itself: a new view, the folder prompt, or closing through the coordinator.
        Action::New | Action::OpenFolder | Action::Close => {}
    }
    true
}

/// Pointer input while several projects share the terminal: the strip's tabs and buttons, then the column under the
/// pointer (a press there gives it the keys), in that view's own coordinates.
fn project_pointer(views: &mut Vec<App>, active: &mut usize, layout: &mut code_core::projects::Layout, coordinator: &mut code_core::close::CloseCoordinator, geometry: &crate::projects::Geometry, mouse: crossterm::event::MouseEvent) -> bool {
    use crossterm::event::{MouseButton, MouseEventKind};
    if mouse.row == 0 {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) { return false; }
        return match geometry.strip_hit(mouse.column) {
            Some(crate::projects::StripHit::Activate(i)) => switch_project(views, active, i),
            Some(crate::projects::StripHit::Close(i)) => close_project(views, active, coordinator, i),
            Some(crate::projects::StripHit::New) => { views[*active].new_view = true; true }
            Some(crate::projects::StripHit::ToggleLayout) => {
                *layout = code_core::projects::Projects::with(views.len(), *active, *layout).toggle_layout();
                true
            }
            None => false,
        };
    }
    let Some((index, translated)) = crate::projects::route(geometry, *active, mouse) else { return false };
    let mut dirty = false;
    if index != *active && crate::projects::activates(&Event::Mouse(mouse)) { dirty |= switch_project(views, active, index); }
    if index == *active { dirty |= dispatch_input(views, active, coordinator, Event::Mouse(translated)); }
    else { dirty |= views[index].handle(Event::Mouse(translated)); }
    dirty
}

fn dispatch_input(views: &mut Vec<App>, active: &mut usize, coordinator: &mut code_core::close::CloseCoordinator, event: Event) -> bool {
    let mut dirty=views[*active].handle(event);
    let all=std::mem::take(&mut views[*active].quit_all);
    let one=std::mem::take(&mut views[*active].quit);
    if all || one {
        for view in views.iter() { coordinator.register(view.session.id()); }
        let mut sessions:Vec<_>=views.iter_mut().enumerate().filter(|(i,_)|all || *i==*active).map(|(_,v)|&mut v.session).collect();
        coordinator.begin(&mut sessions);
    }
    dirty |= progress_closes(views,active,coordinator);
    dirty
}

fn run(path: Option<PathBuf>) -> io::Result<()> {
    let _restore = Restore;
    let mut terminal = enter()?;
    restore_on_signal();
    let mut views = vec![App::new(Config::from_env(true), path)];
    let mut active = 0usize;
    let mut layout = code_core::projects::Layout::Tabs;
    let mut geometry = crate::projects::Geometry::default();
    let mut coordinator = code_core::close::CloseCoordinator::default();
    let mut input = None;
    let mut dirty = true;
    let mut pointer_text = false;
    loop {
        // A bounded batch keeps input responsive even during a fast project scan.
        // Shown: the active project, and the others Side by Side columns showed at the last draw.
        let shown = |i: usize, active: usize, geometry: &crate::projects::Geometry| i == active || geometry.columns.iter().any(|(c, _)| *c == i);
        for (i, view) in views.iter_mut().enumerate() {
            let completions = view.session.completions();
            for _ in 0..code_core::session::COMPLETIONS {
                let Ok(c) = completions.try_recv() else { break };
                dirty |= view.apply(c) && shown(i, active, &geometry);
            }
            let updates = view.session.theme_updates();
            while let Ok(t) = updates.try_recv() {
                dirty |= view.apply(Completion::Theme(Box::new(t))) && shown(i, active, &geometry);
            }
        }
        if dirty {
            let set = code_core::projects::Projects::with(views.len(), active, layout);
            layout = set.layout();
            crate::projects::sync(&mut views, &set);
            terminal.draw(|frame| crate::projects::draw(frame, &mut views, &set, &mut geometry))?;
            for (i, view) in views.iter_mut().enumerate() {
                if shown(i, active, &geometry) { view.session.start_themes(); }
            }
            dirty = false;
        }
        if let Some(launch) = views[active].foreground.take() {
            // Lenz Diff reads the keys until it quits.
            input = None;
            if let Some(message) = run_in_terminal(&mut terminal, &launch)? {
                views[active].dispatch(code_core::session::Command::Notice(message));
            }
            dirty = true;
            continue;
        }
        flush_clipboards(&mut views, &mut io::stdout().lock())?;
        if open_new_view(&mut views, &mut active, |root| App::new(Config::from_env(true), root)) {
            dirty = true;
            continue;
        }
        dirty |= progress_closes(&mut views,&mut active,&mut coordinator);
        if views.is_empty() { return Ok(()); }
        // Drawing reports pane dimensions to the core and can schedule newly visible work.
        if dirty {
            continue;
        }
        match block_on(wait(input.get_or_insert_with(EventStream::new), &views)) {
            Wake::Input(event) => {
                let event = event?;
                if let Event::Key(k) = event
                    && k.kind == KeyEventKind::Press
                    && k.code == KeyCode::F(8) && views[active].modal.is_none()
                {
                    let to = code_core::projects::Projects::with(views.len(), active, layout).step(!k.modifiers.contains(KeyModifiers::SHIFT));
                    switch_project(&mut views, &mut active, to);
                    dirty = true;
                    continue;
                }
                if views.len() > 1 {
                    match &event {
                        Event::Mouse(mouse) => {
                            dirty |= project_pointer(&mut views, &mut active, &mut layout, &mut coordinator, &geometry, *mouse);
                            if views.is_empty() { return Ok(()); }
                            continue;
                        }
                        // Every view lays itself out again, not only the one with the keys.
                        Event::Resize(..) => for (i, view) in views.iter_mut().enumerate() { if i != active { view.handle(event.clone()); } },
                        _ => {}
                    }
                }
                dirty |= dispatch_input(&mut views, &mut active, &mut coordinator, event);
                if views.is_empty() { return Ok(()); }
                dirty |= project_request(&mut views, &mut active, &mut layout);
                if views.is_empty() { return Ok(()); }
                if views[active].pointer_text != pointer_text {
                    pointer_text = views[active].pointer_text;
                    pointer(&mut io::stdout(), pointer_text)?;
                }
            }
            Wake::Completion(i, c) => dirty |= views[i].apply(c) && shown(i, active, &geometry),
            Wake::Theme(i, t) => {
                dirty |= views[i].apply(Completion::Theme(Box::new(t))) && shown(i, active, &geometry)
            }
            Wake::Closed => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "terminal input closed",
                ));
            }
            Wake::Tick => {
                let now = std::time::Instant::now();
                for (i, view) in views.iter_mut().enumerate() {
                    dirty |= view.tick(now) && shown(i, active, &geometry);
                }
            }
        }
    }
}

pub fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{} terminal\n{HELP}", code_core::chrome::APP_NAME);
        return ExitCode::SUCCESS;
    }
    if lenz_content::about::wants_version(&args) {
        println!("{}", lenz_content::about::version_line("lenz-code-tui"));
        return ExitCode::SUCCESS;
    }
    let literal = args.first().is_some_and(|a| a == "--");
    let args = if literal { &args[1..] } else { &args[..] };
    if args.len() > 1
        || (!literal
            && args
                .first()
                .is_some_and(|a| a.to_string_lossy().starts_with('-')))
    {
        eprintln!("{} terminal\n{HELP}", code_core::chrome::APP_NAME);
        return ExitCode::from(2);
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("lenz --tui needs an interactive terminal on stdin and stdout");
        return ExitCode::from(2);
    }
    let path = code_core::session::launch_target(
        args.first().map(PathBuf::from),
        std::env::current_dir().ok(),
        std::env::var_os("HOME").map(PathBuf::from),
    );
    match run(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lenz-code-tui: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn closing_an_unsaved_project_tab_asks_and_only_that_project_closes() {
        use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let d = tempfile::tempdir().unwrap();
        let mut views = Vec::new();
        for name in ["alpha", "beta"] {
            let root = d.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("a.txt"), b"saved\n").unwrap();
            let mut app = super::App::new(crate::app::tests::config(&d.path().join(format!("{name}-state"))), Some(root.join("a.txt")));
            crate::app::tests::settle(&mut app, |a| a.session.state().current_source().is_some());
            app.enable_draft_input_for_test();
            views.push(app);
        }
        let (mut active, mut layout) = (0usize, code_core::projects::Layout::Tabs);
        let mut coordinator = code_core::close::CloseCoordinator::default();
        super::dispatch_input(&mut views, &mut active, &mut coordinator, crossterm::event::Event::Paste("x".into()));
        assert!(views[0].session.project_dirty());
        let mut geometry = crate::projects::Geometry::default();
        let draw = |views: &mut Vec<super::App>, active: usize, layout: code_core::projects::Layout, geometry: &mut crate::projects::Geometry| {
            let set = code_core::projects::Projects::with(views.len(), active, layout);
            crate::projects::sync(views, &set);
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 16)).unwrap();
            terminal.draw(|f| crate::projects::draw(f, views, &set, geometry)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..16).map(|y| (0..100).map(|x| buffer[(x, y)].symbol().to_owned()).collect::<String>()).collect::<Vec<_>>().join("\n")
        };
        assert!(draw(&mut views, active, layout, &mut geometry).lines().next().unwrap().contains(" ●"), "the strip marks unsaved work");
        let close = geometry.tabs.iter().find(|(_, _, i)| *i == 0).unwrap().1;
        let press = MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: close, row: 0, modifiers: KeyModifiers::NONE };
        super::project_pointer(&mut views, &mut active, &mut layout, &mut coordinator, &geometry, press);
        assert!(draw(&mut views, active, layout, &mut geometry).contains("Unsaved changes"), "the project asks before closing");
        // Escape keeps the project and the file; asked again, Discard closes it alone.
        super::dispatch_input(&mut views, &mut active, &mut coordinator, crossterm::event::Event::Key(KeyEvent::new(super::KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(views.len(), 2);
        assert_eq!(std::fs::read(d.path().join("alpha/a.txt")).unwrap(), b"saved\n");
        draw(&mut views, active, layout, &mut geometry);
        super::project_pointer(&mut views, &mut active, &mut layout, &mut coordinator, &geometry, press);
        super::dispatch_input(&mut views, &mut active, &mut coordinator, crossterm::event::Event::Key(KeyEvent::new(super::KeyCode::Char('d'), KeyModifiers::NONE)));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while views.len() == 2 {
            assert!(std::time::Instant::now() < until, "discard did not finish");
            for view in views.iter_mut() { if let Ok(c) = view.session.completions().try_recv() { view.apply(c); } }
            super::progress_closes(&mut views, &mut active, &mut coordinator);
            std::thread::yield_now();
        }
        assert!(views[0].session.state().current_source().unwrap().source().ends_with("beta/a.txt"), "the other project remains");
        assert_eq!(std::fs::read(d.path().join("alpha/a.txt")).unwrap(), b"saved\n");
    }

    #[test]
    fn project_tabs_open_switch_go_side_by_side_and_close_through_real_input() {
        use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        use code_core::{projects::Layout, session::CommandId};
        let d = tempfile::tempdir().unwrap();
        let project = |name: &str| { let root = d.path().join(name); std::fs::create_dir_all(&root).unwrap(); std::fs::write(root.join(format!("{name}.txt")), name).unwrap(); root };
        let (alpha, beta) = (project("alpha"), project("beta"));
        let make = |root: Option<std::path::PathBuf>| {
            let state = d.path().join(format!("state-{}", root.as_ref().and_then(|r| r.file_name()).map_or("none".into(), |n| n.to_string_lossy().into_owned())));
            super::App::new(crate::app::tests::config(&state), root)
        };
        let mut views = vec![make(Some(alpha))];
        crate::app::tests::settle(&mut views[0], |a| a.session.state().project().is_some());
        let (mut active, mut layout) = (0usize, Layout::Tabs);
        let mut coordinator = code_core::close::CloseCoordinator::default();
        let mut geometry = crate::projects::Geometry::default();
        let key = |code, mods| crossterm::event::Event::Key(KeyEvent::new(code, mods));
        let screen = |views: &mut Vec<super::App>, active: usize, layout: Layout, geometry: &mut crate::projects::Geometry| {
            let set = code_core::projects::Projects::with(views.len(), active, layout);
            crate::projects::sync(views, &set);
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 16)).unwrap();
            terminal.draw(|f| crate::projects::draw(f, views, &set, geometry)).unwrap();
            let buffer = terminal.backend().buffer().clone();
            (0..16).map(|y| (0..100).map(|x| buffer[(x, y)].symbol().to_owned()).collect::<String>()).collect::<Vec<_>>()
        };
        // One project: drawn exactly as before, with no strip, and the project commands explain themselves.
        let alone = screen(&mut views, active, layout, &mut geometry);
        assert_eq!(alone.join("\n"), crate::app::tests::draw_screen(&mut views[0], 100, 16).chars().collect::<Vec<_>>().chunks(100).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("\n"));
        assert_eq!(views[0].session.disabled(CommandId::NextProjectTab), Some("Only one project is open"));
        super::dispatch_input(&mut views, &mut active, &mut coordinator, key(super::KeyCode::Char(']'), KeyModifiers::CONTROL | KeyModifiers::ALT));
        assert!(!super::project_request(&mut views, &mut active, &mut layout), "a disabled command asks nothing");
        // Open Folder in New Project Tab: the menu command's prompt, the typed path, Enter.
        views[0].run(CommandId::OpenFolderInProjectTab);
        super::dispatch_input(&mut views, &mut active, &mut coordinator, crossterm::event::Event::Paste(beta.display().to_string()));
        super::dispatch_input(&mut views, &mut active, &mut coordinator, key(super::KeyCode::Enter, KeyModifiers::NONE));
        assert!(super::open_new_view(&mut views, &mut active, make));
        assert_eq!((views.len(), active), (2, 1));
        crate::app::tests::settle(&mut views[1], |a| a.session.state().project().is_some());
        let tabs = screen(&mut views, active, layout, &mut geometry);
        assert!(tabs[0].contains("1 alpha") && tabs[0].contains("2 beta") && tabs[0].contains("Side by Side"), "{:?}", tabs[0]);
        assert_eq!(views[0].session.projects().count, 2);
        // Ctrl+Alt+[ / ] step through the projects (IntelliJ's previous / next project window).
        super::dispatch_input(&mut views, &mut active, &mut coordinator, key(super::KeyCode::Char('['), KeyModifiers::CONTROL | KeyModifiers::ALT));
        assert!(super::project_request(&mut views, &mut active, &mut layout));
        assert_eq!(active, 0);
        // The strip's layout button shows both projects side by side; a click in the right column gives it the keys.
        let toggle = geometry.toggle.start;
        let press = |column, row| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column, row, modifiers: KeyModifiers::NONE };
        assert!(super::project_pointer(&mut views, &mut active, &mut layout, &mut coordinator, &geometry, press(toggle, 0)));
        assert_eq!(layout, Layout::SideBySide);
        let side = screen(&mut views, active, layout, &mut geometry);
        assert_eq!(geometry.columns.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1]);
        let (left, right_rect) = (geometry.column_of(0).unwrap(), geometry.column_of(1).unwrap());
        let column_text = |r: ratatui::layout::Rect| side.iter().skip(1).map(|row| row.chars().skip(r.x as usize).take(r.width as usize).collect::<String>()).collect::<Vec<_>>().join("\n");
        assert!(column_text(left).contains("alpha") && !column_text(left).contains("beta"), "{}", column_text(left));
        assert!(column_text(right_rect).contains("beta") && !column_text(right_rect).contains("alpha"), "{}", column_text(right_rect));
        assert!(side[0].contains("Tabs"));
        let right = geometry.column_of(1).unwrap();
        super::project_pointer(&mut views, &mut active, &mut layout, &mut coordinator, &geometry, press(right.x + 2, right.y + 3));
        assert_eq!(active, 1);
        assert_eq!(views[0].session.projects().layout, Layout::SideBySide);
        // Tiny terminals: the strip and columns clip without panicking, and a project still gets the keys.
        for (w, h) in [(12u16, 3u16), (5, 1), (1, 1), (100, 2), (40, 30)] {
            let set = code_core::projects::Projects::with(views.len(), active, layout);
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            let mut tiny = crate::projects::Geometry::default();
            terminal.draw(|f| crate::projects::draw(f, &mut views, &set, &mut tiny)).unwrap();
            assert!(tiny.columns.len() <= 1 || w >= 2 * code_core::projects::MIN_COLUMN_CELLS as u16, "{w}x{h}: {:?}", tiny.columns);
            assert!(tiny.columns.iter().all(|(_, r)| r.right() <= w && r.bottom() <= h));
        }
        screen(&mut views, active, layout, &mut geometry);
        // The second project's close mark closes it (nothing unsaved); one project returns to the plain layout.
        let close = geometry.tabs.iter().find(|(_, _, i)| *i == 1).unwrap().1;
        assert!(super::project_pointer(&mut views, &mut active, &mut layout, &mut coordinator, &geometry, press(close, 0)));
        assert_eq!((views.len(), active), (1, 0));
        let after = screen(&mut views, active, layout, &mut geometry);
        assert!(!after[0].contains("Side by Side"), "no strip with one project");
        assert_eq!(views[0].session.projects().count, 1);
    }

    #[test]
    fn runtime_quit_all_waits_for_every_independent_draft_decision() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        use code_core::session::Command;
        for first in ['d','s'] {
            let d=tempfile::tempdir().unwrap(); let mut views=Vec::new();
            for name in ["a.txt","b.txt"] {
                let path=d.path().join(name);std::fs::write(&path,b"saved\r\n").unwrap();
                let mut app=super::App::new(crate::app::tests::config(&d.path().join(format!("{name}-state"))),None);
                app.dispatch(Command::OpenRetained(path)); crate::app::tests::settle(&mut app,|a|a.session.state().current_source().is_some());
                app.enable_draft_input_for_test();crate::app::tests::draw(&mut app,80,12);
                app.handle(crossterm::event::Event::Key(KeyEvent::new(super::KeyCode::Char('a'),KeyModifiers::CONTROL)));
                app.handle(crossterm::event::Event::Paste("draft λ\r\n".into()));
                if name=="a.txt" {
                    app.handle(crossterm::event::Event::Key(KeyEvent::new(super::KeyCode::Char('\\'),KeyModifiers::CONTROL)));
                    app.handle(crossterm::event::Event::Key(KeyEvent::new(super::KeyCode::F(10),KeyModifiers::NONE)));
                    let at=app.choices().iter().position(|row|matches!(&row.target,crate::app::ChoiceTarget::Action(code_core::session::CommandId::Unsplit))).unwrap();
                    for _ in 0..at {app.handle(crossterm::event::Event::Key(KeyEvent::new(super::KeyCode::Down,KeyModifiers::NONE)));}
                    app.handle(crossterm::event::Event::Key(KeyEvent::new(super::KeyCode::Enter,KeyModifiers::NONE)));
                    assert!(app.session.state().split().is_none());assert_eq!(app.session.state().tabs().len(),1);assert!(app.session.state().tab_dirty(app.session.state().active().unwrap().id()));assert!(app.modal.is_none());
                }
                views.push(app);
            }
            let positions:Vec<_>=views.iter().map(|v|(v.session.id(),v.session.state().selection(),v.session.state().active().unwrap().top_line())).collect();
            let mut active=0; let mut coordinator=code_core::close::CloseCoordinator::default();
            let event=|code,mods|crossterm::event::Event::Key(KeyEvent::new(code,mods));
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Char('q'),KeyModifiers::CONTROL|KeyModifiers::SHIFT));
            assert_eq!(views.len(),2,"Quit All must prepare every session before removing either view");
            assert!(crate::app::tests::draw_screen(&mut views[active],80,12).contains("Unsaved changes"));
            // Repeated quit, paste and Save shortcuts cannot escape the close decision.
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Char('q'),KeyModifiers::CONTROL|KeyModifiers::SHIFT));
            super::dispatch_input(&mut views,&mut active,&mut coordinator,crossterm::event::Event::Paste("ignored".into()));
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Char('s'),KeyModifiers::CONTROL));
            assert_eq!(std::fs::read(d.path().join("a.txt")).unwrap(),b"saved\r\n");
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Char(first),KeyModifiers::NONE));
            let until=std::time::Instant::now()+std::time::Duration::from_secs(10);
            while active==0 {
                assert!(std::time::Instant::now()<until,"close Save did not finish");
                if let Ok(c)=views[0].session.completions().try_recv() { views[0].apply(c); }
                super::progress_closes(&mut views,&mut active,&mut coordinator);
                std::thread::yield_now();
            }

            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::End,KeyModifiers::NONE));
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Up,KeyModifiers::NONE));
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Esc,KeyModifiers::NONE));
            assert_eq!(views.len(),2);assert!(!coordinator.pending());
            for (i,v) in views.iter().enumerate() {
                let source=v.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(),"draft λ\r\n");
                assert_eq!((v.session.id(),v.session.state().selection(),v.session.state().active().unwrap().top_line()),positions[i]);
                assert!(!v.session.closed());assert!(v.modal.is_none());
            }
            assert_eq!(std::fs::read(d.path().join("a.txt")).unwrap(),if first=='s' {"draft λ\r\n".as_bytes()} else {b"saved\r\n"});
            assert_eq!(std::fs::read(d.path().join("b.txt")).unwrap(),b"saved\r\n");
            // A separate tiny/scrolled close decision dismisses safely without comparing resize-clamped positions.
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Char('q'),KeyModifiers::CONTROL));
            crate::app::tests::draw(&mut views[active],1,1);crate::app::tests::draw(&mut views[active],18,5);
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::End,KeyModifiers::NONE));
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Up,KeyModifiers::NONE));
            super::dispatch_input(&mut views,&mut active,&mut coordinator,event(super::KeyCode::Esc,KeyModifiers::NONE));
            assert_eq!(views.len(),2);assert!(!coordinator.pending());

        }
    }
    #[test]
    fn closing_with_a_running_terminal_and_a_draft_waits_for_input_and_asks_both() {
        use crossterm::event::{Event,KeyCode,KeyEvent,KeyModifiers};
        use code_core::session::Command;
        let d=tempfile::tempdir().unwrap();let path=d.path().join("a.txt");std::fs::write(&path,b"saved\n").unwrap();
        let mut app=super::App::new(crate::app::tests::config(&d.path().join("state")),None);
        app.dispatch(Command::OpenRetained(path.clone()));crate::app::tests::settle(&mut app,|a|a.session.state().current_source().is_some());
        app.enable_draft_input_for_test();crate::app::tests::draw(&mut app,100,30);
        app.handle(Event::Paste("draft ".into()));
        app.dispatch(Command::Terminal(code_core::terminal::Command::New));
        crate::app::tests::settle(&mut app,|a|a.session.state().terminal().active().is_some_and(|t|t.running()));
        app.dispatch(Command::Terminal(code_core::terminal::Command::Focus(false)));
        let mut views=vec![app];let mut active=0;let mut coordinator=code_core::close::CloseCoordinator::default();
        let key=|code,mods|Event::Key(KeyEvent::new(code,mods));
        super::dispatch_input(&mut views,&mut active,&mut coordinator,key(KeyCode::Char('q'),KeyModifiers::CONTROL));
        assert!(crate::app::tests::draw_screen(&mut views[0],100,30).contains("Close with running terminals?"));
        // The loop blocks for input: nothing is left to redraw while the question waits.
        assert!(!super::progress_closes(&mut views,&mut active,&mut coordinator),"the run loop would spin without reading keys");
        super::dispatch_input(&mut views,&mut active,&mut coordinator,key(KeyCode::Enter,KeyModifiers::NONE));
        assert!(crate::app::tests::draw_screen(&mut views[0],100,30).contains("Unsaved changes"));
        assert!(!super::progress_closes(&mut views,&mut active,&mut coordinator));
        super::dispatch_input(&mut views,&mut active,&mut coordinator,key(KeyCode::Char('d'),KeyModifiers::NONE));
        let until=std::time::Instant::now()+std::time::Duration::from_secs(10);
        while !views.is_empty() {
            assert!(std::time::Instant::now()<until,"the view did not close");
            while let Ok(c)=views[0].session.completions().try_recv() { views[0].apply(c); }
            super::progress_closes(&mut views,&mut active,&mut coordinator);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(std::fs::read(&path).unwrap(),b"saved\n");
        // Discarded at quit: its recovery record is gone, so the next start offers nothing.
        fn records(dir:&std::path::Path,found:&mut Vec<std::path::PathBuf>) { for e in std::fs::read_dir(dir).into_iter().flatten().flatten() { let p=e.path(); if p.is_dir() { records(&p,found); } else if p.parent().is_some_and(|d|d.ends_with("recovery")) && p.extension().is_some_and(|x|x=="json") && !p.ends_with("tombstones.json") { found.push(p); } } }
        let mut found=Vec::new(); records(&d.path().join("state"),&mut found);
        assert!(found.is_empty(),"{found:?}");
    }
    #[test]
    fn runtime_empty_view_close_respects_modal_priority_and_tiny_dismissal() {
        use crossterm::event::{Event,KeyCode,KeyEvent,KeyModifiers};
        let d=tempfile::tempdir().unwrap();let mut views=vec![super::App::new(crate::app::tests::config(d.path()),None)];let mut active=0;let mut c=code_core::close::CloseCoordinator::default();
        let key=|code,mods|Event::Key(KeyEvent::new(code,mods));
        for id in [code_core::session::CommandId::Undo,code_core::session::CommandId::Reload] {views[0].run(id);}
        super::dispatch_input(&mut views,&mut active,&mut c,key(KeyCode::F(1),KeyModifiers::NONE));
        super::dispatch_input(&mut views,&mut active,&mut c,key(KeyCode::Char('q'),KeyModifiers::CONTROL));assert_eq!(views.len(),1);assert!(views[0].modal.is_some());
        crate::app::tests::draw(&mut views[0],1,1);super::dispatch_input(&mut views,&mut active,&mut c,key(KeyCode::End,KeyModifiers::NONE));super::dispatch_input(&mut views,&mut active,&mut c,key(KeyCode::Up,KeyModifiers::NONE));
        super::dispatch_input(&mut views,&mut active,&mut c,key(KeyCode::Esc,KeyModifiers::NONE));assert!(views[0].modal.is_none());
        super::dispatch_input(&mut views,&mut active,&mut c,key(KeyCode::Char('q'),KeyModifiers::CONTROL));assert!(views.is_empty());
    }
    #[test]
    fn clipboard_cannot_inject_source_controls_into_the_terminal() {
        let mut out = Vec::new();
        super::clipboard(&mut out, "hello\x1b]52;c;bad\x07").unwrap();
        assert_eq!(out, b"\x1b]52;c;aGVsbG8bXTUyO2M7YmFkBw==\x07");
    }
}
