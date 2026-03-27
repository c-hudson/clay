//! Text-to-speech backend for console mode.
//!
//! Uses external TTS commands (espeak, say, PowerShell) to speak text aloud.
//! No additional Rust dependencies needed — works with musl static builds.
//!
//! Web/Android clients use the browser's Web Speech API via ServerSpeak WebSocket messages.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// TTS backend holding the detected command and a handle to the current process.
pub struct TtsBackend {
    /// The TTS command to use (e.g. "espeak", "espeak-ng", "say")
    command: Option<String>,
    /// Handle to the currently running TTS process (for stopping)
    current_process: Arc<Mutex<Option<std::process::Child>>>,
}

// Safety: TtsBackend is only accessed from the App's owning task/thread.
// The Arc<Mutex<Child>> is already Send+Sync, and the rest is just String/Option.
unsafe impl Send for TtsBackend {}

/// Detect available TTS command on the system.
pub fn init_tts() -> TtsBackend {
    let command = detect_tts_command();
    TtsBackend {
        command,
        current_process: Arc::new(Mutex::new(None)),
    }
}

/// Detect which TTS command is available on this system.
fn detect_tts_command() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        // macOS always has "say"
        return Some("say".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        // Windows uses PowerShell for TTS
        return Some("powershell".to_string());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // Linux/Unix: check for espeak-ng first, then espeak, then festival
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

/// Speak text aloud using the TTS backend.
///
/// Strips ANSI escape codes from the text before speaking.
/// Spawns the TTS command as a background process (non-blocking).
/// Kills any previously running TTS process before starting a new one.
pub fn speak(backend: &TtsBackend, text: &str) {
    let cmd = match &backend.command {
        Some(c) => c.clone(),
        None => return, // No TTS available
    };

    // Strip ANSI codes from the text
    let clean_text = crate::util::strip_ansi_codes(text);
    let clean_text = clean_text.trim().to_string();

    if clean_text.is_empty() {
        return;
    }

    // Kill any currently running TTS process
    stop(backend);

    // Spawn the TTS command
    let child = match cmd.as_str() {
        "say" => {
            // macOS
            Command::new("say")
                .arg(&clean_text)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
        "powershell" => {
            // Windows PowerShell
            let script = format!(
                "Add-Type -AssemblyName System.Speech; \
                 $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
                 $s.Speak('{}')",
                clean_text.replace('\'', "''")
            );
            Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        }
        "festival" => {
            // festival uses stdin
            let mut child = Command::new("festival")
                .arg("--tts")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(ref mut c) = child {
                if let Some(ref mut stdin) = c.stdin {
                    use std::io::Write;
                    let _ = stdin.write_all(clean_text.as_bytes());
                    // Drop stdin to signal EOF
                }
                c.stdin = None;
            }
            child
        }
        _ => {
            // espeak, espeak-ng, or any other command
            Command::new(&cmd)
                .arg(&clean_text)
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

/// Stop any currently running TTS process.
pub fn stop(backend: &TtsBackend) {
    if let Ok(mut guard) = backend.current_process.lock() {
        if let Some(ref mut child) = *guard {
            let _ = child.kill();
            let _ = child.wait();
        }
        *guard = None;
    }
}

/// Returns true if a TTS command was detected on this system.
pub fn is_available(backend: &TtsBackend) -> bool {
    backend.command.is_some()
}

/// Returns the name of the detected TTS command, if any.
pub fn command_name(backend: &TtsBackend) -> Option<&str> {
    backend.command.as_deref()
}
