//! What the graphics stack writes to stderr while the window is up goes to debug.log, not
//! the terminal the GUI was started from. On Linux, Mesa's EGL prints its probing of a GPU
//! it has no driver for ("libEGL warning: pci id for fd 36: 10de:1c03, driver (null)",
//! "failed to create dri2 screen") whenever GPUI's OpenGL backend starts - at the first
//! window, and again after a suspend loses the GPU - and lavapipe warns that it is not a
//! conformant Vulkan; none of it means anything to the user, and only some of it can be
//! turned off (EGL_LOG_LEVEL quiets the "libEGL" lines, nothing quiets the loader's).
//! Kept in debug.log, prefixed, in case it ever explains a rendering problem.
//!
//! Messages the user must see (Clay failed to start, a bad address) go to the terminal
//! with [`terminal`]. Before this process is replaced (a reload, a crash restart, /server)
//! [`release`] puts the terminal back, or the new process would write into a pipe nobody
//! reads.

#[cfg(target_os = "linux")]
mod imp {
    use std::io::{BufRead, BufReader};
    use std::os::fd::{FromRawFd, RawFd};
    use std::sync::Mutex;

    /// The terminal's stderr, while ours goes to debug.log.
    static TERMINAL: Mutex<Option<RawFd>> = Mutex::new(None);

    fn cloexec(fd: RawFd) -> bool {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) == 0 }
    }

    pub fn capture() {
        let mut terminal = TERMINAL.lock().unwrap_or_else(|e| e.into_inner());
        if terminal.is_some() {
            return;
        }
        unsafe {
            let saved = libc::fcntl(2, libc::F_DUPFD_CLOEXEC, 3);
            if saved < 0 {
                return;
            }
            let mut fds: [RawFd; 2] = [-1, -1];
            if libc::pipe(fds.as_mut_ptr()) != 0 || !cloexec(fds[0]) || !cloexec(fds[1]) || libc::dup2(fds[1], 2) < 0 {
                for fd in [fds[0], fds[1], saved] {
                    if fd >= 0 {
                        libc::close(fd);
                    }
                }
                return;
            }
            libc::close(fds[1]);
            let reader = std::fs::File::from_raw_fd(fds[0]);
            let spawned = std::thread::Builder::new().name("stderr-to-log".into()).spawn(move || {
                let mut reader = BufReader::new(reader);
                let mut line = Vec::new();
                // Ends when the last writer goes: release(), or the process.
                while matches!(reader.read_until(b'\n', &mut line), Ok(n) if n > 0) {
                    let text = String::from_utf8_lossy(&line);
                    let text = text.trim_end();
                    if !text.is_empty() {
                        log(text);
                    }
                    line.clear();
                }
            });
            if spawned.is_err() {
                // Nothing would read the pipe: writes would block once it filled.
                libc::dup2(saved, 2);
                libc::close(saved);
                return;
            }
            *terminal = Some(saved);
        }
    }

    /// One captured line into debug.log, as debug_log writes - but never onto stderr when
    /// the file can't be opened (debug_log's fallback), which would be this pipe again.
    fn log(text: &str) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(crate::get_debug_log_path()) {
            let t = crate::util::local_time_now();
            let _ = writeln!(
                file,
                "[{:04}-{:02}-{:02} {:02}:{:02}:{:02}] native-gui stderr: {text}",
                t.year, t.month, t.day, t.hour, t.minute, t.second
            );
        }
    }

    pub fn release() {
        if let Some(saved) = TERMINAL.lock().unwrap_or_else(|e| e.into_inner()).take() {
            unsafe {
                libc::dup2(saved, 2);
                libc::close(saved);
            }
        }
    }

    pub fn terminal(message: &str) {
        let terminal = TERMINAL.lock().unwrap_or_else(|e| e.into_inner());
        match *terminal {
            Some(fd) => {
                let text = format!("{message}\n");
                let mut rest = text.as_bytes();
                while !rest.is_empty() {
                    let n = unsafe { libc::write(fd, rest.as_ptr().cast(), rest.len()) };
                    if n <= 0 {
                        break;
                    }
                    rest = &rest[n as usize..];
                }
            }
            None => eprintln!("{message}"),
        }
    }
}

/// Nothing of the kind elsewhere: on macOS neither GUI prints a byte (checked on a real
/// Mac), and on Windows the GUI frees its console before the first window.
#[cfg(not(target_os = "linux"))]
mod imp {
    pub fn capture() {}
    pub fn release() {}
    pub fn terminal(message: &str) {
        eprintln!("{message}");
    }
}

/// From now on, stderr goes to debug.log.
pub fn capture() {
    imp::capture();
}

/// The terminal back on stderr (before an exec).
pub fn release() {
    imp::release();
}

/// Say `message` on the terminal, captured or not.
pub fn terminal(message: &str) {
    imp::terminal(message);
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn what_the_drivers_write_lands_in_debug_log_not_the_terminal() {
        let marker = "quiet-test: libEGL warning: pci id for fd 99";
        super::capture();
        // As a C library writes it: straight to fd 2.
        let text = format!("{marker}\n");
        let n = unsafe { libc::write(2, text.as_ptr().cast(), text.len()) };
        assert_eq!(n, text.len() as isize);
        let path = crate::get_debug_log_path();
        let mut logged = false;
        for _ in 0..100 {
            if std::fs::read_to_string(&path).is_ok_and(|log| log.contains(&format!("native-gui stderr: {marker}"))) {
                logged = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        super::release();
        assert!(logged, "the line reached debug.log");
        // The terminal is back: fd 2 is no longer the pipe (a write there isn't logged).
        let again = "quiet-test: after release\n";
        unsafe { libc::write(2, again.as_ptr().cast(), again.len()) };
        std::thread::sleep(std::time::Duration::from_millis(100));
        let log = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(!log.contains("quiet-test: after release"), "released stderr isn't captured");
    }
}
