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
            views.retain(|v| !ids.contains(&v.session.id()));
            for id in ids { coordinator.unregister(id); }
            *active=(*active).min(views.len().saturating_sub(1)); true
        }
        Progress::Cancelled => { for view in views.iter_mut() { view.clamp(); } true }
        Progress::Idle => false,
    }
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
    let mut coordinator = code_core::close::CloseCoordinator::default();
    let mut input = None;
    let mut dirty = true;
    let mut pointer_text = false;
    loop {
        // A bounded batch keeps input responsive even during a fast project scan.
        for (i, view) in views.iter_mut().enumerate() {
            let completions = view.session.completions();
            for _ in 0..code_core::session::COMPLETIONS {
                let Ok(c) = completions.try_recv() else { break };
                dirty |= view.apply(c) && i == active;
            }
            let updates = view.session.theme_updates();
            while let Ok(t) = updates.try_recv() {
                dirty |= view.apply(Completion::Theme(Box::new(t))) && i == active;
            }
        }
        if dirty {
            views[active].independent_view = (views.len() > 1).then_some((active + 1, views.len()));
            terminal.draw(|frame| views[active].draw(frame))?;
            views[active].session.start_themes();
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
        if std::mem::take(&mut views[active].new_view) {
            views[active].handle(Event::FocusLost);
            let root = views[active].new_view_root.take();
            views.push(App::new(Config::from_env(true), root));
            active = views.len() - 1;
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
                    views[active].handle(Event::FocusLost);
                    active = if k.modifiers.contains(KeyModifiers::SHIFT) {
                        (active + views.len() - 1) % views.len()
                    } else {
                        (active + 1) % views.len()
                    };
                    views[active].dispatch(code_core::session::Command::Recheck);
                    dirty = true;
                    continue;
                }
                dirty |= dispatch_input(&mut views, &mut active, &mut coordinator, event);
                if views.is_empty() { return Ok(()); }
                if views[active].pointer_text != pointer_text {
                    pointer_text = views[active].pointer_text;
                    pointer(&mut io::stdout(), pointer_text)?;
                }
            }
            Wake::Completion(i, c) => dirty |= views[i].apply(c) && i == active,
            Wake::Theme(i, t) => {
                dirty |= views[i].apply(Completion::Theme(Box::new(t))) && i == active
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
                    dirty |= view.tick(now) && i == active;
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
