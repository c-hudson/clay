//! Platform-specific and system-level functions extracted from main.rs
//! Includes: crash recovery, hot reload, TLS proxy, update checker, FD handling.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
#[cfg(unix)]
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::io::RawFd;
#[cfg(all(unix, not(target_os = "android")))]
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

// Rustls danger module for accepting invalid certificates (MUD servers often have self-signed certs)
#[cfg(feature = "rustls-backend")]
pub mod danger {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, Error, SignatureScheme};

    #[derive(Debug)]
    pub struct NoCertificateVerification;

    impl Default for NoCertificateVerification {
        fn default() -> Self {
            Self::new()
        }
    }

    impl NoCertificateVerification {
        pub fn new() -> Self {
            Self
        }
    }

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
            ]
        }
    }
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

/// Stub for Android/Windows - TLS proxy processes don't exist on these platforms
#[cfg(any(target_os = "android", not(unix)))]
pub(crate) fn is_process_alive(_pid: u32) -> bool {
    false  // Always return false since we never spawn proxy processes on this platform
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
        root_store.roots = webpki_roots::TLS_SERVER_ROOTS.iter().map(|ta| { rustls::pki_types::TrustAnchor { subject: ta.subject.into(), subject_public_key_info: ta.spki.into(), name_constraints: ta.name_constraints.map(|nc| nc.into()), } }).collect();

        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(danger::NoCertificateVerification::new()))
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
            Ok(s) => s,
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

/// Get the correct GitHub release asset name for this platform
#[cfg(not(target_os = "android"))]
pub(crate) fn get_platform_asset_name() -> &'static str {
    #[cfg(all(target_os = "linux", target_env = "musl"))]
    { "clay" }

    #[cfg(all(target_os = "linux", not(target_env = "musl")))]
    { "clay" }

    #[cfg(target_os = "macos")]
    { "clay-macos-universal" }

    #[cfg(target_os = "windows")]
    { "clay.exe" }

    #[cfg(target_os = "android")]
    { "clay-termux-aarch64" }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android")))]
    { "unknown" }
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

    // Find correct asset for this platform
    let asset_name = get_platform_asset_name();
    let assets = release["assets"]
        .as_array()
        .ok_or("No assets in release")?;
    let asset = assets
        .iter()
        .find(|a| a["name"].as_str() == Some(asset_name))
        .ok_or_else(|| format!("No binary for this platform ({}) in release", asset_name))?;

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
    debug_log(true, "RELOAD: Starting exec_reload");

    // Save the current state
    debug_log(true, "RELOAD: Saving state...");
    persistence::save_reload_state(app)?;
    debug_log(true, "RELOAD: State saved successfully");

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
    debug_log(true, &format!("RELOAD: Keeping {} fds", fds_to_keep.len()));

    // Get the executable path with debug info
    let (exe, debug_info) = get_executable_path()?;
    debug_log(true, &format!("RELOAD: Executable path: {} ({})", exe.display(), debug_info));

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
    debug_log(true, &format!("RELOAD: About to exec {} with args={:?} fds={}", exe.display(), args, fds_str));
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

    debug_log(true, "RELOAD: Starting exec_reload (Windows)");

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
    // Give the server tasks a moment to release the port
    std::thread::sleep(std::time::Duration::from_millis(100));

    debug_log(true, "RELOAD: Saving state...");
    persistence::save_reload_state(app)?;
    debug_log(true, "RELOAD: State saved successfully");

    // Make socket handles inheritable so they survive into the child process
    let mut handles_to_keep: Vec<i64> = Vec::new();
    for world in &app.worlds {
        if let Some(handle) = world.socket_fd {
            if make_inheritable(handle as u64).is_ok() {
                handles_to_keep.push(handle);
            }
        }
    }
    debug_log(true, &format!("RELOAD: Keeping {} handles", handles_to_keep.len()));

    let (exe, debug_info) = get_executable_path()?;
    debug_log(true, &format!("RELOAD: Executable path: {} ({})", exe.display(), debug_info));

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

    // Only create a sync event in console mode (not headless/GUI/daemon).
    // In headless mode there's no console terminal to fight over, and waiting
    // would block the old process from releasing the WebSocket port.
    let is_headless = std::env::args().any(|a| a == "--gui" || a == "-D");
    let sync_event = if !is_headless {
        // Create a named event so the new process can signal when it has taken over the console.
        // This prevents the parent shell (cmd.exe/PowerShell) from reclaiming the console
        // in the gap between old process exit and new process raw mode entry.
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

    // Spawn new process with --reload argument
    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--reload" && a != "--crash").collect();
    args.push("--reload".to_string());
    debug_log(true, &format!("RELOAD: About to spawn {} with args={:?} handles={}", exe.display(), args, fds_str));

    // On Windows, disable raw mode and leave alternate screen BEFORE spawning
    // so the new process gets a clean console state
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste,
        crossterm::terminal::LeaveAlternateScreen
    );

    match std::process::Command::new(&exe).args(&args).spawn() {
        Ok(mut child) => {
            // Wait for the child process to exit — this keeps the original process
            // as the shell's foreground job so the prompt doesn't appear.
            // We've already released raw mode and the alternate screen, so the
            // child process can initialize the terminal fresh without competing.
            if sync_event != 0 {
                unsafe { CloseHandle(sync_event); }
            }
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

