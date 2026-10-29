//! Text-to-speech backend for Clay MUD client.
//!
//! Three backends:
//! - **Local**: External TTS commands (espeak, say, PowerShell). Works offline.
//! - **Kokoro**: the Kokoro-82M neural voice, in-process through a runtime that
//!   `/tts install` downloads (`src/kokoro.rs`). Offline; falls back to Local until the
//!   files are installed.
//! - **Edge**: Microsoft Edge neural TTS via cloud WebSocket API. Higher quality, needs internet.
//!   Uses the same tokio-tungstenite + rustls stack as the rest of Clay (no openssl needed).
//!
//! Web/Android clients use the browser's Web Speech API via ServerSpeak WebSocket messages.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

/// TTS mode setting
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TtsMode {
    #[default]
    Off,
    Local,  // espeak, say, PowerShell
    Kokoro, // Kokoro-82M through sherpa-onnx (src/kokoro.rs)
    Edge,   // Microsoft Edge neural TTS
}

impl TtsMode {
    pub fn name(&self) -> &'static str {
        match self {
            TtsMode::Off => "off",
            TtsMode::Local => "local",
            TtsMode::Kokoro => "kokoro",
            TtsMode::Edge => "edge",
        }
    }

    pub fn from_name(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "local" => TtsMode::Local,
            "kokoro" => TtsMode::Kokoro,
            "edge" => TtsMode::Edge,
            _ => TtsMode::Off,
        }
    }

    /// The selector entries every settings UI lists, in order.
    pub const OPTIONS: [(&'static str, &'static str); 4] =
        [("off", "Off"), ("local", "Local"), ("kokoro", "Kokoro"), ("edge", "Edge")];
}

/// TTS speak mode — controls which lines are spoken
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TtsSpeakMode {
    #[default]
    All,    // Speak all non-gagged output
    Limit,  // Only speak lines matching say patterns or whitelisted speakers
}

impl TtsSpeakMode {
    pub fn name(&self) -> &'static str {
        match self {
            TtsSpeakMode::All => "all",
            TtsSpeakMode::Limit => "limit",
        }
    }

    pub fn from_name(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "limit" => TtsSpeakMode::Limit,
            _ => TtsSpeakMode::All,
        }
    }
}

/// One interface's own text-to-speech (user request 2026-10-28: "each interface can have
/// its own setting that can only be set while in that interface"): the engine, which
/// lines, and Kokoro's voice. The console's own is `Settings::tts_mode`/`tts_speak_mode`/
/// `tts_voice`; every other interface's is in `Settings::tts_profiles` under its interface
/// key, Off until that interface chose one. Each client is sent speech by its own.
#[derive(Debug, Clone, PartialEq)]
pub struct TtsProfile {
    pub mode: TtsMode,
    pub speak_mode: TtsSpeakMode,
    pub voice: String,
}

impl Default for TtsProfile {
    fn default() -> Self {
        TtsProfile { mode: TtsMode::Off, speak_mode: TtsSpeakMode::All, voice: crate::kokoro::DEFAULT_VOICE.to_string() }
    }
}

impl TtsProfile {
    /// From the wire's names (an empty voice is the default one).
    pub fn from_names(mode: &str, speak_mode: &str, voice: &str) -> Self {
        TtsProfile {
            mode: TtsMode::from_name(mode),
            speak_mode: TtsSpeakMode::from_name(speak_mode),
            voice: if voice.trim().is_empty() { crate::kokoro::DEFAULT_VOICE.to_string() } else { crate::kokoro::normalize_voice(voice) },
        }
    }

    pub fn from_setting(s: &crate::websocket::TtsSetting) -> Self {
        Self::from_names(&s.mode, &s.speak_mode, &s.voice)
    }

    pub fn to_setting(&self) -> crate::websocket::TtsSetting {
        crate::websocket::TtsSetting {
            mode: self.mode.name().to_string(),
            speak_mode: self.speak_mode.name().to_string(),
            voice: self.voice.clone(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.mode != TtsMode::Off
    }
}

/// Check if a line should be spoken in Limit mode.
/// Detects "X says, ..." patterns and builds a whitelist of speaker names.
/// Lines starting with a whitelisted name are also spoken.
pub fn should_speak(text: &str, whitelist: &mut std::collections::HashSet<String>) -> bool {
    use std::sync::OnceLock;
    static SAY_REGEX: OnceLock<regex::Regex> = OnceLock::new();
    let re = SAY_REGEX.get_or_init(|| {
        regex::Regex::new(r#"(?i)^(\S+) (?:says?|asks?),? [""\u{201c}\u{201d}']"#).unwrap()
    });

    let stripped = crate::util::strip_ansi_codes(text);
    let stripped = crate::util::strip_mud_tag(&stripped);
    let trimmed = stripped.trim();

    // Check if line matches "X says, ..." pattern
    if let Some(caps) = re.captures(trimmed) {
        if let Some(name) = caps.get(1) {
            whitelist.insert(name.as_str().to_string());
        }
        return true;
    }

    // Check if line starts with any whitelisted name
    for name in whitelist.iter() {
        if trimmed.starts_with(name.as_str()) {
            return true;
        }
    }

    false
}

/// TTS backend holding the detected local command and Edge TTS state.
pub struct TtsBackend {
    /// The local TTS command to use (e.g. "espeak", "espeak-ng", "say")
    local_command: Option<String>,
    /// Handle to the currently running local TTS process (for stopping)
    current_process: Arc<Mutex<Option<std::process::Child>>>,
    /// Queue for Edge TTS — ensures utterances play one at a time
    edge_queue_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// The Kokoro engine, started on the first Kokoro utterance (its worker thread loads
    /// the model then; see src/kokoro.rs).
    kokoro: OnceLock<crate::kokoro::Engine>,
}

// Safety: TtsBackend is only accessed from the App's owning task/thread.
unsafe impl Send for TtsBackend {}

/// Initialize the TTS backend.
pub fn init_tts() -> TtsBackend {
    let tokio_handle = tokio::runtime::Handle::try_current().ok();

    // Spawn Edge TTS queue processor — plays utterances one at a time
    let edge_queue_tx = if let Some(ref handle) = tokio_handle {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        handle.spawn(async move {
            while let Some(text) = rx.recv().await {
                if let Err(e) = edge_tts_speak(&text).await {
                    crate::debug_log(crate::is_debug_enabled(), &format!("Edge TTS error: {}", e));
                }
            }
        });
        Some(tx)
    } else {
        None
    };

    TtsBackend {
        local_command: detect_tts_command(),
        current_process: Arc::new(Mutex::new(None)),
        edge_queue_tx,
        kokoro: OnceLock::new(),
    }
}

/// Detect which local TTS command is available on this system.
fn detect_tts_command() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        Some("say".to_string())
    }

    #[cfg(target_os = "windows")]
    {
        Some("powershell".to_string())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // Skipped for --local-server (see LOCAL_SERVER_LOOPBACK_ONLY): the Android app speaks
        // with the phone's own voice, and starting a subprocess from the app's bundled server
        // hangs it at random - this `which` stalled App::new in about half of the emulator's
        // cold starts, so the server never listened and the app opened on a blank screen.
        if crate::LOCAL_SERVER_LOOPBACK_ONLY.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        for cmd in &["espeak-ng", "espeak", "festival"] {
            if Command::new("which")
                .arg(cmd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
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

/// Speak text aloud using the specified TTS mode. `voice` is the Kokoro voice name
/// (`Settings::tts_voice`); the other modes ignore it. Returns true when Kokoro was asked
/// for but its files are not installed here, so the Local engine spoke instead - the
/// caller tells the user, since a silently different voice reads as "the setting does
/// nothing".
pub fn speak(backend: &TtsBackend, text: &str, mode: TtsMode, voice: &str) -> bool {
    let clean_text = crate::util::strip_ansi_codes(text);
    let clean_text = clean_text.trim().to_string();

    if clean_text.is_empty() {
        return false;
    }

    match mode {
        TtsMode::Off => {}
        TtsMode::Local => speak_local(backend, &clean_text),
        TtsMode::Kokoro => return speak_kokoro(backend, &clean_text, voice),
        TtsMode::Edge => speak_edge(backend, &clean_text),
    }
    false
}

/// Speak with Kokoro (src/kokoro.rs), or with the Local engine while its files are not
/// installed - so choosing the mode before `/tts install` still says something.
/// Returns true when it fell back to the Local engine because Kokoro is not installed.
fn speak_kokoro(backend: &TtsBackend, text: &str, voice: &str) -> bool {
    if !crate::kokoro::install().complete() {
        crate::debug_log(crate::is_debug_enabled(), "Kokoro: not installed, speaking with the local engine instead");
        speak_local(backend, text);
        return true;
    }
    stop_local(backend);
    backend.kokoro.get_or_init(crate::kokoro::Engine::new).speak(text, voice);
    false
}

/// The Kokoro engine, if an utterance has started it (for `/tts status`).
pub fn kokoro_engine(backend: &TtsBackend) -> Option<&crate::kokoro::Engine> {
    backend.kokoro.get()
}

/// Load the Kokoro model for `voice` now, in the background, so the first line spoken
/// does not pay for it (`kokoro::Engine::warm`). Nothing without the files.
pub fn warm_kokoro(backend: &TtsBackend, voice: &str) {
    if backend.kokoro.get().is_none() {
        // No session yet in this process: the one moment a leftover build can go.
        crate::kokoro::remove_stale_build();
    }
    if crate::kokoro::install().complete() {
        backend.kokoro.get_or_init(crate::kokoro::Engine::new).warm(voice);
    }
}

/// Kokoro is no longer the engine: unload its model and close its audio stream, if an
/// engine exists (`kokoro::Engine::release`). The engine itself stays for a later
/// return to Kokoro. Nothing to do when no line or warm-up ever started one.
pub fn release_kokoro(backend: &TtsBackend) {
    if let Some(k) = backend.kokoro.get() {
        k.release();
    }
}

/// Speak using local TTS command (espeak, say, PowerShell).
fn speak_local(backend: &TtsBackend, text: &str) {
    let cmd = match &backend.local_command {
        Some(c) => c.clone(),
        None => return,
    };

    stop_local(backend);

    let child = match cmd.as_str() {
        "say" => {
            Command::new("say")
                .arg(text)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
        "powershell" => {
            let script = format!(
                "Add-Type -AssemblyName System.Speech; \
                 $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
                 $s.Speak('{}')",
                text.replace('\'', "''")
            );
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
        "festival" => {
            let mut child = Command::new("festival")
                .arg("--tts")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(ref mut c) = child {
                if let Some(ref mut stdin) = c.stdin {
                    use std::io::Write;
                    let _ = stdin.write_all(text.as_bytes());
                }
                c.stdin = None;
            }
            child
        }
        _ => {
            Command::new(&cmd)
                .arg(text)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
    };

    if let Ok(child) = child {
        if let Ok(mut guard) = backend.current_process.lock() {
            *guard = Some(child);
        }
    }
}

/// Speak using Microsoft Edge neural TTS via WebSocket.
/// Queued — utterances play one at a time, never overlapping.
fn speak_edge(backend: &TtsBackend, text: &str) {
    if let Some(ref tx) = backend.edge_queue_tx {
        let _ = tx.send(text.to_string());
    }
}

/// Generate the Sec-MS-GEC DRM token required by Edge TTS API.
/// Based on current time rounded to 5-minute intervals, hashed with the trusted client token.
fn generate_sec_ms_gec() -> String {
    use sha2::{Sha256, Digest};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    // Convert to Windows file time epoch (1601-01-01)
    let ticks = now + 11644473600.0;
    // Round down to nearest 5 minutes
    let ticks = ticks - (ticks % 300.0);
    // Convert to 100-nanosecond intervals
    let ticks = ticks * 1e7;
    let str_to_hash = format!("{:.0}6A5AA1D4EAFF4E9FB37E23D68491D6F4", ticks);
    let hash = Sha256::digest(str_to_hash.as_bytes());
    hex::encode(hash).to_uppercase()
}

/// Edge TTS WebSocket protocol implementation.
/// Connects to Microsoft's TTS endpoint, sends SSML, receives MP3 audio.
async fn edge_tts_speak(text: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio_tungstenite::tungstenite::Message;
    use futures::StreamExt;

    let request_id = uuid::Uuid::new_v4().to_string().replace('-', "");
    let voice = "en-US-AriaNeural";
    let output_format = "audio-24khz-48kbitrate-mono-mp3";

    // Generate DRM token
    let sec_ms_gec = generate_sec_ms_gec();
    let sec_ms_gec_version = "1-143.0.3650.75";

    // Build the WebSocket URL with required parameters
    let ws_url = format!(
        "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4&ConnectionId={}&Sec-MS-GEC={}&Sec-MS-GEC-Version={}",
        request_id, sec_ms_gec, sec_ms_gec_version
    );

    // Build request with required headers
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(&ws_url)
        .header("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0")
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tokio_tungstenite::tungstenite::handshake::client::generate_key())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Host", "speech.platform.bing.com")
        .body(())?;

    // Connect via tungstenite with rustls
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await?;

    // Send speech config
    let config_msg = format!(
        "Content-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
        {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"{}\"}}}}}}}}",
        output_format
    );
    {
        use futures::SinkExt;
        ws.send(Message::Text(config_msg)).await?;
    }

    // Escape text for SSML
    let escaped_text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");

    // Send SSML synthesis request
    let ssml = format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
        <voice name='{}'>{}</voice></speak>",
        voice, escaped_text
    );

    let ssml_msg = format!(
        "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nPath:ssml\r\n\r\n{}",
        request_id, ssml
    );
    {
        use futures::SinkExt;
        ws.send(Message::Text(ssml_msg)).await?;
    }

    // Collect audio bytes from response
    let mut audio_data: Vec<u8> = Vec::new();
    let header_separator = b"Path:audio\r\n";

    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Binary(data) => {
                // Binary messages contain audio data after a header
                // Find "Path:audio\r\n" in the binary data and extract audio after it
                if let Some(pos) = find_subsequence(&data, header_separator) {
                    let audio_start = pos + header_separator.len();
                    if audio_start < data.len() {
                        audio_data.extend_from_slice(&data[audio_start..]);
                    }
                }
            }
            Message::Text(text) => {
                // Check for turn.end which signals completion
                if text.contains("Path:turn.end") {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Play the collected MP3 audio
    if !audio_data.is_empty() {
        play_mp3_bytes_async(audio_data).await;
    }

    Ok(())
}

/// Find a subsequence in a byte slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Play MP3 bytes via rodio.
async fn play_mp3_bytes_async(data: Vec<u8>) {
    #[cfg(feature = "native-audio")]
    {
        // Spawn blocking since rodio playback blocks
        let _ = tokio::task::spawn_blocking(move || {
            use std::io::Cursor;
            if let Ok((_stream, stream_handle)) = rodio::OutputStream::try_default() {
                if let Ok(source) = rodio::Decoder::new(Cursor::new(data)) {
                    if let Ok(sink) = rodio::Sink::try_new(&stream_handle) {
                        sink.append(source);
                        sink.sleep_until_end();
                    }
                }
            }
        }).await;
    }

    #[cfg(not(feature = "native-audio"))]
    {
        // Without rodio, write to temp file and play with external player
        use std::io::Write;
        let tmp_path = std::env::temp_dir().join("clay_edge_tts.mp3");
        if let Ok(mut f) = std::fs::File::create(&tmp_path) {
            let _ = f.write_all(&data);
            drop(f);
            // Try external players with appropriate flags for each
            let tmp = tmp_path.to_str().unwrap_or("");
            let players: &[(&str, &[&str])] = &[
                ("mpv", &["--no-video", "--really-quiet", tmp]),
                ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet", tmp]),
                ("play", &["-q", tmp]),
                ("aplay", &[tmp]),  // ALSA player (wav only, but worth trying)
                ("paplay", &[tmp]), // PulseAudio player
            ];
            for (player, args) in players {
                if Command::new(player)
                    .args(*args)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
                {
                    break;
                }
            }
            let _ = std::fs::remove_file(&tmp_path);
        }
    }
}

/// Stop any currently running TTS.
pub fn stop(backend: &TtsBackend) {
    stop_local(backend);
    if let Some(k) = backend.kokoro.get() {
        k.stop();
    }
}

fn stop_local(backend: &TtsBackend) {
    if let Ok(mut guard) = backend.current_process.lock() {
        if let Some(ref mut child) = *guard {
            let _ = child.kill();
            let _ = child.wait();
        }
        *guard = None;
    }
}

/// Returns true if a local TTS command was detected.
pub fn is_available(backend: &TtsBackend) -> bool {
    backend.local_command.is_some()
}

/// Returns the name of the detected local TTS command, if any.
pub fn command_name(backend: &TtsBackend) -> Option<&str> {
    backend.local_command.as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that speaks to nobody: no local command, no Edge queue.
    fn silent() -> TtsBackend {
        TtsBackend {
            local_command: None,
            current_process: Arc::new(Mutex::new(None)),
            edge_queue_tx: None,
            kokoro: OnceLock::new(),
        }
    }

    /// Kokoro chosen without its files (the test home has none): the Local engine is used
    /// and the caller is told, so it can say so; the other modes never report a fallback.
    #[test]
    fn kokoro_without_files_reports_the_fallback() {
        let backend = silent();
        assert!(!crate::kokoro::install().complete(), "the test home has no Kokoro files");
        assert!(speak(&backend, "hello", TtsMode::Kokoro, "af_heart"), "Kokoro asked for, Local spoke: say so");
        assert!(!speak(&backend, "hello", TtsMode::Local, "af_heart"));
        assert!(!speak(&backend, "hello", TtsMode::Off, "af_heart"));
        assert!(!speak(&backend, "   ", TtsMode::Kokoro, "af_heart"), "nothing to say: nothing to report");
    }

    /// Leaving Kokoro tells the engine to unload (and is nothing without an engine).
    #[test]
    fn release_reaches_the_engine_when_there_is_one() {
        let backend = silent();
        release_kokoro(&backend);
        assert!(backend.kokoro.get().is_none(), "a release starts no engine");
        let engine = backend.kokoro.get_or_init(crate::kokoro::Engine::new);
        release_kokoro(&backend);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while engine.releases() == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(engine.releases(), 1);
    }
}
