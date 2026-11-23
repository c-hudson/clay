//! Audio backend abstraction for console playback.
//!
//! Supports two backends:
//! - Native: Uses rodio for direct audio playback (requires `native-audio` feature)
//! - External: Uses mpv or ffplay as subprocess
//!
//! The native backend avoids disk writes for ANSI music (plays WAV from memory)
//! and plays downloaded media files directly without spawning subprocesses.

use std::path::Path;

/// Audio playback backend
pub enum AudioBackend {
    /// Native audio via rodio (compiled with `native-audio` feature)
    #[cfg(feature = "native-audio")]
    Native {
        _stream: rodio::OutputStream,
        stream_handle: rodio::OutputStreamHandle,
    },
    /// External player (mpv or ffplay)
    External { player_cmd: String },
    /// No audio available
    None,
}

// Safety: AudioBackend is only accessed from the App's owning task/thread.
// rodio::OutputStream contains *mut () which prevents auto-Send, but we never
// actually move it between threads — tokio requires Send for spawned tasks at
// compile time even when the value stays on one task.
unsafe impl Send for AudioBackend {}

/// Handle to a currently playing sound
pub enum PlayHandle {
    /// rodio Sink (native playback)
    #[cfg(feature = "native-audio")]
    NativeSink(rodio::Sink),
    /// External player process
    ExternalProcess(std::process::Child),
}

// Safety: Same reasoning as AudioBackend — PlayHandle stays on the App's task.
unsafe impl Send for PlayHandle {}

impl PlayHandle {
    /// Stop/kill the playing sound
    pub fn stop(&mut self) {
        match self {
            #[cfg(feature = "native-audio")]
            PlayHandle::NativeSink(sink) => {
                sink.stop();
            }
            PlayHandle::ExternalProcess(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// Kill without waiting (for media processes where we don't want to block)
    pub fn kill(&mut self) {
        match self {
            #[cfg(feature = "native-audio")]
            PlayHandle::NativeSink(sink) => {
                sink.stop();
            }
            PlayHandle::ExternalProcess(child) => {
                let _ = child.kill();
            }
        }
    }

    /// Change the volume (0-100) of what's playing, where the player allows it (rodio
    /// does; an external player keeps the volume it started with).
    pub fn set_volume(&self, volume: i64) {
        match self {
            #[cfg(feature = "native-audio")]
            PlayHandle::NativeSink(sink) => sink.set_volume(volume as f32 / 100.0),
            PlayHandle::ExternalProcess(_) => {
                let _ = volume;
            }
        }
    }

    /// Check if the process has finished (for external processes)
    pub fn try_wait(&mut self) -> Option<bool> {
        match self {
            #[cfg(feature = "native-audio")]
            PlayHandle::NativeSink(sink) => {
                Some(sink.empty())
            }
            PlayHandle::ExternalProcess(child) => {
                match child.try_wait() {
                    Ok(Some(_)) => Some(true),   // finished
                    Ok(None) => Some(false),      // still running
                    Err(_) => Some(true),         // error, treat as finished
                }
            }
        }
    }
}

/// The ALSA devices that are a sound server rather than a card. Playing through one puts
/// the stream under the desktop's mixer - its volume control, the per-application volume,
/// the sink's own level (a 150% boost on an HDMI output, say) - and shares the card.
#[cfg(all(target_os = "linux", feature = "native-audio"))]
const SOUND_SERVER_DEVICES: [&str; 2] = ["pipewire", "pulse"];

#[cfg(all(target_os = "linux", feature = "native-audio"))]
pub(crate) fn is_sound_server_device(name: &str) -> bool {
    SOUND_SERVER_DEVICES.contains(&name)
}

/// Open the output stream Clay plays through - the MUD's sounds and the Kokoro voice
/// alike. Returns the stream, its handle and the device's name (for `/tts status`).
///
/// On Linux, ALSA's `default` is not always the sound server: a PipeWire desktop with
/// the PulseAudio compatibility plugin but neither `pipewire-alsa` nor the package that
/// maps `default` to that plugin has `default` pointing at the sound card itself. Clay
/// then played straight to the hardware, outside the desktop's mixer - no volume control
/// reached it and the sink's boost never applied ("I turned the volume up and can still
/// barely hear it"), while espeak, which talks to the server itself, was loud. So the
/// `pipewire` and `pulse` devices are preferred when they exist; `default` is the
/// fallback, and the only choice on every other platform. cpal opens each device as it
/// lists them, so the walk stops at the first match, before the cards.
#[cfg(feature = "native-audio")]
pub fn open_output_stream() -> Option<(rodio::OutputStream, rodio::OutputStreamHandle, String)> {
    // ALSA complains on stderr about every device it cannot open while cpal probes
    // them; in the console that would land on the screen.
    let _stderr = crate::platform::StderrToLog::new();
    #[cfg(target_os = "linux")]
    {
        use rodio::cpal::traits::{DeviceTrait, HostTrait};
        if let Ok(mut devices) = rodio::cpal::default_host().output_devices() {
            if let Some(device) = devices.find(|d| d.name().map(|n| is_sound_server_device(&n)).unwrap_or(false)) {
                let name = device.name().unwrap_or_default();
                if let Ok((stream, handle)) = rodio::OutputStream::try_from_device(&device) {
                    return Some((stream, handle, name));
                }
            }
        }
    }
    let (stream, handle) = rodio::OutputStream::try_default().ok()?;
    Some((stream, handle, "default".to_string()))
}

/// Initialize the audio backend.
/// Tries rodio first (if compiled in), falls back to external player detection.
pub fn init_audio() -> AudioBackend {
    #[cfg(feature = "native-audio")]
    {
        if let Some((stream, handle, _device)) = open_output_stream() {
            return AudioBackend::Native {
                _stream: stream,
                stream_handle: handle,
            };
        }
    }

    // Fall back to external player
    if let Some(cmd) = detect_external_player() {
        AudioBackend::External { player_cmd: cmd }
    } else {
        AudioBackend::None
    }
}

/// Detect mpv or ffplay on the system
fn detect_external_player() -> Option<String> {
    // On Windows, skip external player detection — use native audio only.
    // Spawning processes to search PATH can be very slow if large apps
    // (e.g. LibreOffice) add many directories to PATH.
    #[cfg(windows)]
    { None }

    #[cfg(not(windows))]
    {
        // The Android app's bundled server (--local-server) has neither player, and must not
        // start a subprocess: it hangs there at random (see tts::detect_tts_command).
        if crate::LOCAL_SERVER_LOOPBACK_ONLY.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        for cmd in &["mpv", "ffplay"] {
            if std::process::Command::new("which")
                .arg(cmd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return Some(cmd.to_string());
            }
        }
        None
    }
}

/// Play WAV data from memory (for ANSI music).
/// With native backend, plays directly from memory buffer.
/// With external backend, writes to disk and spawns player.
pub fn play_wav_memory(
    backend: &AudioBackend,
    wav_data: Vec<u8>,
    cache_dir: &Path,
    counter: u32,
) -> Option<PlayHandle> {
    match backend {
        #[cfg(feature = "native-audio")]
        AudioBackend::Native { stream_handle, .. } => {
            let cursor = std::io::Cursor::new(wav_data);
            let source = match rodio::Decoder::new(cursor) {
                Ok(s) => s,
                Err(_) => return None,
            };
            let sink = match rodio::Sink::try_new(stream_handle) {
                Ok(s) => s,
                Err(_) => return None,
            };
            sink.append(source);
            Some(PlayHandle::NativeSink(sink))
        }
        AudioBackend::External { player_cmd } => {
            let _ = std::fs::create_dir_all(cache_dir);
            let wav_path = cache_dir.join(format!("ansi_music_{}.wav", counter % 4));
            if std::fs::write(&wav_path, &wav_data).is_err() {
                return None;
            }
            let wav_path_str = wav_path.to_string_lossy().to_string();
            let mut cmd = std::process::Command::new(player_cmd);
            match player_cmd.as_str() {
                "mpv" => { cmd.args(["--no-video", "--no-terminal", &wav_path_str]); }
                "ffplay" => { cmd.args(["-nodisp", "-autoexit", &wav_path_str]); }
                _ => { cmd.arg(&wav_path_str); }
            }
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
            cmd.spawn().ok().map(PlayHandle::ExternalProcess)
        }
        AudioBackend::None => None,
    }
}

/// Play an audio file from disk.
/// Returns a PlayHandle for stopping later.
pub fn play_file(
    backend: &AudioBackend,
    path: &Path,
    volume: i64,
    loops: i64,
) -> Option<PlayHandle> {
    // T1.7: the External arm below used to spawn `mpv`/`ffplay` unconditionally, even
    // for a file that was never downloaded (MSP bare-name trigger for a sound pack Clay
    // doesn't have, or a GMCP media fetch that failed). The Native arm already fails
    // safely via `fs::read`'s `Err` — this makes both arms agree before either runs.
    if !path.is_file() {
        return None;
    }
    match backend {
        #[cfg(feature = "native-audio")]
        AudioBackend::Native { stream_handle, .. } => {
            // Read file into memory so we can decode multiple times for looping
            let data = match std::fs::read(path) {
                Ok(d) => d,
                Err(_) => return None,
            };
            let sink = match rodio::Sink::try_new(stream_handle) {
                Ok(s) => s,
                Err(_) => return None,
            };
            // Volume: GMCP uses 0-100, rodio uses 0.0-1.0
            sink.set_volume(volume as f32 / 100.0);

            let play_count = if loops == -1 { 1000 } else { loops.max(1) };
            for _ in 0..play_count {
                let cursor = std::io::Cursor::new(data.clone());
                if let Ok(src) = rodio::Decoder::new(cursor) {
                    sink.append(src);
                } else {
                    break;
                }
            }
            Some(PlayHandle::NativeSink(sink))
        }
        AudioBackend::External { player_cmd } => {
            let path_str = path.to_string_lossy().to_string();
            let mut cmd = std::process::Command::new(player_cmd);
            match player_cmd.as_str() {
                "mpv" => {
                    cmd.args(["--no-video", "--no-terminal"]);
                    cmd.arg(format!("--volume={}", volume));
                    if loops == -1 {
                        cmd.arg("--loop=inf");
                    } else if loops > 1 {
                        cmd.arg(format!("--loop={}", loops));
                    }
                }
                "ffplay" => {
                    cmd.args(["-nodisp", "-autoexit"]);
                    cmd.arg("-volume").arg(format!("{}", volume));
                    if loops == -1 || loops > 1 {
                        cmd.arg("-loop").arg(
                            if loops == -1 { "0".to_string() }
                            else { loops.to_string() }
                        );
                    }
                }
                _ => {}
            }
            cmd.arg(&path_str);
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
            cmd.spawn().ok().map(PlayHandle::ExternalProcess)
        }
        AudioBackend::None => None,
    }
}

/// How many redirects a media download follows, each one re-validated (`fetch_media`).
const MEDIA_MAX_REDIRECTS: usize = 5;
/// How long a media download may take before it is abandoned.
const MEDIA_DOWNLOAD_TIMEOUT_SECS: u64 = 30;

/// Whether a single URL is an allowed media target: an `http`/`https` scheme and a host
/// that isn't a *literal* loopback/link-local/private/ULA address. **Applied to every
/// redirect hop** (`fetch_media`'s redirect policy), not only the first URL — a redirect
/// to an internal address is the exact bypass this closes (security review 2026-11-21).
///
/// - Only `http`/`https` — blocks `file://` (arbitrary local-file read, e.g.
///   `file:///etc/passwd`) and other exotic schemes.
/// - A literal loopback/link-local/private/ULA IP is rejected — the common SSRF targets,
///   notably the cloud-metadata endpoint `169.254.169.254` (link-local).
///
/// Residual limitation (unchanged): a *hostname* that resolves to an internal address is
/// not caught here (DNS rebinding) — closing that fully means resolving in-process and
/// pinning the connection to the vetted IP (see SECURITY-ROADMAP.md). The redirect path,
/// which used to be wide open (`curl -L` followed a redirect to any host), is now closed.
fn media_url_host_allowed(parsed: &url::Url) -> Result<(), &'static str> {
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err("only http/https URLs are allowed"),
    }
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => {
            if ip.is_loopback() || ip.is_link_local() || ip.is_private() || ip.is_unspecified() {
                return Err("target host is a loopback/link-local/private address");
            }
        }
        Some(url::Host::Ipv6(ip)) => {
            // fe80::/10 = link-local unicast; fc00::/7 = unique-local (RFC 4193).
            let is_link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            let is_unique_local = (ip.segments()[0] & 0xfe00) == 0xfc00;
            if ip.is_loopback() || ip.is_unspecified() || is_link_local || is_unique_local {
                return Err("target host is a loopback/link-local/ULA IPv6 address");
            }
            if let Some(v4) = ip.to_ipv4_mapped() {
                if v4.is_loopback() || v4.is_link_local() || v4.is_private() || v4.is_unspecified() {
                    return Err("target host is a loopback/link-local/private address (IPv4-mapped)");
                }
            }
        }
        Some(url::Host::Domain(_)) => {} // resolved at connect time; see limitation above
        None => return Err("URL has no host"),
    }
    Ok(())
}

/// B5 (security remediation): validate a MUD-supplied GMCP `Client.Media.*` / MSP URL
/// before it is fetched. `download_to_cache` is the single choke point both media call
/// sites (`main.rs` "Play"/"Load" and the MSP `U=` path) route through.
fn validate_media_url(url_str: &str) -> Result<url::Url, &'static str> {
    let parsed = url::Url::parse(url_str).map_err(|_| "invalid URL")?;
    media_url_host_allowed(&parsed)?;
    Ok(parsed)
}

/// Job 14 (plan Phase 4, MSP): resolve a MUD-supplied bare filename (from a
/// `!!SOUND(name ...)`/`!!MUSIC(name ...)` trigger with no `U=`) to a path
/// inside `base_dir`, or reject it. Unlike GMCP `Client.Media.*` — URL-only,
/// so every reference already goes through `validate_media_url` — MSP's bare
/// name means "play this file from the local media directory," which is
/// path-traversal surface Clay never had on this code path before: a name
/// like `../../../etc/passwd`, `/etc/passwd`, or `C:\Windows\System32\x` must
/// never resolve outside `base_dir`.
///
/// Two layers, per the plan's explicit requirement that a resolution check
/// alone is not enough:
///
/// 1. Lexical rejection, before anything touches the filesystem: any path
///    separator (`/` or `\` — both, regardless of host platform, since a
///    legitimate filename never needs either), any colon (catches a Windows
///    drive letter like `C:` *and* an NTFS alternate-data-stream suffix like
///    `name:hidden` in one rule), and `.`/`..` themselves. `name` is also
///    required to parse as exactly one `Path` component (`Normal`) — this is
///    what actually rejects `..` as a component in general, the colon/slash
///    checks above are the common cases spelled out for a clearer rejection
///    reason and because `Path`'s own component parser is platform-flavoured
///    in ways a MUD-supplied name shouldn't be trusted to route through
///    (e.g. `Path::new("C:foo").is_absolute()` is `false` on a Unix build of
///    Clay, so relying on `is_absolute()` alone would miss it on that host).
/// 2. Resolution containment: even a lexically clean single-component name
///    could itself be a symlink planted in `base_dir` pointing outside it
///    (nothing stops a malicious or compromised sound-pack archive from
///    shipping one). If the joined path already exists, it is canonicalized
///    and must still start with `base_dir`'s own canonical form - checked
///    *after* resolution, not only before, exactly as the plan requires. If
///    it doesn't exist yet, there is nothing to canonicalize and nothing a
///    symlink could have redirected (a path can't be hijacked via a dirent
///    that isn't there), so the lexically-checked, not-yet-existing
///    candidate is returned as-is; `play_file`'s own `fs::read` simply won't
///    find it, which is the same "nothing to play" outcome as any other
///    never-downloaded MSP sound.
pub fn resolve_msp_local_path(name: &str, base_dir: &Path) -> Result<std::path::PathBuf, &'static str> {
    if name.is_empty() {
        return Err("empty filename");
    }
    if name.contains('/') || name.contains('\\') {
        return Err("path separator not allowed");
    }
    if name.contains(':') {
        return Err("drive letter / colon not allowed");
    }
    if name == "." || name == ".." {
        return Err("current/parent directory reference not allowed");
    }
    // Exactly one Normal component - rejects any remaining Path oddity
    // (e.g. a lone ".." that slipped past the literal check above via some
    // other spelling) without trying to normalise it into something safe.
    let mut components = std::path::Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => {}
        _ => return Err("filename must be a single path component"),
    }

    let _ = std::fs::create_dir_all(base_dir);
    let canonical_base = std::fs::canonicalize(base_dir).map_err(|_| "media directory unavailable")?;
    let candidate = base_dir.join(name);

    match std::fs::canonicalize(&candidate) {
        Ok(canonical_candidate) => {
            if canonical_candidate.starts_with(&canonical_base) {
                Ok(canonical_candidate)
            } else {
                Err("resolved path escapes the media directory")
            }
        }
        // Doesn't exist (yet) - nothing to resolve away from base_dir; see
        // the doc comment above.
        Err(_) => Ok(candidate),
    }
}

/// A reqwest client for a media download: it re-validates **every redirect hop's host**
/// with `media_url_host_allowed`, so a public host can't 302-redirect the fetch to an
/// internal address (security review 2026-11-21 — `curl -L` used to follow a redirect to
/// any host). Normal web-PKI (CA) verification is used, as for any public media host; the
/// pinned `TofuVerifier` is only for Clay/MUD sockets, not this.
fn build_media_client() -> Result<reqwest::Client, reqwest::Error> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MEDIA_MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match media_url_host_allowed(attempt.url()) {
            Ok(()) => attempt.follow(),
            // A redirect to an internal/non-http host: stop here (the bytes gathered so
            // far, if any, are discarded by the caller on the resulting status/None).
            Err(_) => attempt.stop(),
        }
    });
    reqwest::Client::builder()
        .redirect(policy)
        .timeout(std::time::Duration::from_secs(MEDIA_DOWNLOAD_TIMEOUT_SECS))
        .build()
}

/// Fetch `url`'s bytes, following only redirects whose host passes `media_url_host_allowed`.
/// `None` on any error (unreachable, non-2xx, a redirect stopped by the policy). Runs on
/// its own current-thread runtime: `download_to_cache` is called from a plain
/// `std::thread`, never inside the app's tokio runtime.
fn fetch_media(url: &str) -> Option<Vec<u8>> {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().ok()?;
    runtime.block_on(async {
        let client = build_media_client().ok()?;
        let resp = client.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.bytes().await.ok().map(|b| b.to_vec())
    })
}

/// Download a media file to the cache directory, returning the cache path on success.
/// Called from background threads. The URL is a MUD-supplied GMCP/MSP media URL: it and
/// every redirect it follows are validated against the SSRF host block-list
/// (`media_url_host_allowed`).
pub fn download_to_cache(
    url: &str,
    cache_dir: &Path,
) -> Option<std::path::PathBuf> {
    // Reject anything that isn't a plain http/https URL to a non-internal host before we
    // ever touch the filesystem cache path or make a request.
    if validate_media_url(url).is_err() {
        return None;
    }

    let _ = std::fs::create_dir_all(cache_dir);
    let safe_name = url.replace(|c: char| !c.is_alphanumeric() && c != '.', "_");
    let cache_path = cache_dir.join(&safe_name);
    if cache_path.exists() {
        return Some(cache_path);
    }
    let bytes = fetch_media(url)?;
    match std::fs::write(&cache_path, &bytes) {
        Ok(()) => Some(cache_path),
        Err(_) => None,
    }
}

#[cfg(test)]
mod play_file_tests {
    use super::{play_file, AudioBackend};
    use std::path::Path;

    /// T1.7 / Job 4: a nonexistent file must never reach the External arm's
    /// `Command::spawn()` at all — this is the whole point of the `is_file()`
    /// guard added to `play_file` above, not just "the spawn happens to fail."
    /// Using a real player name (`mpv`) rather than something bogus like the
    /// Job 4 tests elsewhere (`true`) makes sure this returns `None` even when
    /// the command itself would exist on PATH.
    #[test]
    fn returns_none_for_nonexistent_path_without_spawning() {
        let backend = AudioBackend::External { player_cmd: "mpv".to_string() };
        assert!(play_file(&backend, Path::new("/nonexistent/x.wav"), 100, 1).is_none());
    }
}

#[cfg(test)]
mod media_url_tests {
    use super::validate_media_url;

    #[test]
    fn rejects_file_scheme() {
        assert!(validate_media_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_other_exotic_schemes() {
        assert!(validate_media_url("gopher://example.com/").is_err());
        assert!(validate_media_url("ftp://example.com/x").is_err());
    }

    #[test]
    fn rejects_link_local_metadata_ip() {
        assert!(validate_media_url("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn rejects_loopback_and_private_ips() {
        assert!(validate_media_url("http://127.0.0.1/x").is_err());
        assert!(validate_media_url("http://10.0.0.5/x").is_err());
        assert!(validate_media_url("http://192.168.1.1/x").is_err());
        assert!(validate_media_url("http://[::1]/x").is_err());
    }

    #[test]
    fn rejects_invalid_url() {
        assert!(validate_media_url("not a url").is_err());
        assert!(validate_media_url("").is_err());
    }

    #[test]
    fn accepts_normal_https_url() {
        assert!(validate_media_url("https://example.com/sound.mp3").is_ok());
    }

    #[test]
    fn accepts_normal_http_url() {
        assert!(validate_media_url("http://cdn.example.com/media/x.wav").is_ok());
    }

    /// The per-hop host check `fetch_media`'s redirect policy applies: a redirect target
    /// that is an internal/non-http host is rejected exactly like an initial URL, so a
    /// public host can't 302 the download to an internal address (security review
    /// 2026-11-21). Also covers the widened IPv6 ULA block.
    #[test]
    fn a_redirect_target_is_held_to_the_same_host_rule() {
        use super::media_url_host_allowed;
        let url = |s: &str| url::Url::parse(s).unwrap();
        for blocked in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1/x", "http://10.0.0.5/x", "http://192.168.1.1/x",
            "http://[::1]/x", "http://[fd00::1]/x", "http://[fc00::1]/x",
            "file:///etc/passwd", "gopher://evil/x",
        ] {
            assert!(media_url_host_allowed(&url(blocked)).is_err(), "should block {blocked}");
        }
        for ok in ["https://cdn.example.com/x.mp3", "http://203.0.113.7/x.wav"] {
            assert!(media_url_host_allowed(&url(ok)).is_ok(), "should allow {ok}");
        }
    }

    /// End to end: the redirect policy `fetch_media` builds stops a redirect to a loopback
    /// (stand-in for "internal") host. A tiny local server 302s to a second local server;
    /// the second must never be reached. The initial request to a loopback host is itself
    /// fine here — the policy fires only on the redirect, which is the real bypass.
    #[test]
    fn the_redirect_policy_blocks_a_redirect_to_an_internal_host() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        // Server B: records whether it was ever reached; it must not be.
        let reached = Arc::new(AtomicBool::new(false));
        let b = TcpListener::bind("127.0.0.1:0").unwrap();
        let b_port = b.local_addr().unwrap().port();
        {
            let reached = reached.clone();
            std::thread::spawn(move || {
                if let Ok((mut s, _)) = b.accept() {
                    reached.store(true, Ordering::SeqCst);
                    let mut buf = [0u8; 256];
                    let _ = s.read(&mut buf);
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad");
                }
            });
        }
        // Server A: 302 to server B.
        let a = TcpListener::bind("127.0.0.1:0").unwrap();
        let a_port = a.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = a.accept() {
                let mut buf = [0u8; 256];
                let _ = s.read(&mut buf);
                let body = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{b_port}/secret\r\nContent-Length: 0\r\n\r\n"
                );
                let _ = s.write_all(body.as_bytes());
            }
        });

        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let got = runtime.block_on(async {
            let client = super::build_media_client().unwrap();
            let resp = client.get(format!("http://127.0.0.1:{a_port}/start")).send().await;
            // The redirect is stopped, so the client returns the 3xx (or an error), never B's body.
            match resp {
                Ok(r) => r.bytes().await.map(|b| b.to_vec()).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!reached.load(Ordering::SeqCst), "the redirect to the internal host was followed");
        assert_ne!(got, b"bad", "server B's body was fetched");
    }
}

/// Job 14 (plan Phase 4): path-traversal coverage for `resolve_msp_local_path` -
/// per the plan, "the most important test in the job." Each `temp_base` call
/// gets its own fresh, uniquely-named directory (keyed by test name + pid) so
/// parallel `cargo test` runs never share or race on one directory.
#[cfg(test)]
mod msp_path_tests {
    use super::resolve_msp_local_path;
    use std::path::PathBuf;

    fn temp_base(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clay_msp_path_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test media dir");
        dir
    }

    #[test]
    fn rejects_forward_slash_separator() {
        let base = temp_base("fwd_slash");
        assert!(resolve_msp_local_path("sub/evil.wav", &base).is_err());
    }

    #[test]
    fn rejects_backslash_separator() {
        let base = temp_base("backslash");
        assert!(resolve_msp_local_path("sub\\evil.wav", &base).is_err());
    }

    #[test]
    fn rejects_parent_directory_components() {
        let base = temp_base("parent");
        assert!(resolve_msp_local_path("..", &base).is_err());
        assert!(resolve_msp_local_path("../evil.wav", &base).is_err());
        assert!(resolve_msp_local_path("a/../../evil.wav", &base).is_err());
    }

    #[test]
    fn rejects_leading_root_absolute_path() {
        let base = temp_base("abs_root");
        assert!(resolve_msp_local_path("/etc/passwd", &base).is_err());
    }

    #[test]
    fn rejects_windows_drive_letter() {
        let base = temp_base("drive_letter");
        assert!(resolve_msp_local_path("C:\\Windows\\System32\\cmd.exe", &base).is_err());
        // No separator at all here - only the colon check catches this one,
        // and Path::new("C:evil.wav").is_absolute() is false even on a
        // Windows build (drive-relative, not drive-absolute), which is
        // exactly the "Path's own component parser is platform-flavoured"
        // trap the doc comment calls out.
        assert!(resolve_msp_local_path("C:evil.wav", &base).is_err());
    }

    #[test]
    fn rejects_empty_name() {
        let base = temp_base("empty");
        assert!(resolve_msp_local_path("", &base).is_err());
    }

    #[test]
    fn accepts_plain_filename_even_when_not_yet_downloaded() {
        // Most bare-name MSP triggers reference a sound pack Clay never
        // pre-populates - this must resolve to a candidate path (so
        // play_file's own fs::read can try it) rather than being rejected
        // just because the file isn't there yet.
        let base = temp_base("plain_absent");
        let result = resolve_msp_local_path("bell.wav", &base).expect("plain filename should resolve");
        assert_eq!(result, base.join("bell.wav"));
    }

    #[test]
    fn accepts_plain_filename_when_present() {
        let base = temp_base("plain_present");
        std::fs::write(base.join("bell.wav"), b"RIFF....").unwrap();
        let result = resolve_msp_local_path("bell.wav", &base).expect("existing plain filename should resolve");
        assert_eq!(
            std::fs::canonicalize(&result).unwrap(),
            std::fs::canonicalize(base.join("bell.wav")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_that_escapes_after_resolution() {
        // The lexical checks alone can't catch this: "evil.wav" is a single,
        // clean path component with no separator/parent-component/drive
        // letter in sight. Only resolving it and checking containment
        // *after* resolution (not just before) catches a symlink planted
        // inside the media directory that points outside it - exactly the
        // plan's "symlinks... make the before-check insufficient on its own."
        let base = temp_base("symlink_escape");
        let outside = temp_base("symlink_escape_target");
        let secret = outside.join("secret.txt");
        std::fs::write(&secret, b"do not play me").unwrap();
        std::os::unix::fs::symlink(&secret, base.join("evil.wav")).unwrap();
        assert!(
            resolve_msp_local_path("evil.wav", &base).is_err(),
            "a symlink escaping the media directory must be rejected after resolution"
        );
    }

    #[cfg(unix)]
    #[test]
    fn accepts_symlink_that_stays_inside_the_directory() {
        let base = temp_base("symlink_inside");
        std::fs::write(base.join("real.wav"), b"RIFF....").unwrap();
        std::os::unix::fs::symlink(base.join("real.wav"), base.join("alias.wav")).unwrap();
        assert!(resolve_msp_local_path("alias.wav", &base).is_ok());
    }
}

#[cfg(all(test, target_os = "linux", feature = "native-audio"))]
mod sound_server_device_tests {
    use super::is_sound_server_device;

    /// The sound server's ALSA devices are preferred over `default`, which on a PipeWire
    /// desktop without the `pipewire-alsa` package can be the card itself - the stream then
    /// plays outside the desktop's mixer and no volume control reaches it.
    #[test]
    fn only_the_sound_servers_devices_are_preferred() {
        assert!(is_sound_server_device("pipewire"));
        assert!(is_sound_server_device("pulse"));
        for card in ["default", "sysdefault:CARD=PCH", "hdmi:CARD=HDMI,DEV=0", "dmix", "null", "lavrate"] {
            assert!(!is_sound_server_device(card), "{card} is a card or a plugin, not the server");
        }
    }
}
