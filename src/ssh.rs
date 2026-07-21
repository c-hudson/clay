//! SSH-tunneled transport for `--console`/`--gui --ssh` and, on Android, the
//! `--ssh-proxy` headless subprocess mode launched by `SshProxyManager.java`.
//!
//! An SSH `direct-tcpip` channel opened to `127.0.0.1:<clay_port>` on the remote
//! host terminates on the daemon's own loopback interface - indistinguishable,
//! from the daemon's point of view, from an ordinary local client (see D8 in
//! SECURITY-ROADMAP.md: loopback is always served plaintext, no cert). Clay's own
//! WebSocket password/auth-key challenge still runs unconditionally over the
//! tunnel - this module only swaps the transport underneath it, nothing else
//! about the security model changes. Callers must connect with plain `ws://`
//! inside the tunnel, never `wss://` - the loopback listener won't present a
//! TLS cert, and SSH already provides confidentiality/integrity end to end.

use std::io;

// Everything below that actually needs the `russh` crate (the tunnel type, host-key
// handler, and connect/auth logic) is gated on `ssh-transport` - see the plain-string
// `SshTarget`/`SshCredentials`/`AuthContext`/`SshError` types just below, which have
// no russh dependency and stay available either way so callers (`main.rs`,
// `remote_client.rs`, `webview_gui.rs`) can parse a target and only need to guard the
// actual `establish_tunnel` call site, mirroring how this codebase already gates
// `try_wss` on `feature = "rustls-backend"` in `remote_client.rs`.
#[cfg(feature = "ssh-transport")]
use std::path::PathBuf;
#[cfg(feature = "ssh-transport")]
use std::pin::Pin;
#[cfg(feature = "ssh-transport")]
use std::sync::Arc;
#[cfg(feature = "ssh-transport")]
use std::task::{Context, Poll};

#[cfg(feature = "ssh-transport")]
use russh::client::{self, Handle};
#[cfg(feature = "ssh-transport")]
use russh::keys::{self, PrivateKey};
#[cfg(feature = "ssh-transport")]
use russh::ChannelStream;
#[cfg(feature = "ssh-transport")]
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(feature = "ssh-transport")]
use crate::platform;

/// Parsed `[user@]host[:clay_port[:ssh_port]]` target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshTarget {
    pub user: String,
    pub host: String,
    pub clay_port: u16,
    pub ssh_port: u16,
}

/// SSH-layer credentials to try, in order: agent (if `use_agent`), then default
/// key files or an explicit key (if provided), then a password. Desktop/Termux
/// `--console`/`--gui --ssh` uses [`SshCredentials::desktop_default`] and never
/// sets `password` (no SSH-layer password auth on that path). Android's
/// `--ssh-proxy` mode has no `~/.ssh` or agent, so it sets `try_default_key_files
/// = false` and supplies `key_pem`/`password` explicitly from what the user
/// entered in Connection Settings.
#[derive(Clone, Default)]
pub struct SshCredentials {
    pub use_agent: bool,
    pub try_default_key_files: bool,
    /// PEM-encoded private key text (Android: pasted key). Tried before the
    /// default key file search, if both are enabled.
    pub key_pem: Option<String>,
    pub key_passphrase: Option<String>,
    pub password: Option<String>,
}

impl SshCredentials {
    /// Agent + the standard `~/.ssh/id_*` files, matching normal `ssh` behavior.
    pub fn desktop_default() -> Self {
        Self {
            use_agent: true,
            try_default_key_files: true,
            key_pem: None,
            key_passphrase: None,
            password: None,
        }
    }
}

/// Whether the caller can prompt on stdio for a key passphrase. The console
/// client runs this before its TUI is initialized, so a plain-terminal prompt is
/// safe there (see CLAUDE.md's no-`println!`-after-TUI-init rule). GUI mode and
/// the headless Android `--ssh-proxy` subprocess have no terminal to prompt on
/// and must fail closed instead.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthContext {
    InteractiveTerminal,
    NonInteractive,
}

#[derive(Debug)]
pub enum SshError {
    /// TCP connect to `ssh_port` failed - wrong host/port/firewall.
    Connect(io::Error),
    /// SSH protocol handshake failed after the TCP connect succeeded.
    Handshake(String),
    /// The remote host's SSH host key doesn't match the pinned fingerprint.
    HostKeyMismatch { host_port: String, old: String, new: String },
    /// No usable credential authenticated (agent, key, and/or password all
    /// failed or were unavailable).
    Auth(String),
    /// SSH itself is fine, but the remote refused to open a `direct-tcpip`
    /// channel to `127.0.0.1:<clay_port>` - almost always means the Clay daemon
    /// isn't listening there (not running, or `clay_port` is wrong).
    ChannelRefused(String),
    Other(String),
}

impl std::fmt::Display for SshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&render_ssh_error(self))
    }
}

impl std::error::Error for SshError {}

#[cfg(feature = "ssh-transport")]
impl From<russh::Error> for SshError {
    fn from(e: russh::Error) -> Self {
        SshError::Handshake(e.to_string())
    }
}

/// Render an [`SshError`] as a user-facing message. Each variant is worded so a
/// user can tell *what layer* failed (SSH transport vs. the Clay daemon on the
/// other end of the tunnel) and what to do about it.
pub fn render_ssh_error(e: &SshError) -> String {
    match e {
        SshError::Connect(io_err) => {
            format!("Could not reach the SSH server: {io_err}")
        }
        SshError::Handshake(msg) => {
            format!("SSH handshake failed: {msg}")
        }
        SshError::HostKeyMismatch { host_port, old, new } => {
            format!(
                "*** SSH HOST KEY FOR {host_port} HAS CHANGED ***\n  old fingerprint: {old}\n  new fingerprint: {new}\n\
                 This could mean the server was reinstalled, or that someone is intercepting your connection."
            )
        }
        SshError::Auth(msg) => {
            format!(
                "SSH authentication failed: {msg}. Tried ssh-agent and the default key files \
                 (~/.ssh/id_ed25519, id_ecdsa, id_rsa). Start an ssh-agent or add a usable key. \
                 A passphrase-protected key with no agent needs `--console --ssh` (GUI mode cannot prompt)."
            )
        }
        SshError::ChannelRefused(msg) => {
            format!(
                "SSH connected, but nothing is listening on the target port on the remote host: {msg}. \
                 Is the Clay daemon running there, and is the clay port correct?"
            )
        }
        SshError::Other(msg) => msg.clone(),
    }
}

/// A live, authenticated SSH session, cheaply cloneable (an `Arc` around the
/// underlying `Handle`) so multiple `direct-tcpip` channels can be opened over
/// the same session - used by the Android `--ssh-proxy` mode, which accepts many
/// local connections over the lifetime of one SSH login and must not
/// re-authenticate per connection (mirrors how an interactive `ssh -L` forward
/// behaves). Desktop/Termux `--console`/`--gui --ssh` only ever open one channel
/// per process, via the [`establish_tunnel`] convenience wrapper below.
#[cfg(feature = "ssh-transport")]
#[derive(Clone)]
pub struct SshSession {
    handle: Arc<Handle<TofuHandler>>,
}

#[cfg(feature = "ssh-transport")]
impl SshSession {
    /// Open a new `direct-tcpip` channel to `127.0.0.1:<clay_port>` on the remote
    /// host over this session.
    pub async fn open_tunnel(&self, clay_port: u16) -> Result<SshTunnel, SshError> {
        let channel = self
            .handle
            .channel_open_direct_tcpip("127.0.0.1", clay_port as u32, "127.0.0.1", 0)
            .await
            .map_err(|e| SshError::ChannelRefused(format!("{e}")))?;
        Ok(SshTunnel { stream: channel.into_stream(), _session: self.clone() })
    }
}

/// A tunneled connection to `127.0.0.1:<clay_port>` on the remote host, ready to
/// hand to `tokio_tungstenite::client_async`. Wraps the SSH channel's stream plus
/// a clone of the owning [`SshSession`] so the whole session (and the background
/// task driving the SSH connection) stays alive for as long as this value does,
/// even if other tunnels sharing the same session have since been dropped.
#[cfg(feature = "ssh-transport")]
pub struct SshTunnel {
    stream: ChannelStream<client::Msg>,
    // Kept only to hold the session open; never read after construction.
    _session: SshSession,
}

#[cfg(feature = "ssh-transport")]
impl AsyncRead for SshTunnel {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

#[cfg(feature = "ssh-transport")]
impl AsyncWrite for SshTunnel {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

/// Parse `[user@]host[:clay_port[:ssh_port]]`. `clay_port` defaults to 9000,
/// `ssh_port` to 22, `user` to the local OS username if omitted.
pub fn parse_ssh_target(spec: &str) -> Result<SshTarget, String> {
    if spec.starts_with("ws://") || spec.starts_with("wss://") {
        return Err("--ssh targets don't take a ws:// or wss:// prefix - the protocol is meaningless inside the SSH tunnel".to_string());
    }

    let (user, rest) = match spec.split_once('@') {
        Some((u, r)) => {
            if u.is_empty() {
                return Err("empty username before '@'".to_string());
            }
            (u.to_string(), r)
        }
        None => (local_os_username(), spec),
    };

    if rest.is_empty() {
        return Err("missing host".to_string());
    }

    let parts: Vec<&str> = rest.split(':').collect();
    let (host, clay_port, ssh_port) = match parts.as_slice() {
        [host] => (*host, 9000u16, 22u16),
        [host, clay_port] => {
            let cp = clay_port.parse::<u16>().map_err(|_| format!("invalid clay port '{clay_port}'"))?;
            (*host, cp, 22u16)
        }
        [host, clay_port, ssh_port] => {
            let cp = clay_port.parse::<u16>().map_err(|_| format!("invalid clay port '{clay_port}'"))?;
            let sp = ssh_port.parse::<u16>().map_err(|_| format!("invalid ssh port '{ssh_port}'"))?;
            (*host, cp, sp)
        }
        _ => return Err(format!("too many ':'-separated fields in '{spec}' (expected host[:clay_port[:ssh_port]])")),
    };

    if host.is_empty() {
        return Err("missing host".to_string());
    }

    Ok(SshTarget { user, host: host.to_string(), clay_port, ssh_port })
}

/// Heuristic check for `--console=`/`--gui=` addresses passed without `--ssh`: the
/// direct (non-SSH) grammar is always `[ws://|wss://]host[:port]` - a bare host, at
/// most one port, never a `user@` prefix. A `[user@]host[:clayport[:sshport]]`
/// argument typed without `--ssh` would otherwise fall through into the direct
/// connect path and fail with a low-level, confusing error (e.g. tungstenite's
/// "HTTP format error: invalid authority" from an `@` in the WS URL) instead of a
/// clear "did you forget --ssh?" message.
pub fn looks_like_ssh_target(addr: &str) -> bool {
    let stripped = addr.strip_prefix("wss://").or_else(|| addr.strip_prefix("ws://")).unwrap_or(addr);
    stripped.contains('@') || stripped.matches(':').count() >= 2
}

/// The local OS username, used as the default SSH user when `[user@]` is
/// omitted - mirrors what a real `ssh`/`scp` client does.
pub fn local_os_username() -> String {
    #[cfg(windows)]
    {
        std::env::var("USERNAME").unwrap_or_else(|_| "root".to_string())
    }
    #[cfg(not(windows))]
    {
        std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "root".to_string())
    }
}

/// SSH host-key TOFU key for the pin store, namespaced against the existing TLS
/// `host:port` keys already stored in the same file (see `persistence::add_pin`).
/// Only used by `TofuHandler` when `ssh-transport` is enabled (and by tests
/// either way), hence the dead-code allowance for a from-scratch build without it.
#[cfg_attr(not(feature = "ssh-transport"), allow(dead_code))]
fn host_key_pin_key(host: &str, ssh_port: u16) -> String {
    format!("ssh:{host}:{ssh_port}")
}

/// `russh::client::Handler` that does host-key TOFU verification against
/// `~/.clay/known_hosts.dat` via the existing `platform::danger::check_pin`
/// (the same silent-first-pin / hard-block-on-mismatch logic the TLS TofuVerifier
/// uses), and records a mismatch for the caller to read back via
/// `platform::danger::take_cert_mismatch()`.
#[cfg(feature = "ssh-transport")]
struct TofuHandler {
    pin_key: String,
}

#[cfg(feature = "ssh-transport")]
impl client::Handler for TofuHandler {
    type Error = SshError;

    async fn check_server_key(&mut self, server_public_key: &keys::ssh_key::PublicKey) -> Result<bool, Self::Error> {
        let blob = server_public_key
            .to_bytes()
            .map_err(|e| SshError::Handshake(format!("could not encode host key: {e}")))?;
        let fingerprint = platform::danger::sha256_hex(&blob);
        match platform::danger::check_pin(&self.pin_key, &fingerprint) {
            Ok(()) => Ok(true),
            Err(_) => {
                // check_pin already stashed the mismatch details via
                // record_cert_mismatch; surface them structured so the caller can
                // decide how to prompt (console) or fail closed (GUI/headless).
                if let Some(mismatch) = platform::danger::take_cert_mismatch() {
                    Err(SshError::HostKeyMismatch {
                        host_port: mismatch.host,
                        old: mismatch.old_fingerprint,
                        new: mismatch.new_fingerprint,
                    })
                } else {
                    Err(SshError::Other("SSH host key mismatch".to_string()))
                }
            }
        }
    }
}

/// Default `~/.ssh/id_*` files tried in the same order OpenSSH's client does.
#[cfg(feature = "ssh-transport")]
fn default_key_paths() -> Vec<PathBuf> {
    let home = crate::get_home_dir();
    let ssh_dir = PathBuf::from(home).join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_ecdsa_sk", "id_ed25519_sk", "id_rsa"]
        .iter()
        .map(|f| ssh_dir.join(f))
        .collect()
}

/// Prompt for a key passphrase on plain stdio. Only ever called under
/// `AuthContext::InteractiveTerminal`, i.e. before the console client's TUI is
/// initialized - see the module doc comment.
#[cfg(feature = "ssh-transport")]
fn prompt_passphrase(prompt: &str) -> Option<String> {
    use std::io::Write;
    eprint!("{prompt}");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_ok() {
        let trimmed = answer.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
    } else {
        None
    }
}

/// Try public-key auth via a loaded `PrivateKey`, picking a sensible RSA hash
/// algorithm when needed (see `Handle::best_supported_rsa_hash`'s doc comment in
/// russh - RSA keys need an explicit hash choice; other key types ignore it).
#[cfg(feature = "ssh-transport")]
async fn try_key_auth(
    handle: &mut Handle<TofuHandler>,
    user: &str,
    key: PrivateKey,
) -> Result<bool, SshError> {
    let hash_alg = if key.algorithm().is_rsa() {
        handle
            .best_supported_rsa_hash()
            .await
            .map_err(|e| SshError::Handshake(format!("{e}")))?
            .flatten()
    } else {
        None
    };
    let with_hash = keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
    match handle.authenticate_publickey(user, with_hash).await {
        Ok(result) => Ok(result.success()),
        Err(e) => Err(SshError::Handshake(format!("{e}"))),
    }
}

/// Connect to `target.host:target.ssh_port` and authenticate with `creds`,
/// without opening any `direct-tcpip` channel yet. Returns a session that can
/// have any number of tunnels opened over it via [`SshSession::open_tunnel`] -
/// used by the Android `--ssh-proxy` mode, which authenticates once and then
/// serves many local connections. Desktop/Termux `--console`/`--gui --ssh`
/// should use [`establish_tunnel`] instead, which opens exactly one channel.
#[cfg(feature = "ssh-transport")]
pub async fn establish_session(
    target: &SshTarget,
    creds: &SshCredentials,
    ctx: AuthContext,
) -> Result<SshSession, SshError> {
    let addr = format!("{}:{}", target.host, target.ssh_port);
    let tcp = tokio::net::TcpStream::connect(&addr).await.map_err(SshError::Connect)?;

    let config = Arc::new(client::Config::default());
    let pin_key = host_key_pin_key(&target.host, target.ssh_port);
    let handler = TofuHandler { pin_key };

    let mut handle = client::connect_stream(config, tcp, handler)
        .await
        .map_err(|e| match e {
            SshError::HostKeyMismatch { .. } => e,
            other => SshError::Handshake(render_ssh_error(&other)),
        })?;

    authenticate(&mut handle, target, creds, ctx).await?;

    Ok(SshSession { handle: Arc::new(handle) })
}

/// Establish an SSH session to `target.host:target.ssh_port`, authenticate with
/// `creds`, and open a single `direct-tcpip` channel to
/// `127.0.0.1:<target.clay_port>` on the remote host. Returns a stream ready for
/// `tokio_tungstenite::client_async("ws://127.0.0.1:<clay_port>/", tunnel)`.
/// Convenience wrapper around [`establish_session`] + [`SshSession::open_tunnel`]
/// for callers (desktop `--console`/`--gui --ssh`) that only ever need one
/// channel per process.
#[cfg(feature = "ssh-transport")]
pub async fn establish_tunnel(
    target: &SshTarget,
    creds: &SshCredentials,
    ctx: AuthContext,
) -> Result<SshTunnel, SshError> {
    let session = establish_session(target, creds, ctx).await?;
    session.open_tunnel(target.clay_port).await
}

#[cfg(feature = "ssh-transport")]
async fn authenticate(
    handle: &mut Handle<TofuHandler>,
    target: &SshTarget,
    creds: &SshCredentials,
    ctx: AuthContext,
) -> Result<(), SshError> {
    // 1. SSH agent, if requested and reachable. connect_env() (the SSH_AUTH_SOCK-based
    // Unix agent protocol) is #[cfg(unix)] in russh - there's no equivalent on Windows
    // (that would be Pageant or the OpenSSH-for-Windows named-pipe agent, neither wired
    // up here yet), so this step is a no-op there and falls through to key files/password.
    #[cfg(unix)]
    if creds.use_agent {
        if let Ok(mut agent) = keys::agent::client::AgentClient::connect_env().await {
            if let Ok(identities) = agent.request_identities().await {
                for public_key in identities {
                    let hash_alg = if public_key.algorithm().is_rsa() {
                        handle.best_supported_rsa_hash().await.ok().flatten().flatten()
                    } else {
                        None
                    };
                    if let Ok(result) = handle
                        .authenticate_publickey_with(&target.user, public_key, hash_alg, &mut agent)
                        .await
                    {
                        if result.success() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    // 2. An explicit key (Android: pasted PEM), or the default ~/.ssh/id_* files.
    if let Some(pem) = &creds.key_pem {
        match keys::decode_secret_key(pem, creds.key_passphrase.as_deref()) {
            Ok(key) => {
                if try_key_auth(handle, &target.user, key).await? {
                    return Ok(());
                }
            }
            Err(keys::Error::KeyIsEncrypted) if ctx == AuthContext::InteractiveTerminal => {
                if let Some(passphrase) = prompt_passphrase("Enter passphrase for provided SSH key: ") {
                    if let Ok(key) = keys::decode_secret_key(pem, Some(&passphrase)) {
                        if try_key_auth(handle, &target.user, key).await? {
                            return Ok(());
                        }
                    }
                }
            }
            Err(_) => {}
        }
    }

    if creds.try_default_key_files {
        for path in default_key_paths() {
            if !path.is_file() {
                continue;
            }
            match keys::load_secret_key(&path, None) {
                Ok(key) => {
                    if try_key_auth(handle, &target.user, key).await? {
                        return Ok(());
                    }
                }
                Err(keys::Error::KeyIsEncrypted) if ctx == AuthContext::InteractiveTerminal => {
                    let prompt = format!("Enter passphrase for {}: ", path.display());
                    if let Some(passphrase) = prompt_passphrase(&prompt) {
                        if let Ok(key) = keys::load_secret_key(&path, Some(&passphrase)) {
                            if try_key_auth(handle, &target.user, key).await? {
                                return Ok(());
                            }
                        }
                    }
                }
                Err(_) => continue,
            }
        }
    }

    // 3. Password (Android's --ssh-proxy only; desktop/Termux --console/--gui
    // --ssh never sets this - see SshCredentials::desktop_default).
    if let Some(password) = &creds.password {
        match handle.authenticate_password(&target.user, password).await {
            Ok(result) if result.success() => return Ok(()),
            _ => {}
        }
    }

    Err(SshError::Auth("no usable key, agent identity, or password".to_string()))
}

/// Headless mode: `clay --ssh-proxy --target=<[user@]host:clayport:sshport> --listen-port=<N>`.
///
/// Establishes one persistent SSH session (credentials from the `CLAY_SSH_KEY`/
/// `CLAY_SSH_KEY_PASSPHRASE`/`CLAY_SSH_PASSWORD` environment variables - at least
/// one of key or password must be set), then accepts local TCP connections on
/// `127.0.0.1:<listen_port>` and bridges each one to a fresh `direct-tcpip`
/// channel opened over that session with a plain byte-for-byte copy - this is a
/// transparent raw-TCP forwarder, not WebSocket-aware, so it carries the
/// CLAY-KNOCK preamble and any TLS bytes through untouched end to end.
///
/// This is what `SshProxyManager.java` launches on Android (exec'ing the app's
/// own bundled `libclay.so`, exactly how `LocalServerManager.java` launches
/// `--local-server` today) so the existing `NativeWebSocket`/CLAY-KNOCK Java code
/// can point at `127.0.0.1:<listen_port>` instead of the real remote host with no
/// changes of its own - see the module doc comment for the security reasoning
/// (a direct-tcpip channel to `127.0.0.1:<clay_port>` looks like an ordinary
/// loopback client to the daemon on the other end).
///
/// This function never returns on success (it serves forever); headless/no-TUI
/// context, so plain `println!`/`eprintln!` are fine here per CLAUDE.md.
#[cfg(feature = "ssh-transport")]
pub async fn run_ssh_proxy_mode(target_spec: &str, listen_port: u16) -> io::Result<()> {
    let target = match parse_ssh_target(target_spec) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("clay: invalid --target for --ssh-proxy: {e}");
            std::process::exit(1);
        }
    };

    let key_pem = std::env::var("CLAY_SSH_KEY").ok().filter(|s| !s.is_empty());
    let key_passphrase = std::env::var("CLAY_SSH_KEY_PASSPHRASE").ok().filter(|s| !s.is_empty());
    let password = std::env::var("CLAY_SSH_PASSWORD").ok().filter(|s| !s.is_empty());

    if key_pem.is_none() && password.is_none() {
        eprintln!("clay: --ssh-proxy requires CLAY_SSH_KEY and/or CLAY_SSH_PASSWORD to be set.");
        std::process::exit(1);
    }

    let creds = SshCredentials {
        use_agent: false,
        try_default_key_files: false,
        key_pem,
        key_passphrase,
        password,
    };

    println!("clay: connecting to {}@{}:{}...", target.user, target.host, target.ssh_port);
    // NonInteractive: this is a headless subprocess with no terminal to prompt on
    // (see AuthContext's doc comment) - an encrypted key with no passphrase, or a
    // host-key mismatch, both fail closed here rather than hanging on a prompt
    // nobody can answer.
    let session = match establish_session(&target, &creds, AuthContext::NonInteractive).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("clay: {}", render_ssh_error(&e));
            std::process::exit(1);
        }
    };
    println!(
        "clay: SSH session established; forwarding 127.0.0.1:{} -> {}:{} (via SSH)",
        listen_port, target.host, target.clay_port
    );

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", listen_port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("clay: could not bind 127.0.0.1:{listen_port}: {e}");
            std::process::exit(1);
        }
    };

    loop {
        let (mut local_stream, _) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                eprintln!("clay: accept failed: {e}");
                continue;
            }
        };
        let session = session.clone();
        let clay_port = target.clay_port;
        tokio::spawn(async move {
            match session.open_tunnel(clay_port).await {
                Ok(mut tunnel) => {
                    let _ = tokio::io::copy_bidirectional(&mut local_stream, &mut tunnel).await;
                }
                Err(e) => {
                    eprintln!("clay: failed to open SSH tunnel for a local connection: {}", render_ssh_error(&e));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence;
    #[cfg(not(feature = "ssh-transport"))]
    use crate::platform;

    #[test]
    fn test_parse_bare_host() {
        let t = parse_ssh_target("example.com").unwrap();
        assert_eq!(t.host, "example.com");
        assert_eq!(t.clay_port, 9000);
        assert_eq!(t.ssh_port, 22);
        assert_eq!(t.user, local_os_username());
    }

    #[test]
    fn test_parse_host_clayport() {
        let t = parse_ssh_target("example.com:9001").unwrap();
        assert_eq!(t.host, "example.com");
        assert_eq!(t.clay_port, 9001);
        assert_eq!(t.ssh_port, 22);
    }

    #[test]
    fn test_parse_host_clayport_sshport() {
        let t = parse_ssh_target("example.com:9001:2222").unwrap();
        assert_eq!(t.host, "example.com");
        assert_eq!(t.clay_port, 9001);
        assert_eq!(t.ssh_port, 2222);
    }

    #[test]
    fn test_parse_user_at_host() {
        let t = parse_ssh_target("alice@example.com").unwrap();
        assert_eq!(t.user, "alice");
        assert_eq!(t.host, "example.com");
        assert_eq!(t.clay_port, 9000);
        assert_eq!(t.ssh_port, 22);
    }

    #[test]
    fn test_parse_user_at_host_clayport_sshport() {
        let t = parse_ssh_target("alice@example.com:9001:2222").unwrap();
        assert_eq!(t.user, "alice");
        assert_eq!(t.host, "example.com");
        assert_eq!(t.clay_port, 9001);
        assert_eq!(t.ssh_port, 2222);
    }

    #[test]
    fn test_parse_rejects_ws_prefix() {
        assert!(parse_ssh_target("ws://example.com").is_err());
        assert!(parse_ssh_target("wss://example.com").is_err());
    }

    #[test]
    fn test_parse_rejects_bad_ports() {
        assert!(parse_ssh_target("example.com:notaport").is_err());
        assert!(parse_ssh_target("example.com:9001:notaport").is_err());
    }

    #[test]
    fn test_parse_rejects_empty_host() {
        assert!(parse_ssh_target("").is_err());
        assert!(parse_ssh_target("alice@").is_err());
    }

    #[test]
    fn test_parse_rejects_empty_user() {
        assert!(parse_ssh_target("@example.com").is_err());
    }

    #[test]
    fn test_parse_rejects_too_many_fields() {
        assert!(parse_ssh_target("example.com:9001:2222:extra").is_err());
    }

    #[test]
    fn test_looks_like_ssh_target() {
        // Valid direct (non-SSH) addresses: never flagged.
        assert!(!looks_like_ssh_target("example.com"));
        assert!(!looks_like_ssh_target("example.com:9000"));
        assert!(!looks_like_ssh_target("ws://example.com:9000"));
        assert!(!looks_like_ssh_target("wss://example.com:9000"));
        assert!(!looks_like_ssh_target("192.168.2.6"));
        assert!(!looks_like_ssh_target("192.168.2.6:9000"));
        // SSH-style addresses (typed without --ssh): flagged.
        assert!(looks_like_ssh_target("adrick@192.168.2.6:9000:22"));
        assert!(looks_like_ssh_target("adrick@192.168.2.6"));
        assert!(looks_like_ssh_target("192.168.2.6:9000:22"));
        assert!(looks_like_ssh_target("wss://adrick@192.168.2.6:9000:22"));
    }

    #[test]
    fn test_render_ssh_error_variants_distinguishable() {
        let variants = [
            SshError::Connect(io::Error::other("boom")),
            SshError::Handshake("boom".to_string()),
            SshError::HostKeyMismatch { host_port: "ssh:h:22".to_string(), old: "aaaa".to_string(), new: "bbbb".to_string() },
            SshError::Auth("boom".to_string()),
            SshError::ChannelRefused("boom".to_string()),
            SshError::Other("boom".to_string()),
        ];
        let mut messages: Vec<String> = variants.iter().map(render_ssh_error).collect();
        messages.sort();
        messages.dedup();
        assert_eq!(messages.len(), variants.len(), "each SshError variant must render a distinguishable message");
    }

    #[test]
    fn test_host_key_tofu_first_connect_pins_silently() {
        let pin_key = host_key_pin_key("clay-test-ssh-tofu.invalid", 22);
        persistence::remove_pin(&pin_key);

        assert_eq!(persistence::get_pin(&pin_key), None);
        assert!(platform::danger::check_pin(&pin_key, "aaaa").is_ok());
        assert_eq!(persistence::get_pin(&pin_key), Some("aaaa".to_string()));

        persistence::remove_pin(&pin_key);
    }

    #[test]
    fn test_host_key_tofu_match_passes_via_check_pin() {
        let pin_key = host_key_pin_key("clay-test-ssh-tofu-2.invalid", 22);
        persistence::remove_pin(&pin_key);

        assert!(platform::danger::check_pin(&pin_key, "aaaa").is_ok());
        // Same fingerprint again: still Ok. (The mismatch branch of check_pin is
        // intentionally NOT exercised here - it writes to platform::danger's
        // process-wide CERT_MISMATCH slot, which platform.rs's own TOFU handshake
        // test also asserts on; racing that from here caused a flaky failure. The
        // mismatch-blocks-and-does-not-replace behavior below is tested directly
        // at the persistence layer instead, which is what actually guarantees it.)
        assert!(platform::danger::check_pin(&pin_key, "aaaa").is_ok());

        persistence::remove_pin(&pin_key);
    }

    #[test]
    fn test_host_key_tofu_mismatch_does_not_replace_pin() {
        // Persistence-layer equivalent of check_pin's mismatch behavior (see note
        // above for why this doesn't call check_pin itself).
        let pin_key = host_key_pin_key("clay-test-ssh-tofu-2b.invalid", 22);
        persistence::remove_pin(&pin_key);

        assert!(persistence::add_pin(&pin_key, "aaaa"));
        // A second add_pin with a different fingerprint is a no-op (first-connect
        // only); an explicit replace_pin is required to change a pinned value.
        assert!(!persistence::add_pin(&pin_key, "bbbb"));
        assert_eq!(persistence::get_pin(&pin_key), Some("aaaa".to_string()));

        persistence::remove_pin(&pin_key);
    }

    #[test]
    fn test_host_key_tofu_key_does_not_collide_with_tls_pins() {
        let ssh_key = host_key_pin_key("clay-test-ssh-tofu-3.invalid", 22);
        let tls_key = "clay-test-ssh-tofu-3.invalid:22".to_string();
        persistence::remove_pin(&ssh_key);
        persistence::remove_pin(&tls_key);

        assert!(platform::danger::check_pin(&ssh_key, "ssh-fp").is_ok());
        assert!(platform::danger::check_pin(&tls_key, "tls-fp").is_ok());
        assert_eq!(persistence::get_pin(&ssh_key), Some("ssh-fp".to_string()));
        assert_eq!(persistence::get_pin(&tls_key), Some("tls-fp".to_string()));

        persistence::remove_pin(&ssh_key);
        persistence::remove_pin(&tls_key);
    }
}
