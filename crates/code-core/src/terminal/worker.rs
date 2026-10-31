use super::{Command, Frame, Id};
use crate::session;
use std::{ffi::OsString, path::PathBuf, sync::Arc};

pub struct Update {
    pub frame: Option<Arc<Frame>>,
    pub running: bool,
    pub status: String,
    pub error: Option<String>,
    pub copy: Option<String>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix::Process;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[derive(Debug)]
pub struct Process;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl Process {
    pub(crate) fn spawn(
        _id: Id,
        _generation: u64,
        _cwd: PathBuf,
        _shell: OsString,
        _tx: async_channel::Sender<session::Completion>,
        _colors: (u32, u32),
    ) -> Result<Self, String> {
        Err("The integrated terminal is supported on Linux and macOS; this platform's process lifecycle is unverified".into())
    }
    pub(crate) fn send(&self, _command: Command) -> Result<(), String> {
        Err("Terminal is unavailable".into())
    }
    pub(crate) fn stop(&self) {}
    pub(crate) fn finished(&self) -> bool {
        true
    }
    pub(crate) fn take(&self) -> Update {
        Update {
            frame: None,
            running: false,
            status: "Unavailable".into(),
            error: None,
            copy: None,
        }
    }
}

/// The terminal's reactor on Unix. What the platforms do differently sits in `os`: following the shell's exit,
/// finding and signalling the processes of its session, and waiting on the descriptors.
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::super::{encode_key, encode_paste, model::Model, quote_paths};
    use super::*;
    use alacritty_terminal::term::TermMode;
    use filedescriptor::FileDescriptor;
    use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
    use rustix::{
        fs::{OFlags, fcntl_getfl, fcntl_setfl},
        process::{Pid, Signal, kill_process},
    };
    use std::{
        collections::VecDeque,
        io::{self, Read, Write},
        os::{
            fd::AsRawFd,
            unix::net::UnixStream,
        },
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicU32, Ordering},
        },
        time::{Duration, Instant},
    };

    const INPUT_LIMIT: usize = 2 * 1024 * 1024;
    const COMMAND_LIMIT: usize = 256;
    const FRAME_INTERVAL: Duration = Duration::from_millis(16);

    enum Input {
        Bytes(Vec<u8>),
        Model(Command),
    }
    #[derive(Default)]
    struct Queue {
        commands: VecDeque<Input>,
        bytes: usize,
        slots: usize,
    }
    struct Latest {
        update: Update,
        notified: bool,
        pending: bool,
    }
    struct Shared {
        queue: Mutex<Queue>,
        latest: Mutex<Latest>,
        cancel: AtomicBool,
        modes: AtomicU32,
        running: AtomicBool,
        finished: AtomicBool,
        wake: UnixStream,
        id: Id,
        generation: u64,
        tx: async_channel::Sender<session::Completion>,
    }
    impl Shared {
        fn wake(&self) {
            let _ = (&self.wake).write(&[1]);
        }
        fn publish(
            &self,
            frame: Option<Arc<Frame>>,
            running: bool,
            status: &str,
            error: Option<String>,
            copy: Option<String>,
        ) {
            let mut latest = self
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(frame) = frame {
                latest.update.frame = Some(frame);
            }
            latest.update.running = running;
            latest.update.status = status.into();
            if error.is_some() {
                latest.update.error = error;
            }
            if copy.is_some() {
                latest.update.copy = copy;
            }
            if !latest.notified {
                latest.pending = true;
            }
            Self::notify(&mut latest, self);
        }
        fn notify(latest: &mut Latest, shared: &Self) {
            if !latest.pending || latest.notified {
                return;
            }
            match shared.tx.try_send(session::Completion::TerminalWake {
                id: shared.id,
                generation: shared.generation,
            }) {
                Ok(()) => {
                    latest.notified = true;
                    latest.pending = false;
                }
                Err(async_channel::TrySendError::Full(_)) => {}
                Err(async_channel::TrySendError::Closed(_)) => {
                    latest.pending = false;
                    shared.cancel.store(true, Ordering::Release);
                    shared.wake();
                }
            }
        }
        fn retry_wake(&self) -> bool {
            let mut latest = self
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Self::notify(&mut latest, self);
            latest.pending
        }
    }

    pub struct Process {
        shared: Arc<Shared>,
    }
    impl std::fmt::Debug for Process {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TerminalProcess")
                .field("id", &self.shared.id)
                .field("generation", &self.shared.generation)
                .finish_non_exhaustive()
        }
    }
    impl Process {
        pub(crate) fn spawn(
            id: Id,
            generation: u64,
            cwd: PathBuf,
            shell: OsString,
            tx: async_channel::Sender<session::Completion>,
            colors: (u32, u32),
        ) -> Result<Self, String> {
            // Reject an unsupported platform (and on Linux reserve a pidfd slot) before launching a shell.
            let preflight = os::preflight()?;
            let (wake, reader) = UnixStream::pair().map_err(|e| e.to_string())?;
            wake.set_nonblocking(true).map_err(|e| e.to_string())?;
            reader.set_nonblocking(true).map_err(|e| e.to_string())?;
            let pair = portable_pty::native_pty_system()
                .openpty(PtySize::default())
                .map_err(|e| e.to_string())?;
            struct Borrowed(i32);
            impl AsRawFd for Borrowed {
                fn as_raw_fd(&self) -> i32 {
                    self.0
                }
            }
            let fd = FileDescriptor::dup(&Borrowed(
                pair.master
                    .as_raw_fd()
                    .ok_or("PTY has no native descriptor")?,
            ))
            .map_err(|e| e.to_string())?;
            fcntl_setfl(
                &fd,
                fcntl_getfl(&fd).map_err(|e| e.to_string())? | OFlags::NONBLOCK,
            )
            .map_err(|e| e.to_string())?;
            let mut command = CommandBuilder::new(shell);
            command.arg("-i");
            command.cwd(cwd);
            command.env("TERM", "xterm-256color");
            command.env("COLORTERM", "truecolor");
            command.env("TERM_PROGRAM", "Lenz");
            let child = pair
                .slave
                .spawn_command(command)
                .map_err(|e| e.to_string())?;
            drop(pair.slave);
            let pid = child
                .process_id()
                .and_then(|pid| i32::try_from(pid).ok())
                .and_then(Pid::from_raw)
                .ok_or("PTY child has no process identity")?;
            drop(preflight);
            let exit = match os::Exit::new(pid) {
                Ok(exit) => exit,
                Err(error) => {
                    kill_failed_start(child, pid);
                    return Err(error);
                }
            };
            let owned = OwnedChild {
                child: Some(child),
                exit,
                pid,
                cleaned: false,
                pending: Vec::new(),
            };
            let shared = Arc::new(Shared {
                queue: Mutex::default(),
                latest: Mutex::new(Latest {
                    update: Update {
                        frame: None,
                        running: true,
                        status: "Running".into(),
                        error: None,
                        copy: None,
                    },
                    notified: false,
                    pending: false,
                }),
                cancel: AtomicBool::new(false),
                modes: AtomicU32::new(0),
                running: AtomicBool::new(true),
                finished: AtomicBool::new(false),
                wake,
                id,
                generation,
                tx,
            });
            let worker = shared.clone();
            std::thread::Builder::new()
                .name(format!("terminal-{}", id.0))
                .spawn(move || reactor(worker, reader, fd, pair.master, owned, colors))
                .map_err(|e| e.to_string())?;
            Ok(Self { shared })
        }
        pub(crate) fn send(&self, command: Command) -> Result<(), String> {
            if matches!(&command, Command::Search { query, .. } if query.len() > 1024 || query.chars().any(char::is_control)) {
                return Err("Terminal find accepts a single line up to 1024 bytes".into());
            }
            let mode = TermMode::from_bits_retain(self.shared.modes.load(Ordering::Acquire));
            let input = match command {
                Command::Text(text) => Input::Bytes(text.into_bytes()),
                Command::Paste(text) => Input::Bytes(encode_paste(
                    &text,
                    mode.contains(TermMode::BRACKETED_PASTE),
                )?),
                Command::Paths(paths) => Input::Bytes(encode_paste(
                    &quote_paths(&paths)?,
                    mode.contains(TermMode::BRACKETED_PASTE),
                )?),
                Command::Key(key) => Input::Bytes(
                    encode_key(
                        &key,
                        mode.contains(TermMode::APP_CURSOR),
                        mode.contains(TermMode::APP_KEYPAD),
                    )
                    .ok_or("This terminal key is not supported")?,
                ),
                command @ (Command::Resize { .. }
                | Command::Scroll { .. }
                | Command::ScrollTo { .. }
                | Command::Select { .. }
                | Command::Copy
                | Command::Search { .. }
                | Command::Mouse { .. }
                | Command::Focus(_)) => Input::Model(command),
                _ => return Err("This command is managed by the terminal session".into()),
            };
            if matches!(&input, Input::Bytes(bytes) if bytes.is_empty()) {
                return Ok(());
            }
            let mut queue = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Input::Bytes(bytes) = &input {
                if !self.shared.running.load(Ordering::Acquire) {
                    return Err("Terminal has exited; restart it to send input".into());
                }
                if bytes.len() > INPUT_LIMIT.saturating_sub(queue.bytes) {
                    return Err(
                        "Terminal input is full (2 MiB); wait for the child to read it".into(),
                    );
                }
                queue.bytes += bytes.len();
            }
            if queue.slots >= COMMAND_LIMIT {
                if let Input::Bytes(bytes) = &input {
                    queue.bytes -= bytes.len();
                }
                return Err("Terminal command queue is full; wait for the child".into());
            }
            queue.slots += 1;
            queue.commands.push_back(input);
            drop(queue);
            self.shared.wake();
            Ok(())
        }
        pub(crate) fn stop(&self) {
            self.shared.running.store(false, Ordering::Release);
            self.shared.cancel.store(true, Ordering::Release);
            self.shared.wake();
        }
        pub(crate) fn finished(&self) -> bool {
            self.shared.finished.load(Ordering::Acquire)
        }
        pub(crate) fn take(&self) -> Update {
            let mut latest = self
                .shared
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            latest.notified = false;
            latest.pending = false;
            Update {
                frame: latest.update.frame.clone(),
                running: latest.update.running,
                status: latest.update.status.clone(),
                error: latest.update.error.take(),
                copy: latest.update.copy.take(),
            }
        }
    }
    impl Drop for Process {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn reap_later(mut child: Box<dyn Child + Send + Sync>) {
        let _ = std::thread::Builder::new()
            .name("terminal-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
    }
    fn kill_failed_start(mut child: Box<dyn Child + Send + Sync>, pid: Pid) {
        // The unreaped child pins this PID; SIGKILL cannot target a recycled process.
        let _ = kill_process(pid, Signal::KILL);
        if !matches!(child.try_wait(), Ok(Some(_))) {
            reap_later(child);
        }
    }
    #[cfg(all(test, target_os = "linux"))]
    #[test]
    fn terminal_pty_failed_start_kills_hup_ignoring_child() {
        use rustix::{event::{PollFd, PollFlags, Timespec, poll}, process::{PidfdFlags, pidfd_open}};
        use std::fs;
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize::default())
            .unwrap();
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "trap '' HUP; printf ready; exec sleep 60"]);
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let pid = Pid::from_raw(child.process_id().unwrap() as i32).unwrap();
        let fd = pidfd_open(pid, PidfdFlags::empty()).unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut ready = [0; 5];
        reader.read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"ready");
        let start = Instant::now();
        kill_failed_start(child, pid);
        assert!(start.elapsed() < Duration::from_millis(100));
        let mut descriptors = [PollFd::new(&fd, PollFlags::IN)];
        assert_eq!(
            poll(
                &mut descriptors,
                Some(&Timespec {
                    tv_sec: 5,
                    tv_nsec: 0
                })
            )
            .unwrap(),
            1
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::metadata(format!("/proc/{}", pid.as_raw_pid())).is_ok() {
            assert!(
                Instant::now() < deadline,
                "Failed-start child was not reaped"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    struct Pending {
        bytes: Vec<u8>,
        offset: usize,
        admitted: bool,
    }
    struct OwnedChild {
        child: Option<Box<dyn Child + Send + Sync>>,
        exit: os::Exit,
        pid: Pid,
        cleaned: bool,
        pending: Vec<os::Member>,
    }
    impl OwnedChild {
        fn ready(&self) -> bool {
            self.exit.exited()
        }
        fn cleanup(&mut self) -> String {
            if self.cleaned {
                return "Exited".into();
            }
            // The unreaped leader pins the SID while the session's members are found.
            for (signal, wait) in [(Signal::HUP, 80), (Signal::TERM, 120), (Signal::KILL, 300)] {
                self.exit.signal(signal);
                let deadline = Instant::now() + Duration::from_millis(wait);
                loop {
                    let members = os::members(self.pid);
                    if members.is_empty() {
                        break;
                    }
                    for member in &members {
                        member.signal(signal);
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            self.cleaned = true;
            self.pending = os::members(self.pid);
            if self.ready() && self.pending.is_empty() {
                match self.child.as_mut().map(|child| child.wait()) {
                    Some(Ok(status)) => format!("Exited: {status}"),
                    Some(Err(error)) => format!("Exit status unavailable: {error}"),
                    None => "Exited".into(),
                }
            } else {
                // The reactor waits off the UI thread before acknowledging final closure.
                "Stopped; kernel process exit is still pending".into()
            }
        }
    }
    impl OwnedChild {
        fn reap(&mut self) -> Option<io::Result<portable_pty::ExitStatus>> {
            // SIGKILL may be pending on an uninterruptible task; closure waits off the UI thread.
            for member in self.pending.drain(..) {
                member.wait();
            }
            self.child.take().map(|mut child| child.wait())
        }
    }
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if !self.cleaned {
                self.cleanup();
            }
            // Also cover OS thread creation failure before a reactor owns the child.
            self.reap();
        }
    }

    fn response(writes: &mut VecDeque<Pending>, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        let pending: usize = writes
            .iter()
            .filter(|p| !p.admitted)
            .map(|p| p.bytes.len() - p.offset)
            .sum();
        if writes.len() >= 512 || pending + bytes.len() > 64 * 1024 {
            return Err(
                "Terminal protocol response queue is full; the child is not reading input".into(),
            );
        }
        writes.push_back(Pending {
            bytes,
            offset: 0,
            admitted: false,
        });
        Ok(())
    }

    fn reactor(
        shared: Arc<Shared>,
        mut wake: UnixStream,
        mut fd: FileDescriptor,
        master: Box<dyn MasterPty + Send>,
        mut child: OwnedChild,
        colors: (u32, u32),
    ) {
        let mut model = Model::new(colors);
        let mut writes: VecDeque<Pending> = VecDeque::new();
        let mut running = true;
        let mut status = String::from("Running");
        let mut dirty = true;
        let mut last_frame = Instant::now() - FRAME_INTERVAL;
        let mut read_open = true;
        let mut failure = None;
        'reactor: loop {
            if shared.cancel.load(Ordering::Acquire) {
                break;
            }
            let commands = {
                let mut queue = shared
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut queue.commands)
            };
            for input in commands {
                match input {
                    Input::Bytes(bytes) => writes.push_back(Pending {
                        bytes,
                        offset: 0,
                        admitted: true,
                    }),
                    Input::Model(command) => {
                        shared
                            .queue
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .slots -= 1;
                        let mut reply = Vec::new();
                        match command {
                            Command::Resize { columns, rows } => {
                                let (columns, rows) = model.resize(columns, rows);
                                if running
                                    && let Err(error) = master.resize(PtySize {
                                        rows: rows as u16,
                                        cols: columns as u16,
                                        pixel_width: 0,
                                        pixel_height: 0,
                                    })
                                {
                                    shared.publish(
                                        None,
                                        running,
                                        &status,
                                        Some(format!("Terminal resize failed: {error}")),
                                        None,
                                    );
                                }
                            }
                            Command::Scroll { lines, local } => {
                                let mode = TermMode::from_bits_retain(model.modes());
                                if !local
                                    && mode
                                        .contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
                                {
                                    for _ in 0..lines.unsigned_abs().min(256) {
                                        reply.extend_from_slice(if lines > 0 {
                                            if model.app_cursor() {
                                                b"\x1bOA"
                                            } else {
                                                b"\x1b[A"
                                            }
                                        } else if model.app_cursor() {
                                            b"\x1bOB"
                                        } else {
                                            b"\x1b[B"
                                        });
                                    }
                                } else {
                                    model.scroll(lines);
                                }
                            }
                            Command::ScrollTo { top } => model.scroll_to(top),
                            Command::Search { query, backwards, advance } => model.search(&query, backwards, advance),
                            Command::Select {
                                row,
                                column,
                                extend,
                            } => model.select(row, column, extend),
                            Command::Copy => {
                                shared.publish(None, running, &status, None, model.copy())
                            }
                            Command::Mouse {
                                row,
                                column,
                                kind,
                                ctrl,
                                alt,
                                shift,
                            } => reply = model.mouse(row, column, kind, ctrl, alt, shift),
                            Command::Focus(focused) => {
                                if TermMode::from_bits_retain(model.modes())
                                    .contains(TermMode::FOCUS_IN_OUT)
                                {
                                    reply.extend_from_slice(if focused {
                                        b"\x1b[I"
                                    } else {
                                        b"\x1b[O"
                                    });
                                }
                            }
                            _ => {
                                failure = Some("Unsupported terminal worker command".into());
                                break 'reactor;
                            }
                        }
                        if running && let Err(error) = response(&mut writes, reply) {
                            failure = Some(error);
                            break 'reactor;
                        }
                        dirty = true;
                    }
                }
            }
            if dirty && last_frame.elapsed() >= FRAME_INTERVAL {
                shared.modes.store(model.modes(), Ordering::Release);
                shared.publish(Some(model.frame()), running, &status, None, None);
                dirty = false;
                last_frame = Instant::now();
            }
            let retry = shared.retry_wake();
            let timeout = (dirty || retry).then(|| {
                if dirty {
                    FRAME_INTERVAL.saturating_sub(last_frame.elapsed())
                } else {
                    FRAME_INTERVAL
                }
            });
            let pty = (running && read_open).then_some((&fd, !writes.is_empty()));
            let ready = match os::wait(&wake, pty, running.then_some(&child.exit), timeout) {
                Ok(ready) => ready,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    failure = Some(format!("Terminal wait failed: {error}"));
                    break;
                }
            };
            if ready.wake {
                let mut bytes = [0; 256];
                while wake.read(&mut bytes).is_ok_and(|n| n != 0) {}
            }
            if shared.cancel.load(Ordering::Acquire) {
                break;
            }
            if running && read_open && ready.pty_read {
                let mut bytes = [0; 4096];
                // A bounded turn keeps input, cancellation and presentation responsive under floods.
                for _ in 0..16 {
                    match fd.read(&mut bytes) {
                        Ok(0) => {
                            read_open = false;
                            break;
                        }
                        Ok(count) => {
                            model.advance(&bytes[..count]);
                            shared.modes.store(model.modes(), Ordering::Release);
                            if let Err(error) = response(&mut writes, model.responses()) {
                                failure = Some(error);
                                break 'reactor;
                            }
                            dirty = true;
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) if error.raw_os_error() == Some(5) => {
                            read_open = false;
                            break;
                        }
                        Err(error) => {
                            failure = Some(format!("Terminal read failed: {error}"));
                            break 'reactor;
                        }
                    }
                }
            }
            if running && ready.pty_write {
                for _ in 0..16 {
                    let Some(pending) = writes.front_mut() else {
                        break;
                    };
                    match fd.write(&pending.bytes[pending.offset..]) {
                        Ok(0) => break,
                        Ok(count) => {
                            pending.offset += count;
                            if pending.admitted {
                                let mut queue = shared
                                    .queue
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                queue.bytes -= count;
                                if pending.offset == pending.bytes.len() {
                                    queue.slots -= 1;
                                }
                            }
                            if pending.offset == pending.bytes.len() {
                                writes.pop_front();
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            failure = Some(format!("Terminal write failed: {error}"));
                            break 'reactor;
                        }
                    }
                }
            }
            if ready.child {
                shared.running.store(false, Ordering::Release);
                status = child.cleanup();
                running = false;
                writes.clear();
                // Queued commands are counted until consumed; discard bytes atomically at exit.
                let mut queue = shared
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                queue
                    .commands
                    .retain(|input| matches!(input, Input::Model(_)));
                queue.slots = queue.commands.len();
                queue.bytes = 0;
                drop(queue);
                shared.modes.store(model.modes(), Ordering::Release);
                shared.publish(Some(model.frame()), false, &status, None, None);
                dirty = false;
            }
        }
        shared.running.store(false, Ordering::Release);
        if running {
            status = child.cleanup();
        }
        let frame = model.frame();
        drop(model);
        drop(fd);
        drop(master);
        // Do not let final application closure race a detached cleanup/reaper thread.
        if !child.ready() || !child.pending.is_empty() {
            shared.publish(Some(frame.clone()), false, &status, None, None);
        }
        if let Some(Ok(exit)) = child.reap() {
            status = format!("Exited: {exit}");
        }
        drop(child);
        shared.finished.store(true, Ordering::Release);
        shared.publish(Some(frame), false, &status, failure, None);
        // Final acknowledgement survives a full inbox, including after stop() cancellation.
        while shared.retry_wake() {
            let _ = os::wait(&wake, None, None, Some(FRAME_INTERVAL));
            let mut bytes = [0; 256];
            while wake.read(&mut bytes).is_ok_and(|n| n != 0) {}
        }
    }

    /// What one wait found ready: the wake socket, the PTY for reading (or hung up) and writing, the shell's exit.
    #[derive(Default)]
    struct Ready {
        wake: bool,
        pty_read: bool,
        pty_write: bool,
        child: bool,
    }

    #[cfg(target_os = "linux")]
    mod os {
        use super::{FileDescriptor, Ready};
        use rustix::{
            event::{PollFd, PollFlags, Timespec, poll},
            process::{Pid, PidfdFlags, Signal, getpid, pidfd_open, pidfd_send_signal},
        };
        use std::{fs, io, os::{fd::OwnedFd, unix::net::UnixStream}, time::Duration};

        /// /proc for the session's members, and one pidfd slot reserved (an unsupported kernel refused) before a
        /// shell starts.
        pub(super) fn preflight() -> Result<Option<OwnedFd>, String> {
            fs::read_dir("/proc").map_err(|error| format!("The terminal needs Linux /proc for owned process cleanup: {error}"))?;
            pidfd_open(getpid(), PidfdFlags::empty()).map(Some).map_err(|error| format!("Linux pidfd support is required for terminal ownership: {error}"))
        }

        /// The shell's pidfd: readable once it exits, and a signal target that cannot reach a recycled PID.
        pub(super) struct Exit(OwnedFd);
        impl Exit {
            pub(super) fn new(pid: Pid) -> Result<Self, String> {
                pidfd_open(pid, PidfdFlags::empty()).map(Self).map_err(|error| format!("Linux pidfd support is required for terminal ownership: {error}"))
            }
            pub(super) fn signal(&self, signal: Signal) {
                let _ = pidfd_send_signal(&self.0, signal);
            }
            pub(super) fn exited(&self) -> bool {
                let mut descriptors = [PollFd::new(&self.0, PollFlags::IN)];
                poll(&mut descriptors, Some(&Timespec { tv_sec: 0, tv_nsec: 0 })).is_ok_and(|n| n != 0)
            }
        }

        /// A live process of the shell's session, pinned by its pidfd.
        pub(super) struct Member(OwnedFd);
        impl Member {
            pub(super) fn signal(&self, signal: Signal) {
                let _ = pidfd_send_signal(&self.0, signal);
            }
            pub(super) fn wait(self) {
                let mut descriptors = [PollFd::new(&self.0, PollFlags::IN)];
                while let Err(rustix::io::Errno::INTR) = poll(&mut descriptors, None) {}
            }
        }

        fn stat_session(pid: u32) -> Option<(u32, char)> {
            let stat = fs::read(format!("/proc/{pid}/stat")).ok()?;
            // Linux comm bytes are arbitrary; only the numeric suffix is ASCII.
            let end = stat.iter().rposition(|byte| *byte == b')')?;
            let mut fields = std::str::from_utf8(stat.get(end + 1..)?)
                .ok()?
                .split_whitespace();
            let state = fields.next()?.chars().next()?;
            let sid = fields.nth(2)?.parse().ok()?;
            Some((sid, state))
        }
        /// The live processes of the session, each pinned by a pidfd.
        pub(super) fn members(sid: Pid) -> Vec<Member> {
            let Ok(entries) = fs::read_dir("/proc") else {
                return Vec::new();
            };
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
                    let (session, state) = stat_session(pid)?;
                    if session != sid.as_raw_nonzero().get() as u32 || state == 'Z' {
                        return None;
                    }
                    let fd = pidfd_open(
                        Pid::from_raw(i32::try_from(pid).ok()?)?,
                        PidfdFlags::empty(),
                    )
                    .ok()?;
                    // Recheck after pinning; a recycled PID cannot target an unrelated session.
                    (stat_session(pid)?.0 == session).then_some(Member(fd))
                })
                .collect()
        }

        pub(super) fn wait(wake: &UnixStream, pty: Option<(&FileDescriptor, bool)>, exit: Option<&Exit>, timeout: Option<Duration>) -> io::Result<Ready> {
            let mut descriptors = vec![PollFd::new(wake, PollFlags::IN)];
            if let Some((fd, write)) = pty {
                descriptors.push(PollFd::new(fd, if write { PollFlags::IN | PollFlags::OUT } else { PollFlags::IN }));
            }
            if let Some(exit) = exit {
                descriptors.push(PollFd::new(&exit.0, PollFlags::IN));
            }
            let timeout = timeout.map(|d| Timespec { tv_sec: d.as_secs() as _, tv_nsec: d.subsec_nanos() as _ });
            poll(&mut descriptors, timeout.as_ref())?;
            let flags = |index: usize| descriptors.get(index).map_or(PollFlags::empty(), PollFd::revents);
            let pty_flags = if pty.is_some() { flags(1) } else { PollFlags::empty() };
            Ok(Ready {
                wake: flags(0).intersects(PollFlags::IN | PollFlags::HUP),
                pty_read: pty_flags.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR),
                pty_write: pty_flags.contains(PollFlags::OUT),
                child: exit.is_some() && !flags(1 + usize::from(pty.is_some())).is_empty(),
            })
        }
    }

    /// macOS has neither pidfds nor /proc. Built by CI on macOS, never run there yet: see docs/code/terminal.md.
    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)] // libproc and select(2), which rustix does not wrap; see the SAFETY comments
    mod os {
        use super::{FileDescriptor, Ready};
        use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, getsid, kill_process, waitid};
        use std::{
            io::{self, Write},
            os::{fd::AsRawFd, unix::net::UnixStream},
            sync::{Arc, atomic::{AtomicBool, Ordering}},
            time::Duration,
        };

        /// `sys/proc.h`'s zombie state, as `proc_bsdinfo::pbi_status` reports it.
        const SZOMB: u32 = 5;

        /// Nothing to reserve: no descriptor stands for the shell here.
        pub(super) fn preflight() -> Result<Option<std::os::fd::OwnedFd>, String> {
            Ok(None)
        }

        /// The shell's exit. A thread waits for it without reaping (`WNOWAIT`), so the unreaped leader keeps its PID
        /// (and the session's ID) from being recycled until `Child::wait`, then wakes the reactor through a socket.
        pub(super) struct Exit {
            pid: Pid,
            fd: UnixStream,
            done: Arc<AtomicBool>,
        }
        impl Exit {
            pub(super) fn new(pid: Pid) -> Result<Self, String> {
                let (fd, mut notify) = UnixStream::pair().map_err(|e| e.to_string())?;
                fd.set_nonblocking(true).map_err(|e| e.to_string())?;
                let done = Arc::new(AtomicBool::new(false));
                let flag = done.clone();
                std::thread::Builder::new()
                    .name("terminal-exit".into())
                    .spawn(move || {
                        // NOWAIT leaves the exited child for its owner to reap.
                        while let Err(rustix::io::Errno::INTR) = waitid(WaitId::Pid(pid), WaitIdOptions::EXITED | WaitIdOptions::NOWAIT) {}
                        // A failed wait reports an exit too: a shell shown as ended beats a reactor that never wakes.
                        flag.store(true, Ordering::Release);
                        let _ = notify.write(&[1]);
                    })
                    .map_err(|e| e.to_string())?;
                Ok(Self { pid, fd, done })
            }
            pub(super) fn signal(&self, signal: Signal) {
                let _ = kill_process(self.pid, signal);
            }
            pub(super) fn exited(&self) -> bool {
                self.done.load(Ordering::Acquire)
            }
        }

        /// A live process of the shell's session, found by its session ID.
        pub(super) struct Member {
            pid: Pid,
            sid: Pid,
        }
        impl Member {
            fn current(&self) -> bool {
                getsid(Some(self.pid)).is_ok_and(|sid| sid == self.sid) && alive(self.pid)
            }
            // ponytail: without pidfds a member that exits between this check and the signal could be replaced by an
            // unrelated process with its PID; pinning each member (kqueue EVFILT_PROC) would close that window.
            pub(super) fn signal(&self, signal: Signal) {
                if self.current() {
                    let _ = kill_process(self.pid, signal);
                }
            }
            pub(super) fn wait(self) {
                while self.current() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }

        /// Whether the process exists and is not a zombie.
        fn alive(pid: Pid) -> bool {
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            // SAFETY: an all-zero proc_bsdinfo is a valid value of this plain C struct.
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            // SAFETY: `info` is writable and `size` is its size in bytes.
            let read = unsafe { libc::proc_pidinfo(pid.as_raw_pid(), libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
            read == size && info.pbi_status != SZOMB
        }

        pub(super) fn members(sid: Pid) -> Vec<Member> {
            // SAFETY: a null buffer asks only for the number of processes.
            let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
            if count <= 0 {
                return Vec::new();
            }
            // Room for processes started since the count.
            let mut pids: Vec<libc::pid_t> = vec![0; count as usize + 64];
            let bytes = std::mem::size_of_val(pids.as_slice()) as libc::c_int;
            // SAFETY: the buffer holds `pids.len()` PIDs and its size is given in bytes.
            let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
            pids.truncate(count.max(0) as usize);
            pids.into_iter()
                .filter_map(Pid::from_raw)
                .map(|pid| Member { pid, sid })
                .filter(Member::current)
                .collect()
        }

        /// poll(2) on macOS does not support devices, and the PTY master is one; select(2) does.
        pub(super) fn wait(wake: &UnixStream, pty: Option<(&FileDescriptor, bool)>, exit: Option<&Exit>, timeout: Option<Duration>) -> io::Result<Ready> {
            let fds = [Some(wake.as_raw_fd()), pty.map(|(fd, _)| fd.as_raw_fd()), exit.map(|exit| exit.fd.as_raw_fd())];
            if fds.iter().flatten().any(|&fd| fd < 0 || fd as usize >= libc::FD_SETSIZE) {
                return Err(io::Error::other("A terminal descriptor is beyond select's limit"));
            }
            let mut timeval = timeout.map(|d| libc::timeval { tv_sec: d.as_secs() as _, tv_usec: d.subsec_micros() as _ });
            let timeval = timeval.as_mut().map_or(std::ptr::null_mut(), |t| t as *mut libc::timeval);
            let nfds = fds.iter().flatten().max().map_or(0, |fd| fd + 1);
            // SAFETY: zeroed fd_sets are valid empty sets, and every descriptor is open and below FD_SETSIZE.
            unsafe {
                let mut read: libc::fd_set = std::mem::zeroed();
                let mut write: libc::fd_set = std::mem::zeroed();
                libc::FD_ZERO(&mut read);
                libc::FD_ZERO(&mut write);
                for &fd in fds.iter().flatten() {
                    libc::FD_SET(fd, &mut read);
                }
                if let Some((fd, true)) = pty {
                    libc::FD_SET(fd.as_raw_fd(), &mut write);
                }
                if libc::select(nfds, &mut read, &mut write, std::ptr::null_mut(), timeval) < 0 {
                    return Err(io::Error::last_os_error());
                }
                let set = |fd: Option<i32>, set: &libc::fd_set| fd.is_some_and(|fd| libc::FD_ISSET(fd, set));
                Ok(Ready { wake: set(fds[0], &read), pty_read: set(fds[1], &read), pty_write: set(fds[1], &write), child: set(fds[2], &read) })
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(process: &Process, predicate: impl Fn(&Update) -> bool) -> Update {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let update = process.take();
            if predicate(&update) {
                return update;
            }
            assert!(
                Instant::now() < deadline,
                "terminal condition timed out: {}",
                update.status
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn terminal_pty_real_shell_exact_paste_exit_and_retained_selection() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(1),
            1,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process.send(Command::Text("printf '\\033[?2004h'; stty -echo -icanon; printf '\\122\\105\\101\\104\\131'; dd bs=1 count=17 2>/dev/null | od -An -tx1 -w32; exit\n".into())).unwrap();
        wait_for(&process, |u| {
            u.frame.as_ref().is_some_and(|f| {
                let text: String = f.cells.iter().map(|c| c.text.as_str()).collect();
                text.contains("READY")
            })
        });
        assert!(
            process
                .send(Command::Paste("\x1b[201~injected\n".into()))
                .is_err()
        );
        assert!(process.send(Command::Paste("x".repeat(1_048_577))).is_err());
        process.send(Command::Paste("a\nbcd".into())).unwrap();
        let update = wait_for(&process, |u| !u.running);
        let text: String = update
            .frame
            .unwrap()
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect();
        assert!(
            text.contains("1b 5b 32 30 30 7e 61 0a 62 63 64 1b 5b 32 30 31 7e"),
            "{text}"
        );
        process
            .send(Command::Select {
                row: 0,
                column: 0,
                extend: false,
            })
            .unwrap();
        process
            .send(Command::Select {
                row: 0,
                column: 5,
                extend: true,
            })
            .unwrap();
        process.send(Command::Copy).unwrap();
        assert!(wait_for(&process, |u| u.copy.is_some()).copy.is_some());
    }

    #[test]
    #[ignore = "Feature-only search under active output; emits raw query/input feedback samples"]
    fn terminal_pty_find_output_release_probe() {
        let directory = tempfile::tempdir().unwrap();
        let (tx,_rx) = async_channel::unbounded();
        let process = Process::spawn(Id(400),1,directory.path().into(),"/bin/sh".into(),tx,(0xffffff,0)).unwrap();
        process.send(Command::Text("stty -echo; (while :; do printf 'needle.*界\\r\\n'; done) & producer=$!; printf '%s' $producer > producer; read answer; kill $producer; wait $producer 2>/dev/null; printf 'INPUT_ACK_%s' \"$answer\"\n".into())).unwrap();
        wait_pid(directory.path(),"producer");
        let start = Instant::now();
        let mut samples = Vec::new();
        let seconds = std::env::var("LENZ_TERMINAL_FIND_PROBE_SECONDS").ok().and_then(|s|s.parse::<u64>().ok()).unwrap_or(10).clamp(1,60);
        let mut index = 0;
        while start.elapsed() < Duration::from_secs(seconds) {
            let query = format!("absent-{index}");
            let asked = Instant::now();
            process.send(Command::Search {query:query.clone(),backwards:index%2 != 0,advance:false}).unwrap();
            let update = wait_for(&process,|u|u.frame.as_ref().is_some_and(|f|f.search.as_ref().is_some_and(|(q,_)|q == &query)));
            assert!(update.running);
            assert!(update.frame.unwrap().scrollback <= 10_000);
            samples.push(asked.elapsed().as_nanos() as u64);
            index += 1;
        }
        let asked = Instant::now();
        process.send(Command::Text("responsive\n".into())).unwrap();
        wait_for(&process,|u|u.frame.as_ref().is_some_and(|f|f.cells.iter().map(|c|c.text.as_str()).collect::<String>().contains("INPUT_ACK_responsive")));
        let input_ns = asked.elapsed().as_nanos() as u64;
        process.send(Command::Search {query:"INPUT_ACK_responsive".into(),backwards:false,advance:false}).unwrap();
        wait_for(&process,|u|u.frame.as_ref().is_some_and(|f|matches!(&f.search,Some((q,Some(true))) if q == "INPUT_ACK_responsive")));
        println!("{}",serde_json::json!({"schema":1,"probe":"terminal_find_active_output","elapsed_ns":start.elapsed().as_nanos() as u64,"query_feedback_ns":samples,"input_completion_ns":input_ns}));
        process.stop();
        let deadline = Instant::now()+Duration::from_secs(5);
        while !process.finished() { assert!(Instant::now()<deadline); std::thread::sleep(Duration::from_millis(2)); }
    }

    #[test]
    fn terminal_pty_cancel_bypasses_saturated_input() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(2),
            1,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process
            .send(Command::Text(
                "printf '%s' $$ > leader; exec sleep 60\n".into(),
            ))
            .unwrap();
        let pid = wait_pid(directory.path(), "leader");
        let payload = "x".repeat(1_048_576);
        let mut rejected = false;
        for _ in 0..8 {
            rejected |= process.send(Command::Text(payload.clone())).is_err();
        }
        assert!(rejected);
        let start = Instant::now();
        drop(process);
        assert!(start.elapsed() < Duration::from_millis(100));
        wait_dead(pid, true);
    }

    fn wait_pid(directory: &std::path::Path, name: &str) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(text) = std::fs::read_to_string(directory.join(name))
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            assert!(
                Instant::now() < deadline,
                "PID fixture {name} did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn wait_dead(pid: u32, reaped: bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let stat = std::fs::read(format!("/proc/{pid}/stat"));
            if stat.is_err()
                || (!reaped
                    && stat.as_ref().is_ok_and(|bytes| {
                        bytes
                            .iter()
                            .rposition(|b| *b == b')')
                            .and_then(|end| bytes.get(end + 1..))
                            .and_then(|suffix| suffix.iter().find(|b| !b.is_ascii_whitespace()))
                            == Some(&b'Z')
                    }))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "Owned process {pid} is still alive (reaped={reaped})"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn terminal_pty_drop_cleans_background_pipeline_and_reaps_leader() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(3),
            1,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process
            .send(Command::Text(
                "printf '%s' $$ > leader; sleep 60 | sleep 60 & printf '%s' $! > pipeline; wait\n"
                    .into(),
            ))
            .unwrap();
        let leader = wait_pid(directory.path(), "leader");
        let pipeline = wait_pid(directory.path(), "pipeline");
        drop(process);
        wait_dead(leader, true);
        wait_dead(pipeline, false);
    }

    #[test]
    fn terminal_pty_shell_exit_cleans_attached_jobs_and_keeps_screen() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(4),
            1,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process
            .send(Command::Text(
                "sleep 60 & printf '%s' $! > attached; printf '\\106\\111\\116\\101\\114'; exit\n"
                    .into(),
            ))
            .unwrap();
        let attached = wait_pid(directory.path(), "attached");
        let update = wait_for(&process, |update| !update.running);
        let frame = update.frame.unwrap();
        let text: String = frame.cells.iter().map(|cell| cell.text.as_str()).collect();
        assert!(text.contains("FINAL"));
        wait_dead(attached, false);
        assert!(process.send(Command::Text("ignored".into())).is_err());
    }

    #[test]
    fn terminal_pty_detached_sessions_are_outside_ownership() {
        use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
        let directory = tempfile::tempdir().unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(5),
            1,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process.send(Command::Text("setsid /bin/sh -c 'echo $$ > detached; exec sleep 60' </dev/null >/dev/null 2>&1 &\n".into())).unwrap();
        let detached = wait_pid(directory.path(), "detached");
        let fd = pidfd_open(Pid::from_raw(detached as i32).unwrap(), PidfdFlags::empty()).unwrap();
        drop(process);
        std::thread::sleep(Duration::from_millis(700));
        let stat = std::fs::read_to_string(format!("/proc/{detached}/stat")).unwrap();
        assert!(
            !stat
                .rsplit_once(')')
                .unwrap()
                .1
                .trim_start()
                .starts_with('Z')
        );
        pidfd_send_signal(fd, Signal::KILL).unwrap();
        wait_dead(detached, false);
    }
    #[test]
    fn terminal_pty_full_completion_inbox_retries_final_wake() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, rx) = async_channel::bounded(1);
        tx.try_send(session::Completion::TerminalWake {
            id: Id(99),
            generation: 99,
        })
        .unwrap();
        let process = Process::spawn(
            Id(6),
            7,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process
            .send(Command::Text("printf '%s' $$ > leader; exit\n".into()))
            .unwrap();
        let leader = wait_pid(directory.path(), "leader");
        wait_dead(leader, true);
        // The sole inbox slot remains occupied until the shell's final state is ready.
        assert!(matches!(
            rx.try_recv().unwrap(),
            session::Completion::TerminalWake { id: Id(99), .. }
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(completion) = rx.try_recv() {
                assert!(matches!(
                    completion,
                    session::Completion::TerminalWake {
                        id: Id(6),
                        generation: 7
                    }
                ));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "The final wake was stranded behind a full inbox"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!process.take().running);
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            rx.is_empty(),
            "Acknowledged terminal must not keep polling or waking"
        );
    }
    #[test]
    fn terminal_pty_resize_query_response_and_bad_spawn() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, _rx) = async_channel::unbounded();
        assert!(
            Process::spawn(
                Id(7),
                1,
                directory.path().into(),
                "/missing/lenz-test-shell".into(),
                tx.clone(),
                (0xffffff, 0)
            )
            .is_err()
        );
        let process = Process::spawn(
            Id(7),
            2,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process
            .send(Command::Resize {
                columns: 101,
                rows: 31,
            })
            .unwrap();
        wait_for(&process, |u| {
            u.frame
                .as_ref()
                .is_some_and(|f| f.columns == 101 && f.rows == 31)
        });
        process.send(Command::Text("stty -echo -icanon; stty size > dimensions; printf '\\033[2J\\033[H\\033[6n'; dd bs=1 count=6 2>/dev/null | od -An -tx1 -w32; exit\n".into())).unwrap();
        let update = wait_for(&process, |u| !u.running);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("dimensions"))
                .unwrap()
                .trim(),
            "31 101"
        );
        let text: String = update
            .frame
            .unwrap()
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect();
        assert!(
            text.contains("1b 5b 31 3b 31 52"),
            "The model must send its cursor-position query response: {text}"
        );
    }
    #[test]
    fn terminal_pty_closed_completion_inbox_wakes_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(8),
            1,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process
            .send(Command::Text(
                "printf '%s' $$ > leader; exec sleep 60\n".into(),
            ))
            .unwrap();
        let leader = wait_pid(directory.path(), "leader");
        drop(rx);
        process.take();
        process
            .send(Command::Resize {
                columns: 83,
                rows: 27,
            })
            .unwrap();
        wait_dead(leader, true);
    }

    #[test]
    fn terminal_pty_stop_ack_waits_for_hup_ignoring_shell_and_retries_full_inbox() {
        let directory = tempfile::tempdir().unwrap();
        let (tx, rx) = async_channel::bounded(1);
        let process = Process::spawn(
            Id(9),
            5,
            directory.path().into(),
            "/bin/sh".into(),
            tx.clone(),
            (0xffffff, 0),
        )
        .unwrap();
        process
            .send(Command::Text(
                "trap '' HUP TERM; printf '%s' $$ > leader; while :; do sleep 60; done\n".into(),
            ))
            .unwrap();
        let leader = wait_pid(directory.path(), "leader");
        while rx.try_recv().is_ok() {}
        process.take();
        loop {
            if tx
                .try_send(session::Completion::TerminalWake {
                    id: Id(99),
                    generation: 99,
                })
                .is_ok()
            {
                break;
            }
            let _ = rx.try_recv();
            process.take();
        }
        process.take();
        let start = Instant::now();
        process.stop();
        assert!(start.elapsed() < Duration::from_millis(100));
        assert!(
            !process.finished(),
            "HUP/TERM-ignoring leader must not be acknowledged before escalation"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !process.finished() {
            assert!(
                Instant::now() < deadline,
                "Stop acknowledgement never completed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        wait_dead(leader, true);
        assert!(matches!(
            rx.try_recv().unwrap(),
            session::Completion::TerminalWake { id: Id(99), .. }
        ));
        loop {
            if let Ok(completion) = rx.try_recv() {
                assert!(matches!(
                    completion,
                    session::Completion::TerminalWake {
                        id: Id(9),
                        generation: 5
                    }
                ));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "The final stop acknowledgement was stranded in a full inbox"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!process.take().running);
    }

    #[test]
    fn terminal_pty_stop_cleans_child_with_non_utf8_process_name() {
        use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open, pidfd_send_signal};
        use std::os::unix::{ffi::OsStringExt, fs::symlink};
        struct ChildGuard(std::os::fd::OwnedFd);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = pidfd_send_signal(&self.0, Signal::KILL);
            }
        }
        let directory = tempfile::tempdir().unwrap();
        symlink(
            "/bin/sleep",
            directory
                .path()
                .join(OsString::from_vec(b"sleep-\xff".to_vec())),
        )
        .unwrap();
        let (tx, _rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(10),
            1,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process.send(Command::Text("printf '%s' $$ > leader; (trap '' HUP TERM; exec \"./$(printf 'sleep-\\377')\" 60) & printf '%s' $! > binary; wait\n".into())).unwrap();
        let leader = wait_pid(directory.path(), "leader");
        let binary = wait_pid(directory.path(), "binary");
        let _guard = ChildGuard(
            pidfd_open(Pid::from_raw(binary as i32).unwrap(), PidfdFlags::empty()).unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if std::fs::read(format!("/proc/{binary}/stat"))
                .is_ok_and(|bytes| bytes.contains(&0xff))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Fixture never exec'd its non-UTF8 process name"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        process.stop();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !process.finished() {
            assert!(
                Instant::now() < deadline,
                "Non-UTF8 child prevented cleanup acknowledgement"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        wait_dead(leader, true);
        wait_dead(binary, false);
    }

    #[test]
    #[ignore = "Feature-only release stress; emits machine-readable RSS and correctness samples"]
    fn terminal_pty_release_stress() {
        fn memory_kib(prefix: &str) -> u64 {
            std::fs::read_to_string("/proc/self/status")
                .unwrap()
                .lines()
                .find_map(|line| {
                    line.strip_prefix(prefix)
                        .and_then(|v| v.split_whitespace().next())
                        .and_then(|v| v.parse().ok())
                })
                .unwrap()
        }
        fn rss_kib() -> u64 {
            memory_kib("VmRSS:")
        }
        fn cpu_ticks(pid: u32) -> u64 {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            let fields: Vec<_> = stat
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .collect();
            fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
        }
        struct Sentinel(std::process::Child);
        impl Drop for Sentinel {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut sentinel = Sentinel(
            std::process::Command::new("sleep")
                .arg("300")
                .spawn()
                .unwrap(),
        );
        let seconds: u64 = std::env::var("LENZ_TERMINAL_STRESS_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10)
            .clamp(1, 120);
        let rss_start = rss_kib();
        let mut rss_peak = rss_start;
        let mut samples = Vec::new();
        let mut cycles = 0_u64;
        let mut cycle_samples = Vec::new();
        // Every cycle owns real shells, session-leader reaping and a background pipeline.
        for index in 0..8 {
            let cycle_start = Instant::now();
            let directory = tempfile::tempdir().unwrap();
            let (tx, _rx) = async_channel::unbounded();
            let process = Process::spawn(
                Id(100 + index),
                1,
                directory.path().into(),
                "/bin/sh".into(),
                tx,
                (0xffffff, 0),
            )
            .unwrap();
            process.send(Command::Text("printf '%s' $$ > leader; sleep 60 | sleep 60 & printf '%s' $! > pipeline; wait\n".into())).unwrap();
            let spawned_ms = cycle_start.elapsed().as_secs_f64() * 1000.0;
            let leader = wait_pid(directory.path(), "leader");
            let pipeline = wait_pid(directory.path(), "pipeline");
            let first_feedback_ms = cycle_start.elapsed().as_secs_f64() * 1000.0;
            let stop_start = Instant::now();
            drop(process);
            wait_dead(leader, true);
            wait_dead(pipeline, false);
            cycle_samples.push(serde_json::json!({"cycle": index, "spawn_ms": spawned_ms,
                "first_feedback_ms": first_feedback_ms, "stop_ms": stop_start.elapsed().as_secs_f64() * 1000.0,
                "rss_kib": rss_kib()}));
            assert!(sentinel.0.try_wait().unwrap().is_none());
            cycles += 1;
            rss_peak = rss_peak.max(rss_kib());
        }
        let directory = tempfile::tempdir().unwrap();
        let (tx, rx) = async_channel::unbounded();
        let process = Process::spawn(
            Id(200),
            4,
            directory.path().into(),
            "/bin/sh".into(),
            tx,
            (0xffffff, 0),
        )
        .unwrap();
        process.send(Command::Text("stty -echo; printf '%s' $$ > leader; while :; do printf '0123456789 terminal-stress \\347\\225\\214 e\\314\\201\\n'; done\n".into())).unwrap();
        let leader = wait_pid(directory.path(), "leader");
        let start = Instant::now();
        let cpu_start = cpu_ticks(std::process::id());
        let child_cpu_start = cpu_ticks(leader);
        let mut frames = 0_u64;
        let mut wakes = 0_u64;
        let mut resizes = 0_u64;
        let mut selections = 0_u64;
        let mut scrolls = 0_u64;
        let mut last_resize = Instant::now() - Duration::from_secs(1);
        let mut last_sample = Instant::now() - Duration::from_secs(1);
        while start.elapsed() < Duration::from_secs(seconds) {
            if last_resize.elapsed() >= Duration::from_millis(100) {
                let (columns, rows) = if resizes.is_multiple_of(2) {
                    (80, 24)
                } else {
                    (120, 32)
                };
                process.send(Command::Resize { columns, rows }).unwrap();
                process
                    .send(Command::Scroll {
                        lines: 3,
                        local: true,
                    })
                    .unwrap();
                process
                    .send(Command::Select {
                        row: 0,
                        column: 0,
                        extend: false,
                    })
                    .unwrap();
                process
                    .send(Command::Select {
                        row: 1,
                        column: 9,
                        extend: true,
                    })
                    .unwrap();
                resizes += 1;
                selections += 1;
                scrolls += 1;
                last_resize = Instant::now();
            }
            if let Ok(completion) = rx.try_recv() {
                wakes += 1;
                assert!(matches!(
                    completion,
                    session::Completion::TerminalWake {
                        id: Id(200),
                        generation: 4
                    }
                ));
                let update = process.take();
                assert!(
                    update.running && update.error.is_none(),
                    "{}",
                    update.status
                );
                if let Some(frame) = update.frame {
                    assert_eq!(frame.cells.len(), frame.columns * frame.rows);
                    assert!(frame.scrollback <= 16 * 1024 * 1024 / 256 / frame.columns);
                    assert!(
                        frame
                            .cursor
                            .is_none_or(|(row, column)| row < frame.rows && column < frame.columns)
                    );
                    assert!(
                        frame
                            .cells
                            .iter()
                            .all(|cell| cell.text.chars().count() <= 9)
                    );
                    frames += 1;
                }
            }
            if last_sample.elapsed() >= Duration::from_millis(100) {
                let rss = rss_kib();
                rss_peak = rss_peak.max(rss);
                samples.push(
                    serde_json::json!({"elapsed_ms": start.elapsed().as_millis(), "rss_kib": rss}),
                );
                last_sample = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let elapsed_ms = start.elapsed().as_millis();
        let process_cpu_ticks = cpu_ticks(std::process::id()) - cpu_start;
        let child_cpu_ticks = cpu_ticks(leader) - child_cpu_start;
        assert!(frames > 5 && resizes >= 5);
        // Saturation is bounded and atomic even while the child produces output continuously.
        assert!(
            process
                .send(Command::Text("x".repeat(2 * 1024 * 1024 + 1)))
                .is_err()
        );
        drop(process);
        wait_dead(leader, true);
        assert!(sentinel.0.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(100));
        let rss_settled = rss_kib();
        rss_peak = rss_peak.max(rss_settled);
        let rss_vm_hwm = memory_kib("VmHWM:");
        assert!(
            rss_vm_hwm.saturating_sub(rss_start) < 96 * 1024,
            "Terminal feature stress exceeded its conservative process-RSS allowance"
        );
        println!(
            "{}",
            serde_json::json!({ "schema": 2, "workload": "terminal-feature-pty", "requested_seconds": seconds,
            "elapsed_ms": elapsed_ms, "sustained_process_cpu_ticks": process_cpu_ticks, "sustained_child_cpu_ticks": child_cpu_ticks,
            "cpu_ticks_per_second": String::from_utf8(std::process::Command::new("getconf").arg("CLK_TCK").output().unwrap().stdout).unwrap().trim().parse::<u64>().unwrap(),
            "wakes": wakes, "rss_start_kib": rss_start, "rss_peak_sampled_kib": rss_peak, "rss_vm_hwm_kib": rss_vm_hwm, "rss_settled_kib": rss_settled,
            "create_stop_cycles": cycles, "cycle_samples": cycle_samples,
            "release": !cfg!(debug_assertions), "executable": std::env::current_exe().unwrap(), "frames_checked": frames, "resizes": resizes, "selections": selections,
            "scrolls": scrolls, "unrelated_process_survived": true, "samples": samples })
        );
    }
}
