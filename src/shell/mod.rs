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
/// The PTY and the emulator: only in a build with the `shell-worlds` feature.
#[cfg(feature = "shell-worlds")]
mod session;
/// Re-attaching a PTY across a `/reload` (Linux only; see the module).
#[cfg(all(target_os = "linux", feature = "shell-worlds"))]
pub mod adopt;
#[cfg(all(test, feature = "shell-worlds"))]
mod tests;

pub use screen::ShellScreen;
#[cfg(feature = "shell-worlds")]
pub use session::ShellSession;

/// A build without the `shell-worlds` feature has no sessions: this type has no values,
/// so `World::shell` is always `None` and nothing behind it is compiled in. A shell world
/// still loads, shows and saves; connecting it says the build can't
/// (`App::spawn_shell_connect`).
#[cfg(not(feature = "shell-worlds"))]
pub enum ShellSession {}

#[cfg(not(feature = "shell-worlds"))]
impl ShellSession {
    pub fn is_live(&self) -> bool {
        match *self {}
    }

    pub fn size(&self) -> Size {
        match *self {}
    }
}

/// What a connect says in a build without shell worlds.
#[cfg(not(feature = "shell-worlds"))]
pub const NOT_IN_THIS_BUILD: &str = "Shell worlds are not in this build (it was built without the 'shell-worlds' feature).";

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
}
