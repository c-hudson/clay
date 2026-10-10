//! Re-attaching a shell world's PTY across a `/reload` (Linux).
//!
//! A reload `exec`s the same Clay image. An open file descriptor survives the exec when it
//! is not `FD_CLOEXEC`, and the child the PTY runs stays this process's child (same pid,
//! same parent) across it. So `platform::exec_reload` clears `FD_CLOEXEC` on each live
//! shell world's master fd and passes `world:fd:pid` triples to the successor, which calls
//! [`ShellSession::adopt`] to resume the terminal where it left off - a `vi` or `top` that
//! was running is still running, and repaints on the SIGWINCH the adoption sends.
//!
//! The child-exit notification uses a pidfd (Linux 5.3+): it becomes readable when the
//! adopted child exits, which is the one event alacritty's loop needs that a fresh
//! `tty::Pty` gets from its own SIGCHLD pipe. Where pidfd is unavailable (older kernels),
//! adoption fails and the world comes back disconnected - never worse than today.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};
use polling::{Event, PollMode, Poller};

// alacritty's own poll tokens are `pub(crate)`; on unix they are these fixed values, and
// its event loop matches `event.key` against them, so an `EventedPty` implemented outside
// the crate must use the same numbers (tty/unix.rs: READ_WRITE = 0, CHILD_EVENT = 1).
const PTY_READ_WRITE_TOKEN: usize = 0;
const PTY_CHILD_EVENT_TOKEN: usize = 1;

use super::{spawn_loop, ShellSession, Size};

/// A PTY master inherited across a reload, plus a pidfd watching its (still-running) child.
pub struct AdoptedPty {
    master: File,
    /// Readable once the child exits (Linux pidfd).
    pidfd: OwnedFd,
    pid: u32,
}

impl AdoptedPty {
    /// Open a pidfd for `pid` and take ownership of the inherited master `fd`, or return an
    /// error if pidfd is unavailable (the caller then leaves the world disconnected).
    fn new(fd: RawFd, pid: u32) -> io::Result<AdoptedPty> {
        let ret = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        let pidfd = unsafe { OwnedFd::from_raw_fd(ret as RawFd) };
        // The master must be non-blocking, like a freshly-made one (alacritty assumes it).
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
        }
        let master = unsafe { File::from_raw_fd(fd) };
        Ok(AdoptedPty { master, pidfd, pid })
    }
}

impl EventedReadWrite for AdoptedPty {
    type Reader = File;
    type Writer = File;

    unsafe fn register(&mut self, poll: &Arc<Poller>, mut interest: Event, poll_opts: PollMode) -> io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        unsafe {
            poll.add_with_mode(&self.master, interest, poll_opts)?;
            poll.add_with_mode(&self.pidfd, Event::readable(PTY_CHILD_EVENT_TOKEN), PollMode::Level)
        }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, mut interest: Event, poll_opts: PollMode) -> io::Result<()> {
        interest.key = PTY_READ_WRITE_TOKEN;
        poll.modify_with_mode(&self.master, interest, poll_opts)?;
        poll.modify_with_mode(&self.pidfd, Event::readable(PTY_CHILD_EVENT_TOKEN), PollMode::Level)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        poll.delete(&self.master)?;
        poll.delete(&self.pidfd)
    }

    fn reader(&mut self) -> &mut File {
        &mut self.master
    }

    fn writer(&mut self) -> &mut File {
        &mut self.master
    }
}

impl EventedPty for AdoptedPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        // The pidfd is readable: the child has exited. Reap it for the status.
        let mut status: libc::c_int = 0;
        let r = unsafe { libc::waitpid(self.pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if r == self.pid as libc::pid_t {
            use std::os::unix::process::ExitStatusExt;
            Some(ChildEvent::Exited(Some(std::process::ExitStatus::from_raw(status))))
        } else if r < 0 {
            // Already reaped (ECHILD) or gone: still an exit, status unknown.
            Some(ChildEvent::Exited(None))
        } else {
            None
        }
    }
}

impl OnResize for AdoptedPty {
    fn on_resize(&mut self, size: WindowSize) {
        let win = libc::winsize {
            ws_row: size.num_lines,
            ws_col: size.num_cols,
            ws_xpixel: size.num_cols * size.cell_width,
            ws_ypixel: size.num_lines * size.cell_height,
        };
        unsafe {
            libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &win as *const _);
        }
    }
}

impl Drop for AdoptedPty {
    fn drop(&mut self) {
        // Mirror tty::Pty::drop: hang the child up and collect it, bounded (super::end_child).
        super::end_child(self.pid);
    }
}

impl ShellSession {
    /// Re-attach a shell world's PTY (master `fd`, child `pid`) after a reload exec, as if
    /// `start` had just produced it. The grid starts empty; the caller injects a note and
    /// nudges the size so a full-screen program repaints.
    pub fn adopt(
        world: &str,
        conn_id: u64,
        fd: RawFd,
        pid: u32,
        size: Size,
        events: tokio::sync::mpsc::Sender<crate::AppEvent>,
    ) -> io::Result<ShellSession> {
        super::install_logger();
        let size = size.clamped();
        let pty = AdoptedPty::new(fd, pid)?;
        let mut session = spawn_loop(world, conn_id, pty, size, events, Some(pid))?;
        session.master_fd = fd;
        // Make the kernel's window size match (and jolt a repaint): the successor's view
        // may differ from the predecessor's, and a full-screen program only redraws on a
        // SIGWINCH.
        session.resize(Size { cols: size.cols.saturating_sub(1).max(super::MIN_COLS), rows: size.rows });
        session.resize(size);
        Ok(session)
    }
}
