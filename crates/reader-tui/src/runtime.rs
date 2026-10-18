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
use reader_core::session::{Completion, Config};

use crate::app::App;

const HELP: &str = "Lenz Reader terminal
Usage: lenz-reader-tui [FILE|FOLDER]
       lenz-reader-tui --help | --version

F10 or : commands; F1 help; Ctrl-K search everywhere; Ctrl-P find file;
Ctrl-Alt-Shift-N symbols; Ctrl-Shift-A actions; Tab switches search scope;
Ctrl-O open; Ctrl-F find;
Ctrl-Shift-F project search; Ctrl-G line; F5 reload; Ctrl-F4 close tab;
Tab focus panes; Ctrl-Tab next tab; F8/Shift-F8 independent views;
Ctrl-Q close view; Ctrl-Shift-Q quit all. Files are read-only.
Optional icons: LENZ_ICONS=nerd (configure a Nerd Font Mono in your terminal);
LENZ_ICONS=plain keeps ordinary labels.";

/// An owned terminal's input modes are restored on success, I/O failure and panic.
struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
            DisableBracketedPaste,
            DisableFocusChange
        );
        ratatui::restore();
    }
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

fn run(path: Option<PathBuf>) -> io::Result<()> {
    let _restore = Restore;
    let mut terminal = ratatui::try_init()?;
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
    let mut views = vec![App::new(Config::from_env(true), path)];
    let mut active = 0usize;
    let mut input = None;
    let mut dirty = true;
    loop {
        // A bounded batch keeps input responsive even during a fast project scan.
        for (i, view) in views.iter_mut().enumerate() {
            let completions = view.session.completions();
            for _ in 0..reader_core::session::COMPLETIONS {
                let Ok(c) = completions.try_recv() else { break };
                dirty |= view.apply(c) && i == active;
            }
            let updates = view.session.theme_updates();
            while let Ok(t) = updates.try_recv() {
                dirty |= view.apply(Completion::Theme(Box::new(t))) && i == active;
            }
        }
        if dirty {
            terminal.draw(|frame| {
                views[active].draw(frame);
                if views.len() > 1 {
                    let area = frame.area();
                    if area.height > 0 {
                        let bar =
                            ratatui::layout::Rect::new(area.x, area.bottom() - 1, area.width, 1);
                        frame.render_widget(
                            ratatui::widgets::Paragraph::new(format!(
                                " View {}/{} · F8/Shift-F8 switch · Ctrl-Q closes view",
                                active + 1,
                                views.len()
                            )),
                            bar,
                        );
                    }
                }
            })?;
            views[active].session.start_themes();
            dirty = false;
        }
        if let Some(text) = views[active].copy.take() {
            clipboard(&mut io::stdout().lock(), &text)?;
        }
        if std::mem::take(&mut views[active].new_view) {
            views[active].handle(Event::FocusLost);
            views.push(App::new(Config::from_env(true), None));
            active = views.len() - 1;
            dirty = true;
            continue;
        }
        if views[active].quit {
            views.remove(active);
            if views.is_empty() {
                return Ok(());
            }
            active = active.min(views.len() - 1);
            dirty = true;
            continue;
        }
        // Drawing reports pane dimensions to the core and can schedule newly visible work.
        if dirty {
            continue;
        }
        match block_on(wait(input.get_or_insert_with(EventStream::new), &views)) {
            Wake::Input(event) => {
                let event = event?;
                if let Event::Key(k) = event
                    && k.kind == KeyEventKind::Press
                {
                    if matches!(k.code, KeyCode::Char('q' | 'Q'))
                        && k.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        if k.modifiers.contains(KeyModifiers::SHIFT) {
                            return Ok(());
                        }
                        views[active].quit = true;
                        continue;
                    }
                    if k.code == KeyCode::F(8) {
                        views[active].handle(Event::FocusLost);
                        active = if k.modifiers.contains(KeyModifiers::SHIFT) {
                            (active + views.len() - 1) % views.len()
                        } else {
                            (active + 1) % views.len()
                        };
                        views[active].dispatch(reader_core::session::Command::Recheck);
                        dirty = true;
                        continue;
                    }
                }
                dirty |= views[active].handle(event);
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
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.len() == 1 && args[0] == "--version" {
        println!("lenz-reader-tui {}", env!("CARGO_PKG_VERSION"));
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
        eprintln!("{HELP}");
        return ExitCode::from(2);
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("lenz-reader-tui needs an interactive terminal on stdin and stdout");
        return ExitCode::from(2);
    }
    let path = reader_core::session::launch_target(
        args.first().map(PathBuf::from),
        std::env::current_dir().ok(),
        std::env::var_os("HOME").map(PathBuf::from),
    );
    match run(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lenz-reader-tui: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn clipboard_cannot_inject_source_controls_into_the_terminal() {
        let mut out = Vec::new();
        super::clipboard(&mut out, "hello\x1b]52;c;bad\x07").unwrap();
        assert_eq!(out, b"\x1b]52;c;aGVsbG8bXTUyO2M7YmFkBw==\x07");
    }
}
