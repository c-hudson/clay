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

/// Download a file to the cache directory.
/// Returns the cache path on success. Uses curl for the download.
/// Called from background threads.
pub fn download_to_cache(
    url: &str,
    cache_dir: &Path,
) -> Option<std::path::PathBuf> {
    let _ = std::fs::create_dir_all(cache_dir);
    let safe_name = url.replace(|c: char| !c.is_alphanumeric() && c != '.', "_");
    let cache_path = cache_dir.join(&safe_name);
    if cache_path.exists() {
        return Some(cache_path);
    }
    let cache_path_str = cache_path.to_string_lossy().to_string();
    let status = std::process::Command::new("curl")
        .args(["-sL", "-o", &cache_path_str, url])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if status.is_ok() && cache_path.exists() {
        Some(cache_path)
    } else {
        None
    }
}
