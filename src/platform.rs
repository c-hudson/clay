//! Platform-specific and system-level functions extracted from main.rs
//! Includes: crash recovery, hot reload, TLS proxy, update checker, FD handling.
#![allow(unused_imports, unused_variables)]

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
#[cfg(any(unix, windows))]
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::io::RawFd;
#[cfg(any(all(unix, not(target_os = "android")), windows))]
use std::path::Path;

use tokio::net::TcpStream;
#[allow(unused_imports)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crossterm::{execute, terminal::{disable_raw_mode, LeaveAlternateScreen}};

use crate::{
    App, VERSION,
    debug_log, is_debug_enabled, persistence,
};

#[cfg(not(target_os = "android"))]
use crate::UpdateSuccess;

// Trust-on-first-use (TOFU) certificate verification. MUD servers (and Clay's own
// remote-console/WebView-proxy endpoints) very often present self-signed certs, so
// Clay does not validate against a CA trust root. Instead it pins the SHA-256
// fingerprint of the end-entity certificate on first connect (silently) and
// requires an exact match on every later connect to the same host:port, blocking
// the connection and surfacing old-vs-new fingerprints to the UI on a mismatch.
//
// This replaces the previous `NoCertificateVerification`, which accepted *any*
// certificate on *every* connect and also used `assertion()` for the handshake
// signature checks (i.e. it never verified the server actually held the private
// key matching the certificate it presented). That combination means an
// on-path attacker could simply replay whatever certificate they wanted —
// signature checks are what prove possession of the private key.
pub mod danger {
    use sha2::{Digest, Sha256};
    use std::sync::{Mutex, OnceLock};

    /// Details of a TLS certificate pin mismatch for `host`, stashed so the
    /// connection-failure handling path (console TUI / web / WebView) can read it
    /// and offer an explicit "Trust new certificate" action.
    #[derive(Debug, Clone)]
    pub struct CertMismatch {
        pub host: String,
        pub old_fingerprint: String,
        pub new_fingerprint: String,
    }

    static CERT_MISMATCH: OnceLock<Mutex<Option<CertMismatch>>> = OnceLock::new();

    fn cert_mismatch_slot() -> &'static Mutex<Option<CertMismatch>> {
        CERT_MISMATCH.get_or_init(|| Mutex::new(None))
    }

    /// Record a pin mismatch for a later UI read (called by both the rustls
    /// verifier below and the native-tls pin check for the MUD path).
    pub fn record_cert_mismatch(host: String, old_fingerprint: String, new_fingerprint: String) {
        if let Ok(mut slot) = cert_mismatch_slot().lock() {
            *slot = Some(CertMismatch { host, old_fingerprint, new_fingerprint });
        }
    }

    /// Take (and clear) the most recently recorded cert mismatch, if any.
    /// Connection-failure handlers call this after a TLS connect fails to learn
    /// whether it was a pin mismatch (vs. an ordinary network error) and, if so,
    /// what the old/new fingerprints were.
    pub fn take_cert_mismatch() -> Option<CertMismatch> {
        cert_mismatch_slot().lock().ok().and_then(|mut g| g.take())
    }

    pub fn sha256_hex(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Check a freshly-hashed end-entity certificate fingerprint against the pin
    /// store for `host_port`. Returns `Ok(())` if this is a first connect (and
    /// pins silently) or the fingerprint matches; returns `Err` (and records a
    /// `CertMismatch`) if it differs from the pinned value.
    ///
    /// Shared by the rustls `TofuVerifier` below and the native-tls MUD path
    /// (which checks the peer certificate post-handshake since native-tls has no
    /// pluggable verifier callback — see `platform::check_native_tls_peer_pin`).
    pub fn check_pin(host_port: &str, fingerprint: &str) -> Result<(), String> {
        match crate::persistence::get_pin(host_port) {
            None => {
                crate::persistence::add_pin(host_port, fingerprint);
                Ok(())
            }
            Some(pinned) if pinned == fingerprint => Ok(()),
            Some(pinned) => {
                record_cert_mismatch(host_port.to_string(), pinned.clone(), fingerprint.to_string());
                Err(format!(
                    "certificate for {} does not match the pinned fingerprint (was {}, now {})",
                    host_port, pinned, fingerprint
                ))
            }
        }
    }
}

// Rustls-specific TOFU verifier. Kept in its own cfg-gated module since it needs
// rustls's ServerCertVerifier trait/types; the pin logic itself (above) is shared
// with the native-tls MUD path.
#[cfg(feature = "rustls-backend")]
pub mod danger_rustls {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::crypto::CryptoProvider;
    use rustls::{DigitallySignedStruct, Error, SignatureScheme};
    use std::sync::OnceLock;
    use super::danger::{check_pin, sha256_hex};

    /// The default rustls crypto provider (ring), cached — used to perform *real*
    /// handshake signature verification (see `TofuVerifier` below) rather than
    /// rustls's `default_provider()` allocating a fresh one on every handshake.
    fn crypto_provider() -> &'static CryptoProvider {
        static PROVIDER: OnceLock<CryptoProvider> = OnceLock::new();
        PROVIDER.get_or_init(rustls::crypto::ring::default_provider)
    }

    /// Trust-on-first-use rustls `ServerCertVerifier`. Pins the SHA-256 fingerprint
    /// of the end-entity certificate presented for `host_port` on first connect;
    /// requires an exact match thereafter.
    #[derive(Debug)]
    pub struct TofuVerifier {
        host_port: String,
    }

    impl TofuVerifier {
        pub fn new(host_port: impl Into<String>) -> Self {
            Self { host_port: host_port.into() }
        }
    }

    impl ServerCertVerifier for TofuVerifier {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            let fingerprint = sha256_hex(end_entity.as_ref());
            match check_pin(&self.host_port, &fingerprint) {
                Ok(()) => Ok(ServerCertVerified::assertion()),
                Err(msg) => Err(Error::General(msg)),
            }
        }

        // CRITICAL: unlike the old NoCertificateVerification, these must perform
        // real signature verification. Certificates are sent in the clear during
        // the TLS handshake, so an attacker who has merely observed a pinned
        // certificate (e.g. from a prior legitimate connection) could replay it
        // byte-for-byte without possessing the corresponding private key. Only
        // the handshake signature (computed over the transcript with the
        // server's private key) proves the peer actually holds that key — so
        // skipping this check (as `assertion()` did) would let a replayed cert
        // sail straight through the pin comparison above.
        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            rustls::crypto::verify_tls12_signature(
                message,
                cert,
                dss,
                &crypto_provider().signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &crypto_provider().signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            crypto_provider().signature_verification_algorithms.supported_schemes()
        }
    }
}

/// Post-handshake TOFU pin check for the native-tls MUD path.
///
/// native-tls has no pluggable certificate-verifier callback like rustls's
/// `ServerCertVerifier` (that's a rustls-specific extension point), so it cannot
/// reject a mismatched certificate *during* the handshake the way `TofuVerifier`
/// does. Instead, connections still use `danger_accept_invalid_certs(true)` to
/// complete the handshake (self-signed MUD certs are expected), and this function
/// is called immediately afterwards with the peer's certificate. On a pin
/// mismatch it records the same `CertMismatch` the rustls path would and returns
/// an error; callers must treat that as a failed connection and tear the stream
/// down rather than using it. Because native-tls (unlike our rustls verifier)
/// performs its own internal handshake signature verification (it isn't skipped
/// the way the old `NoCertificateVerification` skipped rustls's), a replayed
/// certificate without the matching private key still fails the handshake before
/// this function is ever reached.
#[cfg(feature = "native-tls-backend")]
pub fn check_native_tls_peer_pin(
    host_port: &str,
    cert: Option<native_tls::Certificate>,
) -> Result<(), String> {
    let cert = match cert {
        Some(c) => c,
        None => return Err(format!("no peer certificate presented by {}", host_port)),
    };
    let der = cert
        .to_der()
        .map_err(|e| format!("could not read peer certificate for {}: {}", host_port, e))?;
    let fingerprint = danger::sha256_hex(&der);
    danger::check_pin(host_port, &fingerprint)
}

pub fn enable_tcp_keepalive(tcp_stream: &TcpStream) {
    use socket2::SockRef;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(std::time::Duration::from_secs(60))
        .with_interval(std::time::Duration::from_secs(10));
    #[cfg(unix)]
    let keepalive = keepalive.with_retries(6);
    let sock_ref = SockRef::from(tcp_stream);
    let _ = sock_ref.set_tcp_keepalive(&keepalive);
}

#[cfg(not(target_os = "android"))]
pub(crate) const RELOAD_FDS_ENV: &str = "CLAY_RELOAD_FDS";
pub(crate) const CRASH_COUNT_ENV: &str = "CLAY_CRASH_COUNT";
#[cfg(not(target_os = "android"))]
pub(crate) const MAX_CRASH_RESTARTS: u32 = 2;

// Static pointer to App for crash recovery - set when app is running
pub(crate) static APP_PTR: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());
// Track current crash count to avoid re-reading env var
pub(crate) static CRASH_COUNT: AtomicU32 = AtomicU32::new(0);

/// Get the current crash count from environment variable
pub(crate) fn get_crash_count() -> u32 {
    std::env::var(CRASH_COUNT_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Clear the crash count (called after successful operation)
pub(crate) fn clear_crash_count() {
    std::env::remove_var(CRASH_COUNT_ENV);
    CRASH_COUNT.store(0, Ordering::SeqCst);
}

/// Set the global app pointer for crash recovery
pub(crate) fn set_app_ptr(app: *mut App) {
    APP_PTR.store(app, Ordering::SeqCst);
}

/// Get the global app pointer
#[cfg(not(target_os = "android"))]
pub(crate) fn get_app_ptr() -> *mut App {
    APP_PTR.load(Ordering::SeqCst)
}

/// Attempt to restart after a crash (Unix version - uses exec)
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) fn crash_restart() {
    // Read crash count directly from env var, not from atomic
    // This ensures correct count even if crash happens before atomic is initialized
    let crash_count = std::env::var(CRASH_COUNT_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if crash_count >= MAX_CRASH_RESTARTS {
        // Already crashed too many times, don't restart
        eprintln!("Maximum crash restarts ({}) reached, not restarting.", MAX_CRASH_RESTARTS);
        return;
    }

    // Try to save state from the app pointer
    let app_ptr = get_app_ptr();
    if !app_ptr.is_null() {
        // SAFETY: We set this pointer in run_app and it remains valid until run_app returns
        let app = unsafe { &*app_ptr };

        // Try to save state
        if let Err(e) = persistence::save_reload_state(app) {
            eprintln!("Failed to save state during crash: {}", e);
        }

        // Clear CLOEXEC on socket fds so they survive exec
        for world in &app.worlds {
            if let Some(fd) = world.socket_fd {
                let _ = clear_cloexec(fd);
            }
        }

        // Pass fd list via environment
        let fds_str: String = app.worlds
            .iter()
            .filter_map(|w| w.socket_fd)
            .map(|fd| fd.to_string())
            .collect::<Vec<_>>()
            .join(",");
        std::env::set_var(RELOAD_FDS_ENV, &fds_str);
    }

    // Increment crash count in env
    let new_count = crash_count + 1;
    std::env::set_var(CRASH_COUNT_ENV, new_count.to_string());
    // Tell new process which reload file to load (PID-specific)
    std::env::set_var("CLAY_RELOAD_PID", std::process::id().to_string());

    // Try to exec the binary
    if let Ok((exe, _)) = get_executable_path() {
        use std::os::unix::process::CommandExt;
        let mut args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| a != "--reload" && a != "--crash")
            .collect();
        args.push("--crash".to_string());

        // This replaces the current process if successful
        let _ = std::process::Command::new(&exe).args(&args).exec();
    }
}

/// Attempt to restart after a crash (Windows version - uses spawn + exit)
#[cfg(windows)]
pub(crate) fn crash_restart() {
    let crash_count = std::env::var(CRASH_COUNT_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if crash_count >= MAX_CRASH_RESTARTS {
        eprintln!("Maximum crash restarts ({}) reached, not restarting.", MAX_CRASH_RESTARTS);
        return;
    }

    let app_ptr = get_app_ptr();
    if !app_ptr.is_null() {
        let app = unsafe { &*app_ptr };

        if let Err(e) = persistence::save_reload_state(app) {
            eprintln!("Failed to save state during crash: {}", e);
        }

        // Make socket handles inheritable so they survive into the child process
        for world in &app.worlds {
            if let Some(handle) = world.socket_fd {
                let _ = make_inheritable(handle as u64);
            }
        }

        let fds_str: String = app.worlds
            .iter()
            .filter_map(|w| w.socket_fd)
            .map(|fd| fd.to_string())
            .collect::<Vec<_>>()
            .join(",");
        std::env::set_var(RELOAD_FDS_ENV, &fds_str);
    }

    let new_count = crash_count + 1;
    std::env::set_var(CRASH_COUNT_ENV, new_count.to_string());
    std::env::set_var("CLAY_RELOAD_PID", std::process::id().to_string());

    if let Ok((exe, _)) = get_executable_path() {
        let mut args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| a != "--reload" && a != "--crash")
            .collect();
        args.push("--crash".to_string());

        if std::process::Command::new(&exe).args(&args).spawn().is_ok() {
            std::process::exit(1);
        }
    }
}

/// Set up the crash handler (panic hook)
#[cfg(not(target_os = "android"))]
pub(crate) fn setup_crash_handler() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal first to ensure output is visible
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);

        // Log crash to debug file (always log crashes regardless of debug setting)
        let panic_msg = format!("CRASH: {}", panic_info);
        debug_log(is_debug_enabled(), &panic_msg);

        // Also log backtrace if available
        let backtrace = std::backtrace::Backtrace::capture();
        let bt_str = format!("{}", backtrace);
        if !bt_str.is_empty() && !bt_str.contains("disabled") {
            debug_log(is_debug_enabled(), &format!("BACKTRACE:\n{}", bt_str));
        }

        // Print the panic info using the default handler
        eprintln!("\n\nClay crashed! Attempting to restart...\n");
        default_hook(panic_info);

        // Attempt to restart
        crash_restart();

        // If we get here, restart failed - exit normally
    }));
}

// Hot reload helper - clear FD_CLOEXEC on Unix so fd survives exec
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) fn clear_cloexec(fd: RawFd) -> io::Result<()> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        let new_flags = flags & !libc::FD_CLOEXEC;
        if libc::fcntl(fd, libc::F_SETFD, new_flags) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

// Hot reload helper - make socket handle inheritable on Windows so it survives into child process
#[cfg(windows)]
pub(crate) fn make_inheritable(socket: u64) -> io::Result<()> {
    const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
    extern "system" {
        fn SetHandleInformation(hObject: isize, dwMask: u32, dwFlags: u32) -> i32;
    }
    let handle = socket as isize;
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Check if a process with the given PID is still alive
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) fn is_process_alive(pid: u32) -> bool {
    // Use waitpid with WNOHANG to check without blocking
    // A return of 0 means the process is still running
    // A return of -1 with ECHILD means the process doesn't exist (not our child)
    // Use kill with signal 0 instead - this works for any process we can signal
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
    }
}

/// Stub for Android — TLS proxy not supported on Android
#[cfg(target_os = "android")]
pub(crate) fn is_process_alive(_pid: u32) -> bool {
    false
}

/// Windows implementation using OpenProcess + GetExitCodeProcess
#[cfg(windows)]
pub(crate) fn is_process_alive(pid: u32) -> bool {
    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
        fn GetExitCodeProcess(hProcess: isize, lpExitCode: *mut u32) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return false;
        }
        let mut exit_code: u32 = 0;
        let result = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        result != 0 && exit_code == STILL_ACTIVE
    }
}

/// Reap any zombie child processes to prevent defunct processes from accumulating.
/// This should be called periodically from the main event loop.
#[cfg(all(unix, not(target_os = "android")))]
pub fn reap_zombie_children() {
    // Call waitpid with -1 (any child) and WNOHANG (don't block) to reap zombies
    // Keep calling until no more zombies are found
    unsafe {
        loop {
            let mut status: libc::c_int = 0;
            let result = libc::waitpid(-1, &mut status, libc::WNOHANG);
            if result <= 0 {
                // No more zombies to reap (0 = no status available, -1 = error/no children)
                break;
            }
            // Successfully reaped a zombie, continue to check for more
        }
    }
}

/// Generate a unique socket path for the TLS proxy.
/// Uses $XDG_RUNTIME_DIR (typically /run/user/<uid> with mode 0700) for security,
/// falling back to /tmp if not available.
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) fn get_proxy_socket_path(world_name: &str) -> PathBuf {
    let sanitized_name = world_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();

    // Prefer XDG_RUNTIME_DIR for security (typically /run/user/<uid> with mode 0700)
    let base_dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("/tmp"));

    base_dir.join(format!(
        "clay-tls-{}-{}.sock",
        std::process::id(),
        sanitized_name
    ))
}

/// Get the config file path for a TLS proxy (derived from socket path)
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) fn get_proxy_config_path(socket_path: &Path) -> PathBuf {
    let mut config_path = socket_path.to_path_buf();
    config_path.set_extension("conf");
    config_path
}

/// Spawn a TLS proxy process for a world connection.
/// Returns (proxy_pid, socket_path) on success.
/// The proxy process handles the TLS connection to the MUD server and exposes
/// a Unix socket for the main client to connect to.
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) fn spawn_tls_proxy(
    world_name: &str,
    host: &str,
    port: &str,
) -> io::Result<(u32, PathBuf)> {
    use std::process::{Command, Stdio};
    use std::io::Write;

    let socket_path = get_proxy_socket_path(world_name);
    let config_path = get_proxy_config_path(&socket_path);

    // Remove any existing socket and config files
    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&config_path);

    // Write connection info to config file (keeps host:port out of process list)
    {
        let mut file = std::fs::File::create(&config_path)?;
        writeln!(file, "{}:{}", host, port)?;
        writeln!(file, "{}", socket_path.display())?;
    }

    // Get the current executable path
    let exe_path = std::env::current_exe()?;

    // Spawn the proxy with just the config file path (no host:port visible in ps)
    let proxy_arg = format!("--tls-proxy={}", config_path.display());

    let child = Command::new(&exe_path)
        .arg(&proxy_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let child_pid = child.id();

    // Wait up to 10 seconds for the socket to appear
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(10);

    while start.elapsed() < timeout {
        if socket_path.exists() {
            // Socket exists, proxy is ready
            return Ok((child_pid, socket_path));
        }

        // Check if child process died
        if !is_process_alive(child_pid) {
            return Err(io::Error::other(
                "TLS proxy process exited unexpectedly",
            ));
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Timeout - kill the child and return error
    unsafe {
        libc::kill(child_pid as libc::pid_t, libc::SIGTERM);
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "TLS proxy socket not created in time",
    ))
}

/// Async implementation of the TLS proxy main loop (runs in separate process via --tls-proxy)
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) async fn run_tls_proxy_async(host: &str, port: &str, socket_path: &PathBuf) {
    use tokio::net::UnixListener;

    // Ignore SIGUSR1 - the main clay process uses this for reload, but the proxy
    // should not be affected by reload signals (it stays running across reloads)
    unsafe {
        libc::signal(libc::SIGUSR1, libc::SIG_IGN);
    }

    // Step 1: Connect to the MUD server with TLS
    let tcp_stream = match TcpStream::connect(format!("{}:{}", host, port)).await {
        Ok(s) => s,
        Err(_) => return,
    };

    // Enable TCP keepalive to detect dead connections faster
    enable_tcp_keepalive(&tcp_stream);

    // Establish TLS connection
    #[cfg(feature = "rustls-backend")]
    let tls_stream = {
        use rustls::RootCertStore;
        use tokio_rustls::TlsConnector;
        use rustls::pki_types::ServerName;

        // Create a config that accepts invalid certs (common for MUD servers)
        let mut root_store = RootCertStore::empty();
        root_store.roots = webpki_roots::TLS_SERVER_ROOTS.to_vec();

        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(danger_rustls::TofuVerifier::new(format!("{}:{}", host, port))))
            .with_no_client_auth();

        let connector = TlsConnector::from(Arc::new(config));
        let server_name = match ServerName::try_from(host.to_string()) {
            Ok(sn) => sn,
            Err(_) => return,
        };

        match connector.connect(server_name, tcp_stream).await {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    #[cfg(feature = "native-tls-backend")]
    let tls_stream = {
        let connector = match native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let connector = tokio_native_tls::TlsConnector::from(connector);

        match connector.connect(host, tcp_stream).await {
            Ok(s) => {
                let peer_cert = s.get_ref().peer_certificate().ok().flatten();
                if check_native_tls_peer_pin(&format!("{}:{}", host, port), peer_cert).is_err() {
                    return;
                }
                s
            }
            Err(_) => return,
        }
    };

    #[cfg(not(any(feature = "native-tls-backend", feature = "rustls-backend")))]
    {
        // No TLS backend available
        return;
    }

    // Step 2: Create Unix socket listener with restrictive permissions
    // Set umask to 0o077 before binding so socket is created with 0o600 atomically
    // (prevents race condition where socket exists briefly with permissive mode)
    let old_umask = unsafe { libc::umask(0o077) };
    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => {
            unsafe { libc::umask(old_umask) }; // Restore original umask
            l
        }
        Err(_) => {
            unsafe { libc::umask(old_umask) }; // Restore original umask
            return;
        }
    };

    // Get our UID for peer credential verification
    let our_uid = unsafe { libc::getuid() };

    // Step 3: Main loop - accept clients and relay data
    // Supports reconnection: when client disconnects, wait for new client (for hot reload)
    let (mut tls_read, mut tls_write) = tokio::io::split(tls_stream);

    loop {
        // Wait for client connection with timeout (60 seconds for reconnection)
        let client_stream = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            listener.accept()
        ).await {
            Ok(Ok((stream, _))) => stream,
            Ok(Err(_)) => break,
            Err(_) => break,
        };

        // Verify peer credentials - only accept connections from the same user
        // This prevents other users from hijacking the connection
        let peer_uid = {
            use std::os::unix::io::AsRawFd;
            let fd = client_stream.as_raw_fd();

            #[cfg(target_os = "linux")]
            {
                let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
                let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
                let ret = unsafe {
                    libc::getsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_PEERCRED,
                        &mut cred as *mut _ as *mut libc::c_void,
                        &mut len,
                    )
                };
                if ret == 0 { Some(cred.uid) } else { None }
            }

            #[cfg(target_os = "macos")]
            {
                let mut euid: libc::uid_t = 0;
                let mut egid: libc::gid_t = 0;
                let ret = unsafe { libc::getpeereid(fd, &mut euid, &mut egid) };
                if ret == 0 { Some(euid) } else { None }
            }

            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                None // Other Unix platforms: skip peer check
            }
        };

        // Reject connections from different users
        if peer_uid != Some(our_uid) {
            // Close connection silently and wait for next
            continue;
        }

        let (mut client_read, mut client_write) = client_stream.into_split();

        // Track why we exited the relay loop
        let mut tls_server_disconnected = false;

        // Relay data between client and TLS server
        let mut client_buf = [0u8; 8192];
        let mut tls_buf = [0u8; 8192];

        loop {
            tokio::select! {
                // Client -> TLS
                result = client_read.read(&mut client_buf) => {
                    match result {
                        Ok(0) => break, // Client disconnected, wait for new client
                        Ok(n) => {
                            if tls_write.write_all(&client_buf[..n]).await.is_err() {
                                tls_server_disconnected = true;
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                // TLS -> Client
                result = tls_read.read(&mut tls_buf) => {
                    match result {
                        Ok(0) => {
                            tls_server_disconnected = true;
                            break;
                        }
                        Ok(n) => {
                            if client_write.write_all(&tls_buf[..n]).await.is_err() {
                                break; // Client write failed, wait for new client
                            }
                        }
                        Err(_) => {
                            tls_server_disconnected = true;
                            break;
                        }
                    }
                }
            }
        }

        // If TLS server disconnected, exit the proxy
        if tls_server_disconnected {
            break;
        }
        // Otherwise, loop back to accept a new client (for hot reload)
    }

    // Clean up socket file
    let _ = std::fs::remove_file(socket_path);
}

/// Windows: generate a Named Pipe path for the TLS proxy.
/// Format: \\.\pipe\clay-tls-<pid>-<sanitized_worldname>
#[cfg(windows)]
pub(crate) fn get_proxy_socket_path(world_name: &str) -> PathBuf {
    let sanitized_name = world_name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>();
    PathBuf::from(format!(r"\\.\pipe\clay-tls-{}-{}", std::process::id(), sanitized_name))
}

/// Windows: get config file path for a TLS proxy (stored in %TEMP%).
#[cfg(windows)]
pub(crate) fn get_proxy_config_path(pipe_path: &Path) -> PathBuf {
    let pipe_file = pipe_path
        .components()
        .last()
        .and_then(|c: std::path::Component| c.as_os_str().to_str())
        .unwrap_or("clay-tls-proxy");
    let temp_dir = std::env::var("TEMP")
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| "C:\\Temp".to_string());
    PathBuf::from(format!("{}\\{}.conf", temp_dir, pipe_file))
}

/// Windows: spawn a TLS proxy process and wait for its Named Pipe to become available.
/// Returns (proxy_pid, pipe_path) on success.
#[cfg(windows)]
pub(crate) fn spawn_tls_proxy(
    world_name: &str,
    host: &str,
    port: &str,
) -> io::Result<(u32, PathBuf)> {
    use std::process::{Command, Stdio};
    use std::io::Write;

    let pipe_path = get_proxy_socket_path(world_name);
    let config_path = get_proxy_config_path(&pipe_path);

    let _ = std::fs::remove_file(&config_path);

    {
        let mut file = std::fs::File::create(&config_path)?;
        writeln!(file, "{}:{}", host, port)?;
        writeln!(file, "{}", pipe_path.display())?;
    }

    let exe_path = std::env::current_exe()?;
    let proxy_arg = format!("--tls-proxy={}", config_path.display());

    // Spawn with CREATE_BREAKAWAY_FROM_JOB so the proxy survives when the parent
    // process exits during hot reload (prevents job-object-based termination).
    // Fall back to normal spawn if the job doesn't permit breakaway.
    use std::os::windows::process::CommandExt;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let child = Command::new(&exe_path)
        .arg(&proxy_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW)
        .spawn()
        .or_else(|_| {
            Command::new(&exe_path)
                .arg(&proxy_arg)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
        })?;

    let child_pid = child.id();

    // Wait up to 10 seconds for the named pipe server to start listening.
    // Use WaitNamedPipeA which checks availability WITHOUT connecting — using
    // OpenOptions::open would consume the server's pending connection slot and
    // cause a race where the subsequent real connection attempt fails.
    extern "system" {
        fn WaitNamedPipeA(lpNamedPipeName: *const u8, nTimeOut: u32) -> i32;
    }
    const NMPWAIT_NOWAIT: u32 = 1;

    let pipe_str = pipe_path.to_str().unwrap_or("").to_string();
    let pipe_cstr = format!("{}\0", pipe_str);
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(10);

    while start.elapsed() < timeout {
        if unsafe { WaitNamedPipeA(pipe_cstr.as_ptr(), NMPWAIT_NOWAIT) } != 0 {
            return Ok((child_pid, pipe_path));
        }

        if !is_process_alive(child_pid) {
            return Err(io::Error::other("TLS proxy process exited unexpectedly"));
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Timeout — kill the proxy and report error
    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
        fn TerminateProcess(hProcess: isize, uExitCode: u32) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }
    const PROCESS_TERMINATE: u32 = 0x0001;
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, child_pid);
        if handle != 0 {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "TLS proxy named pipe not ready in time"))
}

/// Windows: async TLS proxy main loop. Connects to the MUD server with TLS,
/// then accepts Named Pipe clients (one at a time) and relays data.
/// Survives hot reload because it is a separate process.
#[cfg(windows)]
pub(crate) async fn run_tls_proxy_async(host: &str, port: &str, pipe_path: &PathBuf) {
    use tokio::net::windows::named_pipe::{ServerOptions, PipeMode};

    debug_log(is_debug_enabled(), &format!("TLS-PROXY: starting for {}:{} pipe={}", host, port, pipe_path.display()));

    // Step 1: Connect to the MUD server with TLS
    let tcp_stream = match TcpStream::connect(format!("{}:{}", host, port)).await {
        Ok(s) => s,
        Err(e) => { debug_log(is_debug_enabled(), &format!("TLS-PROXY: TCP connect failed: {}", e)); return; }
    };

    enable_tcp_keepalive(&tcp_stream);

    // Establish TLS connection (same backends as Unix version)
    #[cfg(feature = "rustls-backend")]
    let tls_stream = {
        use rustls::RootCertStore;
        use tokio_rustls::TlsConnector;
        use rustls::pki_types::ServerName;

        let mut root_store = RootCertStore::empty();
        root_store.roots = webpki_roots::TLS_SERVER_ROOTS.to_vec();

        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(danger_rustls::TofuVerifier::new(format!("{}:{}", host, port))))
            .with_no_client_auth();

        let connector = TlsConnector::from(Arc::new(config));
        let server_name = match ServerName::try_from(host.to_string()) {
            Ok(sn) => sn,
            Err(_) => return,
        };

        match connector.connect(server_name, tcp_stream).await {
            Ok(s) => s,
            Err(_) => return,
        }
    };

    #[cfg(feature = "native-tls-backend")]
    let tls_stream = {
        let connector = match native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let connector = tokio_native_tls::TlsConnector::from(connector);

        match connector.connect(host, tcp_stream).await {
            Ok(s) => {
                let peer_cert = s.get_ref().peer_certificate().ok().flatten();
                if check_native_tls_peer_pin(&format!("{}:{}", host, port), peer_cert).is_err() {
                    return;
                }
                s
            }
            Err(_) => return,
        }
    };

    #[cfg(not(any(feature = "native-tls-backend", feature = "rustls-backend")))]
    {
        return;
    }

    debug_log(is_debug_enabled(), "TLS-PROXY: TLS handshake complete");

    let pipe_name = match pipe_path.to_str() {
        Some(s) => s.to_string(),
        None => return,
    };

    // Step 2: Named Pipe server loop — accept one client at a time, relay data
    let (mut tls_read, mut tls_write) = tokio::io::split(tls_stream);
    let mut first_instance = true;

    loop {
        // Create a new server instance for each client connection
        let server = {
            let mut opts = ServerOptions::new();
            opts.pipe_mode(PipeMode::Byte)
                .in_buffer_size(8192)
                .out_buffer_size(8192);
            if first_instance {
                opts.first_pipe_instance(true);
            }
            match opts.create(&pipe_name) {
                Ok(s) => { debug_log(is_debug_enabled(), &format!("TLS-PROXY: pipe server instance created (first={})", first_instance)); s }
                Err(e) => { debug_log(is_debug_enabled(), &format!("TLS-PROXY: pipe create failed: {}", e)); break; }
            }
        };
        first_instance = false;

        // Wait for a client to connect (60s timeout for reconnection after hot reload)
        match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            server.connect(),
        ).await {
            Ok(Ok(())) => {}
            _ => break,
        }

        let (mut client_read, mut client_write) = tokio::io::split(server);
        let mut tls_server_disconnected = false;

        let mut client_buf = [0u8; 8192];
        let mut tls_buf = [0u8; 8192];

        loop {
            tokio::select! {
                result = tokio::io::AsyncReadExt::read(&mut client_read, &mut client_buf) => {
                    match result {
                        Ok(0) => break,
                        Ok(n) => {
                            if tokio::io::AsyncWriteExt::write_all(&mut tls_write, &client_buf[..n]).await.is_err() {
                                tls_server_disconnected = true;
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                result = tokio::io::AsyncReadExt::read(&mut tls_read, &mut tls_buf) => {
                    match result {
                        Ok(0) => { tls_server_disconnected = true; break; }
                        Ok(n) => {
                            if tokio::io::AsyncWriteExt::write_all(&mut client_write, &tls_buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => { tls_server_disconnected = true; break; }
                    }
                }
            }
        }

        if tls_server_disconnected {
            break;
        }
        // Client disconnected (hot reload) — loop to accept new client
    }
}

/// Windows: terminate a proxy process by PID.
#[cfg(windows)]
pub(crate) fn kill_proxy_process(pid: u32) {
    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
        fn TerminateProcess(hProcess: isize, uExitCode: u32) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }
    const PROCESS_TERMINATE: u32 = 0x0001;
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle != 0 {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

/// Windows: async helper that waits up to `timeout_secs` for the proxy's Named Pipe
/// to become available and then connects to it.  Uses WaitNamedPipeA with
/// NMPWAIT_NOWAIT between async sleeps so the tokio thread is never blocked.
#[cfg(windows)]
pub(crate) async fn connect_to_proxy_pipe(
    pipe_path: &std::path::Path,
    timeout_secs: u64,
) -> Option<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;
    extern "system" {
        fn WaitNamedPipeA(lpNamedPipeName: *const u8, nTimeOut: u32) -> i32;
    }
    const NMPWAIT_NOWAIT: u32 = 1;

    let pipe_name = pipe_path.to_str().unwrap_or("").to_string();
    let pipe_cstr = format!("{}\0", pipe_name);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

    while std::time::Instant::now() < deadline {
        let pipe_available = unsafe { WaitNamedPipeA(pipe_cstr.as_ptr(), NMPWAIT_NOWAIT) } != 0;
        if pipe_available {
            match ClientOptions::new().open(&pipe_name) {
                Ok(client) => return Some(client),
                Err(_) => {} // Pipe became busy between wait and open — retry
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    None
}

/// Strip " (deleted)" suffix from a path string if present.
#[cfg(all(unix, not(target_os = "android")))]
pub(crate) fn strip_deleted_suffix(path_str: &str) -> String {
    // Try common variations of the deleted marker
    for suffix in [" (deleted)", "(deleted)"] {
        if let Some(stripped) = path_str.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    path_str.to_string()
}

/// Get the executable path, handling the case where binary was updated.
/// On Linux, if the binary was replaced, /proc/self/exe shows " (deleted)".
/// We strip that suffix to get the path to the new binary.
/// Returns (path, debug_info) for better error messages.
#[cfg(all(unix, not(target_os = "android")))]
pub fn get_executable_path() -> io::Result<(PathBuf, String)> {
    let proc_exe = PathBuf::from("/proc/self/exe");
    let link_target = std::fs::read_link(&proc_exe)?;
    let target_str = link_target.to_string_lossy().to_string();
    let clean_path = strip_deleted_suffix(&target_str);
    let debug_info = format!(
        "raw='{}', cleaned='{}', exists={}",
        target_str,
        clean_path,
        PathBuf::from(&clean_path).exists()
    );
    Ok((PathBuf::from(clean_path), debug_info))
}

/// Get the executable path on Windows.
/// Windows locks running executables, but spawn+exit handles this naturally.
#[cfg(windows)]
pub fn get_executable_path() -> io::Result<(PathBuf, String)> {
    let exe = std::env::current_exe()?;
    let debug_info = format!("path='{}', exists={}", exe.display(), exe.exists());
    Ok((exe, debug_info))
}

/// Get the correct GitHub release asset name for this platform.
/// These names must match exactly what the /release skill uploads to GitHub.
#[cfg(not(target_os = "android"))]
pub(crate) fn get_platform_asset_name() -> &'static str {
    #[cfg(all(target_os = "linux", target_env = "musl"))]
    { "clay-linux-x86_64-musl" }

    #[cfg(all(target_os = "linux", not(target_env = "musl")))]
    { "clay-linux-x86_64-gui" }

    #[cfg(target_os = "macos")]
    { "clay-macos-universal" }

    #[cfg(target_os = "windows")]
    { "clay-windows-x86_64.exe" }

    #[cfg(target_os = "android")]
    { "clay-termux-aarch64" }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android")))]
    { "unknown" }
}

/// Returns true if the given asset name is a plausible match for this platform.
/// Used as a fallback when the exact asset name doesn't match, to give a better
/// error message listing what IS available.
#[cfg(not(target_os = "android"))]
fn is_platform_asset_candidate(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    { name.contains("linux") }
    #[cfg(target_os = "macos")]
    { name.contains("macos") || name.contains("mac") || name.contains("darwin") }
    #[cfg(target_os = "windows")]
    { name.ends_with(".exe") }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    { let _ = name; false }
}

/// Clean up any leftover `.old` file from a previous Windows self-replace.
/// Called once at startup on Windows; silently ignored if the file doesn't exist.
#[cfg(target_os = "windows")]
pub fn cleanup_old_exe() {
    if let Ok((exe_path, _)) = get_executable_path() {
        let old_path = exe_path.with_extension("exe.old");
        let _ = std::fs::remove_file(&old_path);
    }
}

/// Compare two semver-style version strings, returns true if remote is newer than current.
/// Handles: "1.0.0" vs "1.0.1", pre-release suffixes like "-alpha", "-beta", "-rc1"
#[cfg(any(not(target_os = "android"), test))]
pub(crate) fn is_newer_version(remote: &str, current: &str) -> bool {
    // Split off pre-release suffix (e.g. "1.0.0-alpha" → "1.0.0", "alpha")
    fn split_version(v: &str) -> (Vec<u64>, Option<&str>) {
        let (base, pre) = if let Some(idx) = v.find('-') {
            (&v[..idx], Some(&v[idx + 1..]))
        } else {
            (v, None)
        };
        let parts: Vec<u64> = base.split('.').filter_map(|p| p.parse().ok()).collect();
        (parts, pre)
    }

    let (remote_parts, remote_pre) = split_version(remote);
    let (current_parts, current_pre) = split_version(current);

    // Compare numeric parts
    let max_len = remote_parts.len().max(current_parts.len());
    for i in 0..max_len {
        let r = remote_parts.get(i).copied().unwrap_or(0);
        let c = current_parts.get(i).copied().unwrap_or(0);
        if r > c { return true; }
        if r < c { return false; }
    }

    // Same base version: pre-release is older than release
    // remote has no pre-release, current does → remote is newer (release > pre-release)
    if remote_pre.is_none() && current_pre.is_some() {
        return true;
    }

    false
}

/// Check GitHub for latest release and download the binary if newer
#[cfg(not(target_os = "android"))]
pub async fn check_and_download_update(force: bool) -> Result<UpdateSuccess, String> {
    let client = reqwest::Client::builder()
        .user_agent("clay-mud-client")
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // Try /releases/latest first (works for non-prerelease tags), then fall back to
    // /releases (includes pre-releases like v1.0.0-beta)
    let release: serde_json::Value = {
        let resp = client
            .get("https://api.github.com/repos/c-hudson/clay/releases/latest")
            .send()
            .await
            .map_err(|e| format!("Failed to check for updates: {}", e))?;

        if resp.status().is_success() {
            resp.json().await
                .map_err(|e| format!("Invalid response: {}", e))?
        } else {
            // /latest returned 404 (pre-release only) — try listing all releases
            let releases: serde_json::Value = client
                .get("https://api.github.com/repos/c-hudson/clay/releases?per_page=1")
                .send()
                .await
                .map_err(|e| format!("Failed to check for updates: {}", e))?
                .json()
                .await
                .map_err(|e| format!("Invalid response: {}", e))?;

            releases.as_array()
                .and_then(|a| a.first().cloned())
                .ok_or("No releases found on GitHub")?
        }
    };

    // Extract version from tag_name (strip leading 'v')
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| format!("No version tag in release (keys: {:?})",
            release.as_object().map(|o| o.keys().take(5).collect::<Vec<_>>())))?;
    let remote_version = tag.trim_start_matches('v');

    // Compare versions
    if !force && !is_newer_version(remote_version, VERSION) {
        return Err(format!(
            "Already up to date (current: v{}, latest: v{})",
            VERSION, remote_version
        ));
    }

    // Find correct asset for this platform.
    // First try an exact name match; if that fails, fall back to a platform predicate
    // so a future rename in the release skill doesn't produce a cryptic error.
    let asset_name = get_platform_asset_name();
    let assets = release["assets"]
        .as_array()
        .ok_or("No assets in release")?;
    let asset = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(asset_name))
        .or_else(|| assets.iter().find(|a| {
            a["name"].as_str().is_some_and(is_platform_asset_candidate)
        }))
        .ok_or_else(|| {
            let available: Vec<&str> = assets.iter()
                .filter_map(|a| a["name"].as_str())
                .collect();
            format!("No binary for this platform ({}) in release. Available: [{}]",
                asset_name, available.join(", "))
        })?;

    let download_url = asset["browser_download_url"]
        .as_str()
        .ok_or("No download URL for asset")?;
    let expected_size = asset["size"].as_u64().unwrap_or(0);

    // Download to temp file
    let temp_path = std::env::temp_dir().join(format!("clay-update-{}", remote_version));
    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Download incomplete: {}", e))?;

    // Validate download size
    if expected_size > 0 && bytes.len() as u64 != expected_size {
        return Err(format!(
            "Download size mismatch (got {} bytes, expected {})",
            bytes.len(),
            expected_size
        ));
    }

    // Minimum size sanity check (Clay binary should be at least 1MB)
    if bytes.len() < 1_000_000 {
        return Err(format!(
            "Downloaded file too small ({} bytes) - likely not a valid binary",
            bytes.len()
        ));
    }

    // Check ELF header (Linux), Mach-O header (macOS), or PE header (Windows)
    #[cfg(target_os = "linux")]
    if bytes.len() >= 4 && &bytes[0..4] != b"\x7fELF" {
        return Err("Downloaded file is not a valid ELF binary".to_string());
    }
    #[cfg(target_os = "macos")]
    if bytes.len() >= 4 {
        let magic = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        // Mach-O magic numbers: MH_MAGIC_64, MH_CIGAM_64, FAT_MAGIC, FAT_CIGAM
        if !matches!(magic, 0xFEEDFACF | 0xCFFAEDFE | 0xBEBAFECA | 0xCAFEBABE) {
            return Err("Downloaded file is not a valid Mach-O binary".to_string());
        }
    }
    #[cfg(target_os = "windows")]
    if bytes.len() >= 2 && &bytes[0..2] != b"MZ" {
        return Err("Downloaded file is not a valid Windows executable".to_string());
    }

    // Write to temp file
    std::fs::write(&temp_path, &bytes)
        .map_err(|e| format!("Failed to write update: {}", e))?;

    Ok(UpdateSuccess {
        version: remote_version.to_string(),
        temp_path,
    })
}

#[cfg(all(unix, not(target_os = "android")))]
pub fn exec_reload(app: &mut App) -> io::Result<()> {
    // Always log reload (not gated by debug flag) so we can trace issues
    debug_log(is_debug_enabled(), "RELOAD: Starting exec_reload");

    // Save the current state
    debug_log(is_debug_enabled(), "RELOAD: Saving state...");
    persistence::save_reload_state(app)?;
    debug_log(is_debug_enabled(), "RELOAD: State saved successfully");

    // Collect socket fds that need to survive exec (plain TCP only)
    // TLS proxy connections reconnect via Unix socket path after reload
    let mut fds_to_keep: Vec<RawFd> = Vec::new();
    for world in &app.worlds {
        // Plain TCP socket - skip if FD is stale (connection closed but fd not cleared)
        if let Some(fd) = world.socket_fd {
            if clear_cloexec(fd).is_ok() {
                fds_to_keep.push(fd);
            }
            // If clear_cloexec fails, the FD is stale - just skip it
        }
    }
    debug_log(is_debug_enabled(), &format!("RELOAD: Keeping {} fds", fds_to_keep.len()));

    // Get the executable path with debug info
    let (exe, debug_info) = get_executable_path()?;
    debug_log(is_debug_enabled(), &format!("RELOAD: Executable path: {} ({})", exe.display(), debug_info));

    // Verify the executable exists
    if !exe.exists() {
        return Err(io::Error::other(format!(
            "Executable not found. Debug: {}",
            debug_info
        )));
    }

    // Pass fd list via environment (fds must survive exec)
    let fds_str: String = fds_to_keep
        .iter()
        .map(|fd| fd.to_string())
        .collect::<Vec<_>>()
        .join(",");
    std::env::set_var(RELOAD_FDS_ENV, &fds_str);
    // Tell new process which reload file to load (PID-specific)
    std::env::set_var("CLAY_RELOAD_PID", std::process::id().to_string());

    // Execute the new binary with --reload argument
    use std::os::unix::process::CommandExt;
    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--reload" && a != "--crash").collect();
    args.push("--reload".to_string());
    debug_log(is_debug_enabled(), &format!("RELOAD: About to exec {} with args={:?} fds={}", exe.display(), args, fds_str));
    let err = std::process::Command::new(&exe)
        .args(&args)
        .exec();

    // If we get here, exec failed
    Err(io::Error::other(format!("exec failed: {} (path: {})", err, exe.display())))
}

/// Hot reload on Windows: spawn new process and exit old one.
/// Uses a named event to synchronize so the old process stays alive until
/// the new one has taken over the console (prevents shell prompt flash).
#[cfg(windows)]
pub fn exec_reload(app: &mut App) -> io::Result<()> {
    extern "system" {
        fn CreateEventA(
            lpEventAttributes: *const std::ffi::c_void,
            bManualReset: i32,
            bInitialState: i32,
            lpName: *const u8,
        ) -> isize;
        fn CloseHandle(hObject: isize) -> i32;
    }

    debug_log(is_debug_enabled(), "RELOAD: Starting exec_reload (Windows)");

    // Shut down HTTP/HTTPS servers to release the port before spawning new process
    if let Some(ref mut server) = app.http_server {
        if let Some(tx) = server.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
    #[cfg(feature = "rustls-backend")]
    if let Some(ref mut server) = app.https_server {
        if let Some(tx) = server.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
    #[cfg(feature = "native-tls-backend")]
    if let Some(ref mut server) = app.https_server {
        if let Some(tx) = server.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
    debug_log(is_debug_enabled(), "RELOAD: Saving state...");
    persistence::save_reload_state(app)?;
    debug_log(is_debug_enabled(), "RELOAD: State saved successfully");

    // Make socket handles inheritable so they survive into the child process
    let mut handles_to_keep: Vec<i64> = Vec::new();
    for world in &app.worlds {
        if let Some(handle) = world.socket_fd {
            if make_inheritable(handle as u64).is_ok() {
                handles_to_keep.push(handle);
            }
        }
    }
    debug_log(is_debug_enabled(), &format!("RELOAD: Keeping {} handles", handles_to_keep.len()));

    let (exe, debug_info) = get_executable_path()?;
    debug_log(is_debug_enabled(), &format!("RELOAD: Executable path: {} ({})", exe.display(), debug_info));

    if !exe.exists() {
        return Err(io::Error::other(format!(
            "Executable not found. Debug: {}",
            debug_info
        )));
    }

    // Pass handle list via environment
    let fds_str: String = handles_to_keep
        .iter()
        .map(|h| h.to_string())
        .collect::<Vec<_>>()
        .join(",");
    std::env::set_var(RELOAD_FDS_ENV, &fds_str);
    // Tell new process which reload file to load (PID-specific)
    std::env::set_var("CLAY_RELOAD_PID", std::process::id().to_string());

    // Detect GUI mode from App state (not args — GUI is default on Windows/macOS with no flag)
    let is_headless = app.gui_tx.is_some() || std::env::args().any(|a| a == "-D");

    // Only create a sync event in console mode (not headless/GUI/daemon).
    let sync_event = if !is_headless {
        let event_name = format!("ClayReloadSync-{}\0", std::process::id());
        let handle = unsafe {
            CreateEventA(std::ptr::null(), 1, 0, event_name.as_ptr())
        };
        if handle != 0 {
            std::env::set_var("CLAY_RELOAD_SYNC_EVENT", &event_name[..event_name.len()-1]);
        }
        handle
    } else {
        0
    };

    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--reload" && a != "--crash").collect();
    args.push("--reload".to_string());
    // On Windows/macOS, GUI is the default mode (no --gui flag in args).
    // Ensure the reload child also runs in GUI mode.
    if is_headless && !args.iter().any(|a| a == "--gui" || a.starts_with("--gui=")) {
        args.push("--gui".to_string());
    }
    debug_log(is_debug_enabled(), &format!("RELOAD: About to spawn {} with args={:?} handles={}", exe.display(), args, fds_str));

    // On Windows, disable raw mode and leave alternate screen BEFORE spawning
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen
    );

    if is_headless {
        // GUI/headless mode: pass the HTTP listener socket to the child process
        // so it can reuse port 9000 without rebinding (avoids Windows port zombie issue).
        if let Some(ref server) = app.http_server {
            if let Some(handle) = server.listener_handle {
                if make_inheritable(handle as u64).is_ok() {
                    std::env::set_var("CLAY_HTTP_LISTENER", handle.to_string());
                }
            }
        }

        match std::process::Command::new(&exe).args(&args).spawn() {
            Ok(_child) => {
                // Use exit(0) for clean shutdown — lets the OS properly close sockets
                // so the new process can bind port 9000. TerminateProcess was leaving
                // the port in a zombie state on Windows.
                std::process::exit(0);
            }
            Err(e) => {
                return Err(io::Error::other(format!("Failed to spawn reload process: {}", e)));
            }
        }
    }

    match std::process::Command::new(&exe).args(&args).spawn() {
        Ok(mut child) => {
            if sync_event != 0 {
                unsafe { CloseHandle(sync_event); }
            }
            // Console mode: wait for the child to exit so the shell prompt
            // doesn't appear between the old and new process.
            let code = child.wait().map(|s| s.code().unwrap_or(0)).unwrap_or(0);
            std::process::exit(code);
        }
        Err(e) => {
            if sync_event != 0 {
                std::env::remove_var("CLAY_RELOAD_SYNC_EVENT");
                unsafe { CloseHandle(sync_event); }
            }
            Err(io::Error::other(format!("Failed to spawn reload process: {} (path: {})", e, exe.display())))
        }
    }
}

/// Relaunch this process attached to a different Clay server, or (when `connect_addr` is
/// `None`) as an independent master — used by `/connect host:port` and `/connect --close`.
/// Unlike `exec_reload`, no app state is saved/restored: the new process starts fresh and
/// (for a remote client) fetches its state from the target server over WebSocket.
/// Exec-replaces the process on Unix; spawns a new process and exits on Windows.
pub fn exec_relaunch(connect_addr: Option<&str>, use_gui: bool) -> io::Result<()> {
    debug_log(is_debug_enabled(), &format!(
        "RELAUNCH: Starting exec_relaunch (connect_addr={:?}, use_gui={})", connect_addr, use_gui
    ));

    // Keep the existing argv (e.g. --conf=...) but drop any previous mode/reload flags —
    // detaching/switching must not carry over a stale --console=/--gui= target. --ssh is
    // dropped for the same reason: it's meaningless without a target (detach case), and
    // would otherwise silently keep applying to whatever new target this relaunch is for
    // (switch case) even though the user didn't ask for an SSH tunnel to it. A `/connect`
    // to an SSH-reachable target starts a fresh direct connection unless re-specified;
    // `/reload` (a separate code path using the full original argv) correctly preserves
    // --ssh, since a reload should resume the same SSH-tunneled session.
    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| {
        a != "--reload" && a != "--crash"
            && a != "--console" && a != "--gui" && a != "--ssh"
            && !a.starts_with("--console=") && !a.starts_with("--gui=")
    }).collect();
    let mode_flag = match (use_gui, connect_addr) {
        (true, Some(addr)) => format!("--gui={}", addr),
        (true, None) => "--gui".to_string(),
        (false, Some(addr)) => format!("--console={}", addr),
        (false, None) => "--console".to_string(),
    };
    args.push(mode_flag);

    exec_relaunch_with_args(args, use_gui)
}

#[cfg(all(unix, not(target_os = "android")))]
fn exec_relaunch_with_args(args: Vec<String>, use_gui: bool) -> io::Result<()> {
    use std::os::unix::process::CommandExt;

    let (exe, debug_info) = get_executable_path()?;
    debug_log(is_debug_enabled(), &format!("RELAUNCH: Executable path: {} ({})", exe.display(), debug_info));
    if !exe.exists() {
        return Err(io::Error::other(format!("Executable not found. Debug: {}", debug_info)));
    }

    // Console sessions own the terminal directly; restore it before handing off so the new
    // process starts from a clean screen (GUI/headless processes don't touch the terminal).
    if !use_gui {
        let _ = execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
        let _ = execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    }

    debug_log(is_debug_enabled(), &format!("RELAUNCH: About to exec {} with args={:?}", exe.display(), args));
    let err = std::process::Command::new(&exe).args(&args).exec();

    // If we get here, exec failed — restore the terminal so the caller isn't left broken.
    if !use_gui {
        let _ = crossterm::terminal::enable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableBracketedPaste
        );
    }
    Err(io::Error::other(format!("exec failed: {} (path: {})", err, exe.display())))
}

#[cfg(windows)]
fn exec_relaunch_with_args(args: Vec<String>, _use_gui: bool) -> io::Result<()> {
    let (exe, debug_info) = get_executable_path()?;
    debug_log(is_debug_enabled(), &format!("RELAUNCH: Executable path: {} ({})", exe.display(), debug_info));
    if !exe.exists() {
        return Err(io::Error::other(format!("Executable not found. Debug: {}", debug_info)));
    }
    match std::process::Command::new(&exe).args(&args).spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => Err(io::Error::other(format!("Failed to spawn process: {} (path: {})", e, exe.display()))),
    }
}

#[cfg(not(any(all(unix, not(target_os = "android")), windows)))]
fn exec_relaunch_with_args(_args: Vec<String>, _use_gui: bool) -> io::Result<()> {
    Err(io::Error::other("Switching servers is not supported on this platform."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_newer_version_basic() {
        assert!(is_newer_version("1.0.1", "1.0.0"));
        assert!(is_newer_version("1.1.0", "1.0.9"));
        assert!(is_newer_version("2.0.0", "1.9.9"));
        assert!(!is_newer_version("1.0.0", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "1.0.1"));
    }

    #[test]
    fn test_is_newer_version_prerelease() {
        // Release is newer than a pre-release with the same base version
        assert!(is_newer_version("1.0.0", "1.0.0-beta"));
        assert!(!is_newer_version("1.0.0-alpha", "1.0.0"));
    }

    // Simulate the asset-lookup logic from check_and_download_update to verify
    // the exact-match + fallback behaviour without making real HTTP requests.
    fn find_asset<'a>(assets: &'a serde_json::Value, asset_name: &str) -> Option<&'a serde_json::Value> {
        let arr = assets.as_array()?;
        // Exact match first
        arr.iter()
            .find(|a| a["name"].as_str() == Some(asset_name))
            .or_else(|| {
                // Fallback: platform-predicate match (mirrors is_platform_asset_candidate logic).
                // We test the fallback by checking names that end with .exe (Windows) or
                // contain "musl"/"gui"/"macos" (Linux/macOS).
                arr.iter().find(|a| {
                    a["name"].as_str().map(|n| {
                        n.ends_with(".exe") || n.contains("musl") || n.contains("gui")
                            || n.contains("macos") || n.contains("linux")
                    }).unwrap_or(false)
                })
            })
    }

    #[test]
    fn test_asset_exact_match_windows() {
        let assets = json!([
            {"name": "clay-linux-x86_64-musl"},
            {"name": "clay-linux-x86_64-gui"},
            {"name": "clay-windows-x86_64.exe"},
            {"name": "clay-macos-universal"},
            {"name": "clay-android.apk"},
        ]);
        let found = find_asset(&assets, "clay-windows-x86_64.exe");
        assert_eq!(found.and_then(|a| a["name"].as_str()), Some("clay-windows-x86_64.exe"));
    }

    #[test]
    fn test_asset_exact_match_linux_musl() {
        let assets = json!([
            {"name": "clay-linux-x86_64-musl"},
            {"name": "clay-linux-x86_64-gui"},
            {"name": "clay-windows-x86_64.exe"},
            {"name": "clay-macos-universal"},
        ]);
        let found = find_asset(&assets, "clay-linux-x86_64-musl");
        assert_eq!(found.and_then(|a| a["name"].as_str()), Some("clay-linux-x86_64-musl"));
    }

    #[test]
    fn test_asset_exact_match_macos() {
        let assets = json!([
            {"name": "clay-linux-x86_64-musl"},
            {"name": "clay-macos-universal"},
            {"name": "clay-windows-x86_64.exe"},
        ]);
        let found = find_asset(&assets, "clay-macos-universal");
        assert_eq!(found.and_then(|a| a["name"].as_str()), Some("clay-macos-universal"));
    }

    #[test]
    fn test_asset_not_found_produces_available_list() {
        // Simulate the error-path: no match at all
        let assets = json!([
            {"name": "clay-linux-x86_64-musl"},
            {"name": "clay-macos-universal"},
        ]);
        let arr = assets.as_array().unwrap();
        let found = arr.iter().find(|a| a["name"].as_str() == Some("clay-unknown-platform"));
        assert!(found.is_none(), "Should not find an asset for unknown platform");
        // Verify the error message would include the available names
        let available: Vec<&str> = arr.iter().filter_map(|a| a["name"].as_str()).collect();
        let err = format!("No binary for this platform (clay-unknown-platform) in release. Available: [{}]",
            available.join(", "));
        assert!(err.contains("clay-linux-x86_64-musl"));
        assert!(err.contains("clay-macos-universal"));
    }

    #[test]
    fn test_asset_fallback_for_old_exe_name() {
        // Simulate a release that still uses the old "clay.exe" name instead of
        // "clay-windows-x86_64.exe". The fallback predicate should still find it.
        let assets = json!([
            {"name": "clay.exe"},   // old name
            {"name": "clay-macos-universal"},
        ]);
        // Exact match for the new name fails, but fallback (.exe predicate) succeeds
        let found = find_asset(&assets, "clay-windows-x86_64.exe");
        assert_eq!(found.and_then(|a| a["name"].as_str()), Some("clay.exe"),
            "Fallback should pick up old clay.exe even when exact name changed");
    }

    // -----------------------------------------------------------------
    // TOFU certificate pinning (danger module)
    // -----------------------------------------------------------------

    #[test]
    fn test_sha256_hex_known_vector() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            danger::sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // SHA-256("abc") is a standard NIST test vector
        assert_eq!(
            danger::sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn test_check_pin_first_connect_then_match_then_mismatch() {
        let host = "clay-test-platform-danger.invalid:1234";
        crate::persistence::remove_pin(host);

        // First connect: silent pin
        assert!(danger::check_pin(host, "fingerprint-a").is_ok());
        assert_eq!(crate::persistence::get_pin(host), Some("fingerprint-a".to_string()));
        // No mismatch should have been recorded for a first connect
        assert!(danger::take_cert_mismatch().is_none());

        // Same fingerprint again: still ok, no mismatch recorded
        assert!(danger::check_pin(host, "fingerprint-a").is_ok());
        assert!(danger::take_cert_mismatch().is_none());

        // Different fingerprint: blocked, and mismatch details recorded
        assert!(danger::check_pin(host, "fingerprint-b").is_err());
        let mismatch = danger::take_cert_mismatch().expect("mismatch should be recorded");
        assert_eq!(mismatch.host, host);
        assert_eq!(mismatch.old_fingerprint, "fingerprint-a");
        assert_eq!(mismatch.new_fingerprint, "fingerprint-b");

        // take_cert_mismatch() clears the slot
        assert!(danger::take_cert_mismatch().is_none());

        // The old pin is still what's stored — a mismatch does NOT auto-replace the pin
        assert_eq!(crate::persistence::get_pin(host), Some("fingerprint-a".to_string()));

        crate::persistence::remove_pin(host);
    }

    #[cfg(feature = "rustls-backend")]
    #[test]
    fn test_tofu_verifier_supported_schemes_nonempty() {
        use rustls::client::danger::ServerCertVerifier;
        let verifier = danger_rustls::TofuVerifier::new("unused:0".to_string());
        assert!(!verifier.supported_verify_schemes().is_empty());
    }

    /// End-to-end proof that `TofuVerifier` (a) pins silently on first connect,
    /// (b) blocks and records a mismatch when a *different* certificate is
    /// presented later, and — most importantly — (c) still rejects a connection
    /// where the server replays the *exact* pinned certificate bytes but signs
    /// the handshake with a different private key (i.e. doesn't actually possess
    /// the certificate's private key). (c) is what proves
    /// `verify_tls12_signature`/`verify_tls13_signature` perform real
    /// verification instead of the old `assertion()` rubber-stamp — a bare
    /// fingerprint check alone would NOT catch this, since certificates are
    /// public and sent in the clear during the handshake.
    #[cfg(feature = "rustls-backend")]
    #[tokio::test]
    async fn test_tofu_full_handshake_pin_then_mismatch_then_key_replay_fails() {
        use rcgen::{CertificateParams, KeyPair};
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName};
        use tokio::net::TcpListener;

        fn make_cert() -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
            let key_pair = KeyPair::generate().unwrap();
            let params = CertificateParams::new(vec!["127.0.0.1".to_string(), "localhost".to_string()]).unwrap();
            let cert = params.self_signed(&key_pair).unwrap();
            let cert_der = cert.der().clone();
            let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
            (cert_der, key_der)
        }

        // A `ResolvesServerCert` that always hands back a fixed (cert, key) pair,
        // used instead of `ServerConfig::builder().with_single_cert(...)` below.
        // rustls (since 0.23) has `with_single_cert` call `CertifiedKey::keys_match()`
        // internally and refuse to build a config for a cert/key pair that doesn't
        // match — which is exactly the mismatched pair step 3 of this test needs to
        // construct, to prove the *handshake* (not config construction) is what
        // rejects a replayed certificate signed by the wrong key. Building the
        // `CertifiedKey` directly skips that sanity check, so the mismatched config
        // still builds and the rejection has to come from the TLS signature
        // verification this test is actually exercising.
        #[derive(Debug)]
        struct FixedResolver(std::sync::Arc<rustls::sign::CertifiedKey>);
        impl rustls::server::ResolvesServerCert for FixedResolver {
            fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<std::sync::Arc<rustls::sign::CertifiedKey>> {
                Some(self.0.clone())
            }
        }

        async fn run_one_shot_server(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>, listener: TcpListener) {
            let provider = rustls::crypto::ring::default_provider();
            let signing_key = provider
                .key_provider
                .load_private_key(key)
                .expect("load private key (test-generated PKCS8 key)");
            let certified_key = std::sync::Arc::new(rustls::sign::CertifiedKey::new(vec![cert], signing_key));
            let config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_cert_resolver(std::sync::Arc::new(FixedResolver(certified_key)));
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
            if let Ok((stream, _)) = listener.accept().await {
                // We only care whether the handshake itself completes; drop the result.
                let _ = acceptor.accept(stream).await;
            }
        }

        async fn try_client_connect(addr: std::net::SocketAddr, host_port_key: &str) -> Result<(), String> {
            let config = rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(std::sync::Arc::new(danger_rustls::TofuVerifier::new(host_port_key.to_string())))
                .with_no_client_auth();
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
            let tcp = tokio::net::TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
            let server_name = ServerName::try_from("127.0.0.1").unwrap();
            connector.connect(server_name, tcp).await.map(|_| ()).map_err(|e| e.to_string())
        }

        // Distinct host_port_key from any real pin a developer might have, and
        // deliberately decoupled from the actual (ephemeral, per-listener) TCP
        // port — the pin store keys purely on this string.
        let host_port_key = "clay-tofu-integration-test.invalid:0";
        crate::persistence::remove_pin(host_port_key);

        // --- 1. First connect (correct cert+key pair): silent pin, handshake succeeds ---
        let (cert_a, key_a) = make_cert();
        let fp_a = danger::sha256_hex(cert_a.as_ref());
        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_a = listener_a.local_addr().unwrap();
        let server_a = tokio::spawn(run_one_shot_server(cert_a.clone(), key_a, listener_a));
        let result_a = try_client_connect(addr_a, host_port_key).await;
        let _ = server_a.await;
        assert!(result_a.is_ok(), "first connect with a correctly-signed cert should succeed: {:?}", result_a);
        assert_eq!(crate::persistence::get_pin(host_port_key), Some(fp_a.clone()), "first connect should pin silently");
        assert!(danger::take_cert_mismatch().is_none(), "no mismatch should be recorded on first connect");

        // --- 2. Reconnect with a DIFFERENT (but correctly signed) cert: blocked + mismatch surfaced ---
        let (cert_b, key_b) = make_cert();
        let fp_b = danger::sha256_hex(cert_b.as_ref());
        assert_ne!(fp_a, fp_b);
        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_b = listener_b.local_addr().unwrap();
        let server_b = tokio::spawn(run_one_shot_server(cert_b.clone(), key_b, listener_b));
        let result_b = try_client_connect(addr_b, host_port_key).await;
        let _ = server_b.await;
        assert!(result_b.is_err(), "reconnect with a DIFFERENT certificate must be blocked");
        let mismatch = danger::take_cert_mismatch().expect("mismatch should be recorded for a changed cert");
        assert_eq!(mismatch.host, host_port_key);
        assert_eq!(mismatch.old_fingerprint, fp_a);
        assert_eq!(mismatch.new_fingerprint, fp_b);
        assert_eq!(crate::persistence::get_pin(host_port_key), Some(fp_a.clone()), "a mismatch must NOT auto-replace the pin");

        // --- 3. THE CRITICAL CHECK: replay the pinned cert_a bytes signed with a
        //        DIFFERENT private key (key_c) — simulating an attacker who
        //        captured cert_a's public bytes off the wire (certs are sent
        //        unencrypted during the handshake) but does not possess key_a.
        //        The fingerprint check alone would PASS here (it's the same
        //        cert_a bytes as the pin) — only real signature verification
        //        catches this.
        let (_cert_c_unused, key_c) = make_cert();
        let listener_c = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_c = listener_c.local_addr().unwrap();
        let server_c = tokio::spawn(run_one_shot_server(cert_a.clone(), key_c, listener_c));
        let result_c = try_client_connect(addr_c, host_port_key).await;
        let _ = server_c.await;
        assert!(
            result_c.is_err(),
            "presenting the pinned certificate without its matching private key must still fail the handshake"
        );
        // Crucially, this must fail via signature verification, not the pin
        // comparison — cert_a's fingerprint matches the existing pin exactly, so
        // no CertMismatch should be recorded for this attempt.
        assert!(
            danger::take_cert_mismatch().is_none(),
            "the key-replay rejection must come from signature verification, not a fingerprint mismatch"
        );
        assert_eq!(crate::persistence::get_pin(host_port_key), Some(fp_a), "pin must remain unchanged");

        crate::persistence::remove_pin(host_port_key);
    }
}

