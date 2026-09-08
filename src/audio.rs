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

/// Initialize the audio backend.
/// Tries rodio first (if compiled in), falls back to external player detection.
pub fn init_audio() -> AudioBackend {
    #[cfg(feature = "native-audio")]
    {
        if let Ok((stream, handle)) = rodio::OutputStream::try_default() {
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
    { return None; }

    #[cfg(not(windows))]
    {
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

/// B5 (security remediation): validate a MUD-supplied GMCP `Client.Media.*` URL before
/// it's ever handed to curl. `download_to_cache` is the single choke point both media
/// call sites (`main.rs` "Play" and "Load") route through, so validating here covers
/// both.
///
/// - Only `http`/`https` schemes are allowed — this alone blocks `file://` (arbitrary
///   local file read, e.g. `file:///etc/passwd`) and other exotic schemes (`gopher://`,
///   etc.).
/// - The host is rejected if it's a *literal* loopback/link-local/private (RFC1918) IP —
///   this blocks the common SSRF targets, notably the cloud-metadata endpoint
///   `169.254.169.254` (link-local).
///
/// Known limitation (documented rather than solved here): this only inspects the literal
/// host in the URL. A *hostname* that resolves to an internal address (or one that's
/// rebound between this check and curl's own DNS resolution — "DNS rebinding") is not
/// caught. Closing that fully would mean resolving the hostname ourselves and pinning
/// curl to the resolved IP (e.g. via `curl --resolve` or `--connect-to`), which is more
/// invasive than this Phase-B pass; left as a follow-up (see SECURITY-ROADMAP.md).
fn validate_media_url(url_str: &str) -> Result<url::Url, &'static str> {
    let parsed = url::Url::parse(url_str).map_err(|_| "invalid URL")?;
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
            // fe80::/10 = link-local unicast (top 10 bits of the first segment).
            let is_link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            if ip.is_loopback() || ip.is_unspecified() || is_link_local {
                return Err("target host is a loopback/link-local IPv6 address");
            }
            if let Some(v4) = ip.to_ipv4_mapped() {
                if v4.is_loopback() || v4.is_link_local() || v4.is_private() || v4.is_unspecified() {
                    return Err("target host is a loopback/link-local/private address (IPv4-mapped)");
                }
            }
        }
        Some(url::Host::Domain(_)) => {} // resolved by curl at connect time; see limitation above
        None => return Err("URL has no host"),
    }
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

/// Download a file to the cache directory.
/// Returns the cache path on success. Uses curl for the download.
/// Called from background threads.
pub fn download_to_cache(
    url: &str,
    cache_dir: &Path,
) -> Option<std::path::PathBuf> {
    // B5: reject anything that isn't a plain http/https URL to a non-internal host
    // before we ever touch the filesystem cache path or spawn curl.
    if validate_media_url(url).is_err() {
        return None;
    }

    let _ = std::fs::create_dir_all(cache_dir);
    let safe_name = url.replace(|c: char| !c.is_alphanumeric() && c != '.', "_");
    let cache_path = cache_dir.join(&safe_name);
    if cache_path.exists() {
        return Some(cache_path);
    }
    let cache_path_str = cache_path.to_string_lossy().to_string();
    let status = std::process::Command::new("curl")
        .args([
            "-s", "-o", &cache_path_str,
            // B5: constrain both the initial request and any redirect target to
            // http/https — matches the scheme check above but also stops a same-scheme
            // redirect chain from hopping to a disallowed scheme mid-flight.
            "--proto", "=http,https",
            "--proto-redir", "=http,https",
            "-L",
            url,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if status.is_ok() && cache_path.exists() {
        Some(cache_path)
    } else {
        None
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
