//! Shell worlds: a world whose "connection" is a terminal running the user's shell.
//!
//! Clay's server process owns the PTY and the terminal emulator (alacritty_terminal's
//! `Term`); what every client gets is the *screen*, as rows of ANSI text
//! (`screen::ShellScreen`), and what it sends back are keys (`keys`). The App side -
//! connecting, frames, activity, input - is `app.rs`. Nothing a shell prints is a line:
//! it never reaches `output_lines`, the log, the archive, TF or the triggers.
//!
//! Three rules this module lives by, each learned from how alacritty is built:
//! - `Proxy::send_event` is called by alacritty's reader thread while it holds the `Term`
//!   lock, and Clay's event channel is bounded. It must never block and never touch the
//!   `Term`, or the App (waiting for the lock) and the reader (waiting for the channel)
//!   deadlock. It only `try_send`s a nudge; the App reads the screen when it gets to it.
//! - `Pty`'s `Drop` sends the child SIGHUP and then *waits* for it. A program that ignores
//!   SIGHUP would hang whoever drops the session, so the drop happens on the session's own
//!   wrapper thread, which kills the child first if it has to (`end_child`).
//! - Clay reaps every finished child it has on each loop pass, and alacritty finds its
//!   child's exit the same way. The shell runs in its own process group (alacritty's
//!   `setsid`), and Clay's reaper only looks in its own (`platform::reap_zombie_children`);
//!   otherwise the exit status is lost and alacritty's loop spins on the dead PTY.

pub mod app;
pub mod keys;
pub mod screen;
/// Re-attaching a PTY across a `/reload` (Linux only; see the module).
#[cfg(target_os = "linux")]
pub mod adopt;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Processor, Rgb, StdSyncHandler};

pub use screen::ShellScreen;

/// What a session reports to the App (`AppEvent::Shell`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    /// The screen may have changed: read it (`App::refresh_shell_screen`).
    Wakeup,
    /// The program set (or cleared) the window title.
    Title(Option<String>),
    Bell,
    /// The session is over: the shell exited with this code (None: killed, or unknown).
    Exited(Option<i32>),
}

/// The PTY's size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// Smaller than this and nothing works (and alacritty refuses to resize below 2x1).
pub const MIN_COLS: u16 = 20;
pub const MIN_ROWS: u16 = 5;

impl Size {
    /// With nobody looking yet.
    pub const DEFAULT: Size = Size { cols: 80, rows: 24 };

    pub fn clamped(self) -> Size {
        Size { cols: self.cols.max(MIN_COLS), rows: self.rows.max(MIN_ROWS) }
    }

    fn window_size(self) -> WindowSize {
        // Cell pixel sizes only matter to programs that ask (CSI 14 t); nominal values.
        WindowSize { num_lines: self.rows, num_cols: self.cols, cell_width: 8, cell_height: 16 }
    }
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

/// alacritty's view of us: where the emulator's events go. Cloned into the `Term` and the
/// event loop. See the module doc for why it never blocks.
#[derive(Clone)]
pub struct Proxy {
    world: String,
    conn_id: u64,
    events: tokio::sync::mpsc::Sender<crate::AppEvent>,
    /// Set once the event loop exists; the terminal's answers (DA, DSR, ...) go back
    /// through it.
    pty: Arc<OnceLock<EventLoopSender>>,
    /// A Wakeup nudge is in the App's channel (or was dropped: the App's per-pass
    /// `flush_shell_frames` reads the flag too). Cleared by the App as it reads the screen.
    wakeup: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
    size: Arc<Mutex<Size>>,
}

impl Proxy {
    fn nudge(&self, ev: ShellEvent) {
        let _ = self.events.try_send(crate::AppEvent::Shell(self.world.clone(), self.conn_id, ev));
    }

    fn write(&self, bytes: Vec<u8>) {
        if let Some(tx) = self.pty.get() {
            let _ = tx.send(Msg::Input(Cow::Owned(bytes)));
        }
    }
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => {
                if !self.wakeup.swap(true, Ordering::AcqRel) {
                    self.nudge(ShellEvent::Wakeup);
                }
            }
            Event::PtyWrite(text) => self.write(text.into_bytes()),
            Event::ColorRequest(index, format) => self.write(format(default_color(index)).into_bytes()),
            Event::TextAreaSizeRequest(format) => {
                let size = *self.size.lock().unwrap_or_else(|e| e.into_inner());
                self.write(format(size.window_size()).into_bytes());
            }
            Event::Title(title) => self.nudge(ShellEvent::Title(Some(title))),
            Event::ResetTitle => self.nudge(ShellEvent::Title(None)),
            Event::Bell => self.nudge(ShellEvent::Bell),
            Event::ChildExit(status) => {
                *self.exit_code.lock().unwrap_or_else(|e| e.into_inner()) = status.code();
            }
            // `Exit` follows `ChildExit` (the wrapper thread reports the end), the clipboard
            // is never a program's to read or write (`Osc52::Disabled`), and the rest are
            // hints for a renderer that draws its own cursor.
            Event::Exit
            | Event::MouseCursorDirty
            | Event::CursorBlinkingChange
            | Event::ClipboardStore(..)
            | Event::ClipboardLoad(..) => {}
        }
    }
}

/// xterm's default palette, for a program that asks what colour N is (OSC 4). Clients
/// draw with their own themes, so this is only ever an answer, never a colour drawn.
fn default_color(index: usize) -> Rgb {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0), (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
        (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0), (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
    ];
    let (r, g, b) = match index {
        0..=15 => BASE[index],
        16..=231 => {
            let n = index - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + 40 * v) as u8 };
            (level(n / 36), level(n / 6 % 6), level(n % 6))
        }
        232..=255 => {
            let v = (8 + 10 * (index - 232)) as u8;
            (v, v, v)
        }
        257 => BASE[0],         // background
        _ => (229, 229, 229),   // foreground, cursor, and anything else
    };
    Rgb { r, g, b }
}

/// One running shell: the emulator, the way into its PTY, and who it belongs to.
///
/// Dropping it ends the session: alacritty's loop is told to shut down, the wrapper thread
/// ends the child and reports `ShellEvent::Exited`.
pub struct ShellSession {
    term: Arc<FairMutex<Term<Proxy>>>,
    tx: EventLoopSender,
    pub conn_id: u64,
    /// The shell's pid (always known on Unix; on Windows when ConPTY reports it).
    pub pid: Option<u32>,
    /// The PTY master, for a reload to carry across the exec.
    #[cfg(unix)]
    pub master_fd: std::os::fd::RawFd,
    size: Arc<Mutex<Size>>,
    wakeup: Arc<AtomicBool>,
    /// False once the event loop has ended; the wrapper thread's `Exited` follows.
    live: Arc<AtomicBool>,
}

impl ShellSession {
    /// Start `command` (or the login shell) in a PTY of `size` for world `world`.
    /// Everything the session has to say arrives as `AppEvent::Shell(world, conn_id, ..)`.
    pub fn start(
        world: &str,
        conn_id: u64,
        command: Option<&str>,
        size: Size,
        events: tokio::sync::mpsc::Sender<crate::AppEvent>,
    ) -> io::Result<ShellSession> {
        install_logger();
        let size = size.clamped();
        let (program, args) = program_and_args(command);
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("CLAY_WORLD".to_string(), world.to_string());
        let options = tty::Options {
            shell: Some(tty::Shell::new(program, args)),
            working_directory: home_dir(),
            drain_on_exit: true,
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, size.window_size(), 0)?;
        #[cfg(unix)]
        let pid = Some(pty.child().id());
        #[cfg(unix)]
        let master_fd = {
            use std::os::fd::AsRawFd;
            pty.file().as_raw_fd()
        };
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(|p| p.get());
        #[cfg(unix)]
        let mut session = spawn_loop(world, conn_id, pty, size, events, pid)?;
        #[cfg(unix)]
        {
            session.master_fd = master_fd;
        }
        #[cfg(not(unix))]
        let session = spawn_loop(world, conn_id, pty, size, events, pid)?;
        Ok(session)
    }

    /// The loop is still running (the shell may still be exiting).
    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    pub fn size(&self) -> Size {
        *self.size.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bytes for the program: keys, text, a paste (see `keys`).
    pub fn write(&self, bytes: Vec<u8>) {
        if self.is_live() && !bytes.is_empty() {
            let _ = self.tx.send(Msg::Input(Cow::Owned(bytes)));
        }
    }

    /// The viewers' size changed. The emulator first, then the PTY (SIGWINCH), so the
    /// program's redraw lands on a grid of the new size. Never after the loop ended:
    /// alacritty exits the process when it can't set the size of a dead PTY.
    pub fn resize(&self, size: Size) {
        let size = size.clamped();
        if !self.is_live() || size == self.size() {
            return;
        }
        self.term.lock().resize(size);
        *self.size.lock().unwrap_or_else(|e| e.into_inner()) = size;
        let _ = self.tx.send(Msg::Resize(size.window_size()));
    }

    /// Read the screen (`screen::render_rows`), briefly keeping the reader thread out.
    pub fn with_term<R>(&self, f: impl FnOnce(&Term<Proxy>) -> R) -> R {
        f(&self.term.lock())
    }

    /// The emulator's current modes, as a program's key needs them encoded.
    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    /// Text onto the screen as if the program had written it (Clay's own notes, such as
    /// "shell exited"). Works after the loop has ended, too.
    pub fn inject(&self, bytes: &[u8]) {
        let mut term = self.term.lock();
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut *term, bytes);
    }

    /// True once since the last call if the emulator changed something.
    pub fn take_wakeup(&self) -> bool {
        self.wakeup.swap(false, Ordering::AcqRel)
    }

    /// End the session: the loop stops, the wrapper thread ends the child and reports
    /// `Exited`. Dropping the session does the same.
    pub fn shutdown(&self) {
        let _ = self.tx.send(Msg::Shutdown);
    }

    /// SIGHUP the child now, synchronously. Used right before a reload `exec`, which wipes
    /// the wrapper thread that would otherwise do it: without this the child would be
    /// reparented to init and keep running. (Phase 7 will instead carry it across.)
    #[cfg(unix)]
    pub fn hangup(&self) {
        if let Some(pid) = self.pid {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGHUP);
            }
        }
    }
}

impl Drop for ShellSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Build the emulator, run alacritty's event loop over `pty` on its own thread, and wrap
/// it in a thread that reports `Exited` and does the bounded teardown off the main thread.
/// Shared by `start` (a fresh `tty::Pty`) and `adopt` (an `AdoptedPty` across a reload).
fn spawn_loop<P>(
    world: &str,
    conn_id: u64,
    pty: P,
    size: Size,
    events: tokio::sync::mpsc::Sender<crate::AppEvent>,
    pid: Option<u32>,
) -> io::Result<ShellSession>
where
    P: alacritty_terminal::tty::EventedPty + alacritty_terminal::event::OnResize + Send + 'static,
{
    let wakeup = Arc::new(AtomicBool::new(false));
    let exit_code = Arc::new(Mutex::new(None));
    let size_cell = Arc::new(Mutex::new(size));
    let proxy = Proxy {
        world: world.to_string(),
        conn_id,
        events: events.clone(),
        pty: Arc::new(OnceLock::new()),
        wakeup: wakeup.clone(),
        exit_code: exit_code.clone(),
        size: size_cell.clone(),
    };
    let config = Config { scrolling_history: 0, kitty_keyboard: false, osc52: Osc52::Disabled, ..Config::default() };
    let term = Arc::new(FairMutex::new(Term::new(config, &size, proxy.clone())));
    let event_loop = EventLoop::new(term.clone(), proxy.clone(), pty, true, false)?;
    let tx = event_loop.channel();
    let _ = proxy.pty.set(tx.clone());
    let live = Arc::new(AtomicBool::new(true));
    let loop_thread = event_loop.spawn();

    let wrapper = {
        let world = world.to_string();
        let live = live.clone();
        let exit_code = exit_code.clone();
        std::thread::Builder::new().name(format!("clay-shell-{world}")).spawn(move || {
            let joined = loop_thread.join();
            live.store(false, Ordering::Release);
            let code = exit_code.lock().unwrap_or_else(|e| e.into_inner()).take();
            let code = match joined {
                Ok((event_loop, _state)) => {
                    #[cfg(unix)]
                    let code = match pid {
                        Some(pid) => code.or(end_child(pid)),
                        None => code,
                    };
                    // The pty's Drop: SIGHUP and a wait, on a child that is already gone.
                    drop(event_loop);
                    code
                }
                Err(_) => code,
            };
            let _ = events.blocking_send(crate::AppEvent::Shell(world, conn_id, ShellEvent::Exited(code)));
        })
    };
    if let Err(e) = wrapper {
        let _ = tx.send(Msg::Shutdown);
        return Err(e);
    }
    Ok(ShellSession {
        term,
        tx,
        conn_id,
        pid,
        #[cfg(unix)]
        master_fd: -1,
        size: size_cell,
        wakeup,
        live,
    })
}

/// The loop has ended; make sure the child has too, and collect its status when alacritty
/// didn't: SIGHUP (what a closed terminal means) and up to two seconds, then SIGKILL.
/// Returns the exit code of a child that exited normally.
#[cfg(unix)]
fn end_child(pid: u32) -> Option<i32> {
    let pid = pid as libc::pid_t;
    let mut status: libc::c_int = 0;
    let code = |status: libc::c_int| libc::WIFEXITED(status).then(|| libc::WEXITSTATUS(status));
    // Reaped already (by alacritty, normally)?
    let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    if r == pid {
        return code(status);
    }
    if r < 0 {
        return None;
    }
    unsafe { libc::kill(pid, libc::SIGHUP) };
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if r == pid {
            return code(status);
        }
        if r < 0 {
            return None;
        }
    }
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, &mut status, 0);
    }
    code(status)
}

/// The program a Shell world runs: the login shell, or `command` through it.
fn program_and_args(command: Option<&str>) -> (String, Vec<String>) {
    let command = command.map(str::trim).filter(|c| !c.is_empty());
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".to_string());
        let mut args = vec!["-l".to_string()];
        if let Some(c) = command {
            args.push("-c".to_string());
            args.push(c.to_string());
        }
        (shell, args)
    }
    #[cfg(windows)]
    {
        let shell = std::env::var("COMSPEC").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "cmd.exe".to_string());
        let args = match command {
            Some(c) => vec!["/C".to_string(), c.to_string()],
            None => Vec::new(),
        };
        (shell, args)
    }
}

/// Where the shell starts: the home directory, not wherever Clay happened to be started.
fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty() && p.is_dir())
}

/// alacritty_terminal reports PTY trouble only through `log`, and nothing else in Clay
/// installs a logger, so its warnings would vanish. Only its own targets, and only at
/// warning level unless debug mode is on: with the native GUI built in, wgpu and GPUI
/// log too, and debug.log is not the place for a graphics stack's chatter.
struct ClayLogger;

impl log::Log for ClayLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("alacritty_terminal") && (m.level() <= log::Level::Warn || crate::is_debug_enabled())
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            crate::debug_log(true, &format!("shell: {} {}", record.level(), record.args()));
        }
    }

    fn flush(&self) {}
}

static LOGGER: ClayLogger = ClayLogger;

fn install_logger() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Debug);
        }
    });
}
