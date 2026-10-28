//! What Clay leaves behind when it ends, for every mode (user request 2026-10-28: the GUI
//! "closed without any warning a few times", and a debug.log session that just stopped could
//! not be told from a quit). Everything here goes to `~/.clay/debug.log` whether Debug mode
//! is on or not:
//!
//! - `session pid=N mode=...` when a long-running mode starts ([`session`]);
//! - `CRASH (pid N, thread T): <panic>`, a context line and a backtrace from the one panic
//!   hook ([`install`]), then what the mode does with a panic: the console paths restart
//!   ([`restart_on_panic`], `platform::crash_restart`), the others end;
//! - `FATAL SIGNAL <n> (<name>) (pid N) vX` from an async-signal-safe handler (Unix), or
//!   `FATAL EXCEPTION 0x... (pid N) vX` from an unhandled-exception filter (Windows);
//! - `exit (<reason>) (pid N)` at every orderly end ([`finish`]; on Unix also through
//!   `atexit`, which every `std::process::exit` runs);
//! - at the next start, one line for each earlier session that ended with none of these:
//!   a marker file per running process (`~/.clay/run/<pid>`) that only an orderly exit
//!   removes. A session that crashed is also told to the user ([`take_notices`]).

// Android: the bundled server is started and stopped by the app, so no session markers.
#![cfg_attr(target_os = "android", allow(dead_code))]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

/// A panic happened in this process (any thread).
static CRASHED: AtomicBool = AtomicBool::new(false);
/// `exit` was written for this session.
static FINISHED: AtomicBool = AtomicBool::new(false);
/// The pid whose session this is; 0 before [`session`] (or in a mode with no session).
static SESSION_PID: AtomicU32 = AtomicU32::new(0);
/// The console paths: a panic restores the terminal and restarts Clay.
static RESTART_ON_PANIC: AtomicBool = AtomicBool::new(false);
/// Local time's offset from UTC in seconds, taken at start (the signal handler can't ask).
static UTC_OFFSET: AtomicI64 = AtomicI64::new(0);
static EXIT_REASON: Mutex<Option<String>> = Mutex::new(None);
static CONTEXT: Mutex<BTreeMap<&'static str, String>> = Mutex::new(BTreeMap::new());
static LAST_CRASH: Mutex<Option<String>> = Mutex::new(None);
static NOTICES: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Why the process this one replaced restarted it (`CLAY_CRASH_REASON`).
static RESTARTED_AFTER: OnceLock<Option<String>> = OnceLock::new();
static MODE: OnceLock<String> = OnceLock::new();

/// `crash_restart` hands the panic to the restarted Clay in this.
pub(crate) const CRASH_REASON_ENV: &str = "CLAY_CRASH_REASON";
/// The crash counter starts again after this long without a crash.
const CRASH_COUNT_RESET_SECS: u64 = 600;
/// How much of debug.log's end is searched for an ended session's crash line.
const LOG_TAIL_BYTES: u64 = 1024 * 1024;

fn pid() -> u32 {
    std::process::id()
}

/// First thing in `main`, every mode: the panic hook and the fatal-signal (Unix) or
/// unhandled-exception (Windows) handler, the start time for debug.log's banner, and the
/// crash counter from the environment.
pub(crate) fn install() {
    crate::STARTUP_TIME.store(now_secs() as u64, Ordering::Relaxed);
    UTC_OFFSET.store(local_offset(now_secs()), Ordering::Relaxed);
    let _ = RESTARTED_AFTER.set(std::env::var(CRASH_REASON_ENV).ok().filter(|r| !r.is_empty()));
    if std::env::var_os(CRASH_REASON_ENV).is_some() {
        std::env::remove_var(CRASH_REASON_ENV);
    }
    crate::platform::CRASH_COUNT.store(crate::platform::get_crash_count(), Ordering::SeqCst);
    install_panic_hook();
    imp::install();
}

/// The console paths (the console, `--console`, `--web-gui`): after a panic is logged, the
/// terminal is put back and Clay restarts (`platform::crash_restart`), and a fatal signal
/// puts the terminal back too.
pub(crate) fn restart_on_panic() {
    RESTART_ON_PANIC.store(true, Ordering::SeqCst);
    imp::restore_terminal_on_signal();
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        CRASHED.store(true, Ordering::SeqCst);
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").to_string();
        crate::debug_log(true, &format!("CRASH (pid {}, thread {name}): {info}", pid()));
        if let Some(context) = context_line() {
            crate::debug_log(true, &format!("CRASH CONTEXT: {context}"));
        }
        crate::debug_log(true, &format!("BACKTRACE:\n{}", std::backtrace::Backtrace::force_capture()));
        if let Ok(mut last) = LAST_CRASH.try_lock() {
            *last = Some(one_line(&info.to_string()));
        }
        if RESTART_ON_PANIC.load(Ordering::SeqCst) {
            #[cfg(not(target_os = "android"))]
            {
                use crossterm::{execute, terminal::{disable_raw_mode, LeaveAlternateScreen}};
                let _ = disable_raw_mode();
                let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
                eprintln!("\n\nClay crashed! Attempting to restart...\n");
                default_hook(info);
                crate::platform::crash_restart();
            }
            #[cfg(target_os = "android")]
            default_hook(info);
        } else {
            default_hook(info);
        }
    }));
}

/// The panic, on one line (its message may run to several).
fn one_line(text: &str) -> String {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

/// What a crash should say about the state it happened in: `key` replaced by `value`.
/// Cheap enough to call every frame (a value that didn't change is not copied).
pub(crate) fn set_context(key: &'static str, value: String) {
    if let Ok(mut ctx) = CONTEXT.try_lock() {
        if ctx.get(key) != Some(&value) {
            ctx.insert(key, value);
        }
    }
}

fn context_line() -> Option<String> {
    let ctx = CONTEXT.try_lock().ok()?;
    (!ctx.is_empty()).then(|| ctx.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "))
}

/// The panic `crash_restart` passes on to the Clay it starts.
pub(crate) fn last_crash() -> Option<String> {
    LAST_CRASH.try_lock().ok().and_then(|l| l.clone())
}

/// What a Clay restarted after a crash says (instead of "Crash recovery complete.").
pub(crate) fn restart_notice() -> String {
    match RESTARTED_AFTER.get().cloned().flatten() {
        Some(reason) => format!("Clay restarted after a crash: {}. {} has the details.", shorten(&reason, 200), log_path()),
        None => format!("Clay restarted after a crash. {} has the details.", log_path()),
    }
}

/// A long-running mode starts (`mode`: "console", "gui attached to mini", "daemon", ...):
/// the session line, the reports for earlier sessions that ended without a record, and
/// this process's own marker.
pub(crate) fn session(mode: &str) {
    let _ = MODE.set(mode.to_string());
    SESSION_PID.store(pid(), Ordering::SeqCst);
    // The WebView GUI relaunched itself after its hardware rendering aborted.
    #[cfg(target_os = "linux")]
    let note = if std::env::var_os(crate::platform::GUI_SOFTWARE_FALLBACK_ENV).is_some() { " (software rendering: the hardware rendering aborted)" } else { "" };
    #[cfg(not(target_os = "linux"))]
    let note = "";
    crate::debug_log(true, &format!("session pid={} mode={mode}{note}", pid()));
    #[cfg(not(target_os = "android"))]
    {
        let dir = crate::clay_config_path("run");
        if std::fs::create_dir_all(&dir).is_ok() {
            report_ended_sessions(&dir);
            let marker = Marker { pid: pid(), version: crate::VERSION.to_string(), mode: mode.to_string(), started: format_time(now_secs()) };
            let path = dir.join(pid().to_string());
            if std::fs::write(&path, marker.to_text()).is_ok() {
                imp::remember_marker(&path);
            }
        }
        imp::finish_at_exit();
    }
    // A Clay that has run this long without crashing earns its restarts back.
    let _ = std::thread::Builder::new().name("crash-count-reset".into()).spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(CRASH_COUNT_RESET_SECS));
        crate::platform::CRASH_COUNT.store(0, Ordering::SeqCst);
    });
    // `CLAY_CRASHLOG_TEST=panic`: a panic on the main thread's runtime two seconds in, to
    // check what a crash leaves behind end to end. Read once and cleared, so the Clay a
    // crash restart starts doesn't panic again.
    if std::env::var(TEST_ENV).as_deref() == Ok("panic") {
        std::env::remove_var(TEST_ENV);
        let _ = std::thread::Builder::new().name("crashlog-test".into()).spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            panic!("CLAY_CRASHLOG_TEST asked for this panic");
        });
    }
}

/// See `session`: a test panic on request.
const TEST_ENV: &str = "CLAY_CRASHLOG_TEST";

/// Lines for the user about earlier sessions that crashed (taken once).
pub(crate) fn take_notices() -> Vec<String> {
    NOTICES.lock().map(|mut n| std::mem::take(&mut *n)).unwrap_or_default()
}

/// Why this process is about to end, for its `exit` line.
pub(crate) fn exit_reason(reason: &str) {
    if let Ok(mut r) = EXIT_REASON.lock() {
        *r = Some(reason.to_string());
    }
}

/// `exit_reason`, unless one was given already (a SIGTERM's, before the quit it causes).
pub(crate) fn default_exit_reason(reason: &str) {
    if let Ok(mut r) = EXIT_REASON.lock() {
        r.get_or_insert_with(|| reason.to_string());
    }
}

/// The `exit` line, once, and the marker removed - unless this process crashed, when the
/// marker stays for the next start to report.
pub(crate) fn finish() {
    let session = SESSION_PID.load(Ordering::SeqCst);
    if session == 0 || session != pid() || FINISHED.swap(true, Ordering::SeqCst) {
        return;
    }
    let reason = EXIT_REASON.lock().ok().and_then(|mut r| r.take()).unwrap_or_else(|| "normal".to_string());
    if CRASHED.load(Ordering::SeqCst) {
        crate::debug_log(true, &format!("exit ({reason}, after the crash above) (pid {})", pid()));
    } else {
        crate::debug_log(true, &format!("exit ({reason}) (pid {})", pid()));
        #[cfg(not(target_os = "android"))]
        let _ = std::fs::remove_file(crate::clay_config_path("run").join(pid().to_string()));
    }
}

/// End the process for `reason` with `code`, after the `exit` line. For the exits that
/// `atexit` doesn't see (Windows' `ExitProcess`), and to say why.
pub(crate) fn exit_process(code: i32, reason: &str) -> ! {
    exit_reason(reason);
    finish();
    std::process::exit(code)
}

/// This process is about to be replaced (`exec`: a reload, `/server`, a crash restart). On
/// Unix the pid carries on and the new image's session takes the marker over; if the exec
/// fails, this process carries on too, so nothing is finished here.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn replacing(reason: &str) {
    if SESSION_PID.load(Ordering::SeqCst) == pid() {
        crate::debug_log(true, &format!("replacing this process: {reason} (pid {})", pid()));
    }
}

/// One `~/.clay/run/<pid>` marker.
#[derive(Debug, Clone, PartialEq)]
struct Marker {
    pid: u32,
    version: String,
    mode: String,
    started: String,
}

impl Marker {
    fn to_text(&self) -> String {
        format!("pid={}\nversion={}\nmode={}\nstarted={}\n", self.pid, self.version, self.mode, self.started)
    }

    fn parse(text: &str) -> Option<Marker> {
        let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('='))).map(str::to_string);
        Some(Marker { pid: get("pid")?.parse().ok()?, version: get("version").unwrap_or_default(), mode: get("mode").unwrap_or_default(), started: get("started").unwrap_or_default() })
    }
}

/// Each marker whose process is gone (and isn't this one) becomes a line in debug.log, and
/// a notice for the user when that session crashed; the marker is removed.
#[cfg(not(target_os = "android"))]
fn report_ended_sessions(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let tail = read_log_tail();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(marker) = std::fs::read_to_string(&path).ok().as_deref().and_then(Marker::parse) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if marker.pid == pid() || imp::alive(marker.pid) {
            continue;
        }
        // Removing it claims it: a second Clay starting at the same moment reports nothing.
        if std::fs::remove_file(&path).is_err() {
            continue;
        }
        let (line, notice) = ended_report(&marker, crash_line_for(&tail, marker.pid, &marker.started).as_deref());
        crate::debug_log(true, &line);
        if let Some(notice) = notice {
            if let Ok(mut n) = NOTICES.lock() {
                n.push(notice);
            }
        }
    }
}

/// Where debug.log is, as the user would type it.
fn log_path() -> String {
    crate::get_debug_log_path().display().to_string()
}

/// The debug.log line for an ended session, and the user's notice when it crashed.
fn ended_report(m: &Marker, crash: Option<&str>) -> (String, Option<String>) {
    let who = format!("Clay ({}) started {} (pid {}, v{})", m.mode, m.started, m.pid, m.version);
    match crash {
        Some(crash) => (
            format!("previous session: {who} crashed: {crash}"),
            Some(format!("A Clay session ({}) started {} closed unexpectedly: {}. {} has the details.", m.mode, m.started, shorten(crash, 200), log_path())),
        ),
        None => (
            format!(
                "previous session: {who} ended without a record: a signal (a logout, a closed terminal, kill), running out of memory, or the machine stopping. On Linux, journalctl -k --since \"{}\" shows an out-of-memory kill.",
                m.started
            ),
            None,
        ),
    }
}

/// The last `CRASH`/`FATAL` line of process `pid` at or after `since` in `tail`, without
/// its timestamp.
fn crash_line_for(tail: &str, pid: u32, since: &str) -> Option<String> {
    let crash = format!("CRASH (pid {pid},");
    let fatal = format!("(pid {pid}) v");
    tail.lines()
        .rev()
        .filter(|l| l.contains(&crash) || (l.contains("] FATAL ") && l.contains(&fatal)))
        .find(|l| l.get(1..20).is_none_or(|ts| ts >= since))
        .map(|l| l.split_once("] ").map_or(l, |(_, rest)| rest).to_string())
}

#[cfg(not(target_os = "android"))]
fn read_log_tail() -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(crate::get_debug_log_path()) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL_BYTES)));
    let mut bytes = Vec::new();
    let _ = f.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Local time minus UTC, in seconds, at `now`.
fn local_offset(now: i64) -> i64 {
    let t = crate::util::local_time_from_epoch(now);
    days_from_civil(t.year as i64, t.month as u32, t.day as u32) * 86_400 + (t.hour * 3600 + t.minute * 60 + t.second) as i64 - now
}

/// "YYYY-MM-DD HH:MM:SS", local time, as debug.log writes it.
fn format_time(epoch: i64) -> String {
    let mut b = Buf::new();
    b.time(epoch + UTC_OFFSET.load(Ordering::Relaxed));
    String::from_utf8_lossy(b.bytes()).into_owned()
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date and time of `secs` after 1970-01-01 00:00:00 (no time zone), with arithmetic
/// only, so the signal handler can use it.
fn civil_from_secs(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// A line built without allocating (the signal handler's).
struct Buf {
    b: [u8; 256],
    n: usize,
}

impl Buf {
    const fn new() -> Self {
        Buf { b: [0; 256], n: 0 }
    }

    fn push(&mut self, s: &[u8]) -> &mut Self {
        let take = s.len().min(self.b.len() - self.n);
        self.b[self.n..self.n + take].copy_from_slice(&s[..take]);
        self.n += take;
        self
    }

    fn num(&mut self, mut v: u64) -> &mut Self {
        let mut digits = [0u8; 20];
        let mut i = digits.len();
        loop {
            i -= 1;
            digits[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.push(&digits[i..])
    }

    fn hex(&mut self, v: u64, width: usize) -> &mut Self {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = [b'0'; 16];
        for (k, slot) in out.iter_mut().rev().enumerate().take(16) {
            *slot = HEX[((v >> (4 * k)) & 0xF) as usize];
        }
        self.push(&out[16 - width.min(16)..])
    }

    fn pad2(&mut self, v: u32) -> &mut Self {
        self.push(&[b'0' + (v / 10 % 10) as u8, b'0' + (v % 10) as u8])
    }

    /// "YYYY-MM-DD HH:MM:SS" for `secs` (already in the time zone wanted).
    fn time(&mut self, secs: i64) -> &mut Self {
        let (y, mo, d, h, mi, s) = civil_from_secs(secs);
        self.num(y.max(0) as u64).push(b"-").pad2(mo).push(b"-").pad2(d).push(b" ").pad2(h).push(b":").pad2(mi).push(b":").pad2(s)
    }

    fn bytes(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

/// `[time] FATAL <what> (pid N) vX` as the handlers write it.
fn fatal_line(epoch_local: i64, what: &[u8], code: Option<(u64, usize)>, pid: u32) -> Buf {
    let mut b = Buf::new();
    b.push(b"[").time(epoch_local).push(b"] FATAL ").push(what);
    if let Some((code, width)) = code {
        b.push(b" 0x").hex(code, width);
    }
    b.push(b" (pid ").num(pid as u64).push(b") v").push(crate::VERSION.as_bytes()).push(b"\n");
    b
}

#[cfg(all(unix, not(target_os = "android")))]
mod imp {
    use super::{fatal_line, pid, Buf, FINISHED, UTC_OFFSET};
    use std::cell::UnsafeCell;
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use std::sync::OnceLock;

    const FATAL: [(libc::c_int, &[u8]); 5] = [
        (libc::SIGSEGV, b"SIGNAL 11 (SIGSEGV)"),
        (libc::SIGBUS, b"SIGNAL 7 (SIGBUS)"),
        (libc::SIGILL, b"SIGNAL 4 (SIGILL)"),
        (libc::SIGFPE, b"SIGNAL 8 (SIGFPE)"),
        (libc::SIGABRT, b"SIGNAL 6 (SIGABRT)"),
    ];
    const ENDING: [(libc::c_int, &str); 2] = [(libc::SIGTERM, "SIGTERM"), (libc::SIGHUP, "SIGHUP")];

    /// debug.log, opened at start for the handlers (which may not open files).
    static LOG_FD: AtomicI32 = AtomicI32::new(-1);
    static RESTORE_TERMINAL: AtomicBool = AtomicBool::new(false);
    static STDOUT_TTY: AtomicBool = AtomicBool::new(false);
    static TERMIOS: OnceLock<libc::termios> = OnceLock::new();
    static MARKER: OnceLock<CString> = OnceLock::new();

    /// The handlers that were there before ours, one per signal in `FATAL`, written while
    /// installing and only read by the handler afterwards.
    struct Previous(UnsafeCell<[MaybeUninit<libc::sigaction>; 5]>);
    unsafe impl Sync for Previous {}
    static PREVIOUS: Previous = Previous(UnsafeCell::new([MaybeUninit::uninit(); 5]));

    pub(super) fn install() {
        if let Ok(path) = CString::new(crate::get_debug_log_path().into_os_string().into_encoded_bytes()) {
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT | libc::O_CLOEXEC, 0o644) };
            LOG_FD.store(fd, Ordering::SeqCst);
        }
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::isatty(0) == 1 && libc::tcgetattr(0, &mut t) == 0 {
                let _ = TERMIOS.set(t);
            }
            STDOUT_TTY.store(libc::isatty(1) == 1, Ordering::SeqCst);
        }
        install_fatal();
    }

    /// Ours on every fatal signal, chaining to whatever was there. Called again later by a
    /// mode whose libraries may have put their own in (the native GUI's graphics stack).
    pub(crate) fn install_fatal() {
        for (k, (sig, _)) in FATAL.iter().enumerate() {
            unsafe {
                let mut current: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(*sig, std::ptr::null(), &mut current) != 0 {
                    continue;
                }
                if current.sa_sigaction == on_fatal as *const () as usize {
                    continue;
                }
                (*PREVIOUS.0.get())[k] = MaybeUninit::new(current);
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_fatal as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(*sig, &action, std::ptr::null_mut());
            }
        }
    }

    pub(super) fn restore_terminal_on_signal() {
        RESTORE_TERMINAL.store(true, Ordering::SeqCst);
    }

    pub(super) fn remember_marker(path: &std::path::Path) {
        if let Ok(c) = CString::new(path.as_os_str().as_encoded_bytes()) {
            let _ = MARKER.set(c);
        }
    }

    pub(super) fn finish_at_exit() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            libc::atexit(at_exit);
        });
    }

    extern "C" fn at_exit() {
        super::finish();
    }

    pub(super) fn alive(pid: u32) -> bool {
        let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
        unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
    }

    fn write_log(bytes: &[u8]) {
        let fd = LOG_FD.load(Ordering::SeqCst);
        if fd >= 0 {
            unsafe {
                libc::write(fd, bytes.as_ptr().cast(), bytes.len());
            }
        }
    }

    fn now_local() -> i64 {
        (unsafe { libc::time(std::ptr::null_mut()) }) as i64 + UTC_OFFSET.load(Ordering::Relaxed)
    }

    /// Only async-signal-safe calls from here on: write, tcsetattr, sigaction, raise.
    extern "C" fn on_fatal(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        let k = FATAL.iter().position(|(s, _)| *s == sig).unwrap_or(0);
        let line = fatal_line(now_local(), FATAL[k].1, None, pid());
        write_log(line.bytes());
        if RESTORE_TERMINAL.load(Ordering::SeqCst) {
            if let Some(t) = TERMIOS.get() {
                unsafe {
                    libc::tcsetattr(0, libc::TCSANOW, t);
                }
            }
            if STDOUT_TTY.load(Ordering::SeqCst) {
                let reset = b"\x1b[?1049l\x1b[?25h\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\r\n";
                unsafe {
                    libc::write(1, reset.as_ptr().cast(), reset.len());
                }
            }
        }
        // The WebView GUI's first abort may be its hardware rendering: it relaunches with
        // software rendering instead (and never comes back here when it does).
        if sig == libc::SIGABRT {
            crate::platform::maybe_gui_software_fallback();
        }
        // The handler that was there before: Rust's stack-overflow report on SIGSEGV/SIGBUS.
        let previous = unsafe { (*PREVIOUS.0.get())[k].assume_init() };
        let handler = previous.sa_sigaction;
        if handler != libc::SIG_DFL && handler != libc::SIG_IGN && handler != on_fatal as *const () as usize {
            unsafe {
                if previous.sa_flags & libc::SA_SIGINFO != 0 {
                    let f: extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) = std::mem::transmute(handler);
                    f(sig, info, ctx);
                } else {
                    let f: extern "C" fn(libc::c_int) = std::mem::transmute(handler);
                    f(sig);
                }
            }
        }
        // Then the signal ends the process as it would have without us: a core dump, the
        // desktop's crash reporter. (Raised now, it arrives as this handler returns.)
        unsafe {
            let mut dfl: libc::sigaction = std::mem::zeroed();
            dfl.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut dfl.sa_mask);
            libc::sigaction(sig, &dfl, std::ptr::null_mut());
            libc::raise(sig);
        }
    }

    /// A mode with no SIGTERM/SIGHUP handling of its own (the attached clients) ends on one
    /// with an `exit` line rather than none. A signal ignored at start stays ignored
    /// (`nohup`).
    pub(crate) fn record_ending_signals() {
        for (sig, _) in ENDING {
            unsafe {
                let mut current: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(sig, std::ptr::null(), &mut current) != 0 || current.sa_sigaction != libc::SIG_DFL {
                    continue;
                }
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_ending as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO;
                libc::sigemptyset(&mut action.sa_mask);
                libc::sigaction(sig, &action, std::ptr::null_mut());
            }
        }
    }

    extern "C" fn on_ending(sig: libc::c_int, _: *mut libc::siginfo_t, _: *mut libc::c_void) {
        let name = ENDING.iter().find(|(s, _)| *s == sig).map_or("signal", |(_, n)| n);
        if !FINISHED.swap(true, Ordering::SeqCst) {
            let mut b = Buf::new();
            b.push(b"[").time(now_local()).push(b"] exit (").push(name.as_bytes()).push(b") (pid ").num(pid() as u64).push(b")\n");
            write_log(b.bytes());
            if let Some(marker) = MARKER.get() {
                unsafe {
                    libc::unlink(marker.as_ptr());
                }
            }
        }
        unsafe {
            let mut dfl: libc::sigaction = std::mem::zeroed();
            dfl.sa_sigaction = libc::SIG_DFL;
            libc::sigemptyset(&mut dfl.sa_mask);
            libc::sigaction(sig, &dfl, std::ptr::null_mut());
            libc::raise(sig);
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::{fatal_line, pid, UTC_OFFSET};
    use std::os::windows::io::IntoRawHandle;
    use std::sync::atomic::{AtomicIsize, Ordering};

    static LOG: AtomicIsize = AtomicIsize::new(0);

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *mut ExceptionRecord,
        address: *mut std::ffi::c_void,
        parameters: u32,
        information: [usize; 15],
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut std::ffi::c_void,
    }

    type Filter = unsafe extern "system" fn(*const ExceptionPointers) -> i32;

    extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
        fn WriteFile(file: isize, buffer: *const u8, len: u32, written: *mut u32, overlapped: *mut std::ffi::c_void) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    pub(super) fn install() {
        if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::get_debug_log_path()) {
            LOG.store(f.into_raw_handle() as isize, Ordering::SeqCst);
        }
        unsafe {
            SetUnhandledExceptionFilter(Some(on_exception));
        }
    }

    unsafe extern "system" fn on_exception(pointers: *const ExceptionPointers) -> i32 {
        let code = if pointers.is_null() || (*pointers).record.is_null() { 0 } else { (*(*pointers).record).code };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let line = fatal_line(now + UTC_OFFSET.load(Ordering::Relaxed), b"EXCEPTION", Some((code as u64, 8)), pid());
        let log = LOG.load(Ordering::SeqCst);
        if log != 0 {
            let bytes = line.bytes();
            let mut written = 0u32;
            WriteFile(log, bytes.as_ptr(), bytes.len() as u32, &mut written, std::ptr::null_mut());
        }
        0 // EXCEPTION_CONTINUE_SEARCH: Windows Error Reporting still runs
    }

    pub(super) fn restore_terminal_on_signal() {}
    pub(super) fn remember_marker(_: &std::path::Path) {}
    pub(super) fn finish_at_exit() {}
    pub(crate) fn install_fatal() {}
    pub(crate) fn record_ending_signals() {}

    pub(super) fn alive(pid: u32) -> bool {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h == 0 {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            ok && code == STILL_ACTIVE
        }
    }
}

#[cfg(target_os = "android")]
mod imp {
    pub(super) fn install() {}
    pub(super) fn restore_terminal_on_signal() {}
    #[allow(dead_code)]
    pub(crate) fn install_fatal() {}
    #[allow(dead_code)]
    pub(crate) fn record_ending_signals() {}
}

/// After a mode's libraries have started (the native GUI's graphics stack may put its own
/// fatal-signal handlers in): ours again, chaining to theirs.
#[cfg_attr(not(feature = "native-gui"), allow(dead_code))]
pub(crate) fn reinstall_fatal_handlers() {
    imp::install_fatal();
}

/// An attached client has no SIGTERM/SIGHUP handling of its own: record those as exits.
pub(crate) fn record_ending_signals() {
    imp::record_ending_signals();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_civil_date_arithmetic_matches_known_dates() {
        assert_eq!(civil_from_secs(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil_from_secs(951_782_400), (2000, 2, 29, 0, 0, 0), "a leap day");
        assert_eq!(civil_from_secs(1_793_216_504), (2026, 10, 28, 19, 41, 44), "date -u -d @1793216504");
        for secs in [0i64, 951_782_400, 1_793_216_504, 4_102_444_800] {
            let (y, m, d, h, mi, s) = civil_from_secs(secs);
            assert_eq!(days_from_civil(y, m, d) * 86_400 + (h * 3600 + mi * 60 + s) as i64, secs);
        }
    }

    #[test]
    fn the_fatal_line_is_built_without_allocating_and_reads_like_debug_log() {
        let b = fatal_line(1_793_216_504, b"SIGNAL 11 (SIGSEGV)", None, 4242);
        assert_eq!(String::from_utf8_lossy(b.bytes()), format!("[2026-10-28 19:41:44] FATAL SIGNAL 11 (SIGSEGV) (pid 4242) v{}\n", crate::VERSION));
        let e = fatal_line(0, b"EXCEPTION", Some((0xC000_0005, 8)), 7);
        assert!(String::from_utf8_lossy(e.bytes()).contains("FATAL EXCEPTION 0xC0000005 (pid 7)"));
    }

    #[test]
    fn a_marker_round_trips() {
        let m = Marker { pid: 31337, version: "1.7.25".into(), mode: "gui attached to mini".into(), started: "2026-10-27 22:39:24".into() };
        assert_eq!(Marker::parse(&m.to_text()), Some(m));
        assert_eq!(Marker::parse("junk"), None);
    }

    #[test]
    fn an_ended_session_is_reported_by_what_the_log_says_of_it() {
        let tail = "\
[2026-10-20 10:00:00] CRASH (pid 77, thread main): panicked at old.rs:1:1: an older session with the same pid
[2026-10-27 20:14:51] session pid=77 mode=gui attached to mini
[2026-10-27 22:39:15] CRASH (pid 77, thread main): panicked at src/native_gui/ui/output.rs:796:34: index out of bounds: the len is 2 but the index is 2
[2026-10-27 22:40:00] FATAL SIGNAL 11 (SIGSEGV) (pid 78) v1.7.25
";
        assert_eq!(
            crash_line_for(tail, 77, "2026-10-27 20:14:51").as_deref(),
            Some("CRASH (pid 77, thread main): panicked at src/native_gui/ui/output.rs:796:34: index out of bounds: the len is 2 but the index is 2")
        );
        assert_eq!(crash_line_for(tail, 78, "2026-10-27 22:00:00").as_deref(), Some("FATAL SIGNAL 11 (SIGSEGV) (pid 78) v1.7.25"));
        assert_eq!(crash_line_for(tail, 77, "2026-10-28 00:00:00"), None, "only lines from that session");
        assert_eq!(crash_line_for(tail, 7, "2026-10-01 00:00:00"), None, "pid 7 is not pid 77");

        let m = Marker { pid: 77, version: "1.7.24".into(), mode: "gui attached to mini".into(), started: "2026-10-27 20:14:51".into() };
        let (line, notice) = ended_report(&m, crash_line_for(tail, 77, &m.started).as_deref());
        assert!(line.starts_with("previous session: Clay (gui attached to mini) started 2026-10-27 20:14:51 (pid 77, v1.7.24) crashed: CRASH"), "{line}");
        let notice = notice.unwrap();
        assert!(notice.starts_with("A Clay session (gui attached to mini) started 2026-10-27 20:14:51 closed unexpectedly: CRASH"), "{notice}");
        assert!(notice.ends_with(&format!("{} has the details.", log_path())), "{notice}");
        let (line, notice) = ended_report(&m, None);
        assert!(line.contains("ended without a record") && line.contains("journalctl -k --since \"2026-10-27 20:14:51\""), "{line}");
        assert!(notice.is_none(), "an ending with no record (a logout, a closed terminal) is not shown as a crash");
    }

    #[test]
    fn a_panic_message_is_one_line_and_long_ones_are_cut() {
        assert_eq!(one_line("panicked at a.rs:1:2:\nindex out of bounds\n"), "panicked at a.rs:1:2: index out of bounds");
        assert_eq!(shorten("abcdef", 3), "abc…");
        assert_eq!(shorten("abc", 3), "abc");
    }
}
