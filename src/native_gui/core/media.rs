//! Sound and speech: a MUD's media (GMCP `Client.Media`, MCMP), ANSI music and the
//! server's text-to-speech, as app.js plays them (`handleMcmpMedia`, `playAnsiMusic`,
//! `ServerSpeak`), through the console's players (`audio.rs`, `tts.rs`).
//!
//! A media file is fetched off the GUI thread by `audio::download_to_cache`, which turns
//! away anything but http(s) to a public address, and starts playing at the next `poll`
//! if it is still wanted by then (a Stop may have come first).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::ansi_music::MusicNote;
use crate::audio::{self, AudioBackend, PlayHandle};
use crate::tts::{self, TtsBackend, TtsMode};

/// Something playing, by the key and name a Stop can name it by.
struct Playing {
    key: String,
    name: String,
    handle: PlayHandle,
}

/// A Play waiting for its file.
struct Want {
    id: u64,
    key: String,
    name: String,
    music: bool,
    volume: i64,
    loops: i64,
}

/// A Play's settings, from its JSON (app.js `mcmpPlay`'s defaults).
#[derive(Debug, PartialEq)]
pub struct Play {
    pub url: String,
    pub music: bool,
    pub volume: i64,
    pub loops: i64,
    pub key: String,
    pub name: String,
    pub continue_: bool,
}

/// What a Stop names (app.js `mcmpStop`).
#[derive(Debug, Default, PartialEq)]
pub struct Stop {
    pub music: bool,
    pub sounds: bool,
    pub key: String,
    pub name: String,
}

fn text(v: &serde_json::Value, field: &str) -> String {
    v.get(field).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// app.js `mcmpResolveUrl`: the name under the play's own url, else the world's default
/// (`Client.Media.Default`), else the server's; a name that is a whole URL stands alone.
pub fn resolve_url(data: &serde_json::Value, default_url: &str, server_default: &str) -> String {
    let name = text(data, "name");
    if name.starts_with("http://") || name.starts_with("https://") {
        return name;
    }
    let own = text(data, "url");
    let mut base = [own.as_str(), default_url, server_default].into_iter().find(|u| !u.is_empty()).unwrap_or("").to_string();
    if base.is_empty() {
        return String::new();
    }
    if !base.ends_with('/') {
        base.push('/');
    }
    base + &name
}

pub fn parse_play(data: &serde_json::Value, default_url: &str, server_default: &str) -> Option<Play> {
    let url = resolve_url(data, default_url, server_default);
    if url.is_empty() {
        return None;
    }
    let name = text(data, "name");
    let key = [text(data, "key"), name.clone(), url.clone()].into_iter().find(|k| !k.is_empty()).unwrap_or_default();
    Some(Play {
        music: text(data, "type").eq_ignore_ascii_case("music"),
        volume: data.get("volume").and_then(|v| v.as_i64()).unwrap_or(50).clamp(0, 100),
        loops: data.get("loops").and_then(|v| v.as_i64()).unwrap_or(1),
        continue_: data.get("continue").and_then(|v| v.as_bool()).unwrap_or(true),
        name: if name.is_empty() { url.clone() } else { name },
        key,
        url,
    })
}

pub fn parse_stop(data: &serde_json::Value) -> Stop {
    let kind = text(data, "type").to_lowercase();
    let (key, name) = (text(data, "key"), text(data, "name"));
    let everything = kind.is_empty() && key.is_empty() && name.is_empty();
    Stop { music: kind == "music" || everything, sounds: kind == "sound" || everything, key, name }
}

pub struct Media {
    runtime: Option<tokio::runtime::Handle>,
    audio: Option<AudioBackend>,
    tts: Option<TtsBackend>,
    cache_dir: PathBuf,
    /// The MUD's `Client.Media.Default`.
    default_url: String,
    music: Option<Playing>,
    sounds: HashMap<String, Playing>,
    ansi: Option<PlayHandle>,
    ansi_count: u32,
    wanted: Vec<Want>,
    next_id: u64,
    fetched_tx: mpsc::Sender<(u64, PathBuf)>,
    fetched_rx: mpsc::Receiver<(u64, PathBuf)>,
}

impl Media {
    pub fn new(runtime: Option<tokio::runtime::Handle>) -> Media {
        let (fetched_tx, fetched_rx) = mpsc::channel();
        Media {
            runtime,
            audio: None,
            tts: None,
            cache_dir: crate::clay_config_dir().join("media"),
            default_url: String::new(),
            music: None,
            sounds: HashMap::new(),
            ansi: None,
            ansi_count: 0,
            wanted: Vec::new(),
            next_id: 0,
            fetched_tx,
            fetched_rx,
        }
    }

    /// Opened the first time something plays (rodio, which tries every output device
    /// when the default won't open; else mpv or ffplay).
    fn audio(&mut self) -> &AudioBackend {
        self.audio.get_or_insert_with(audio::init_audio)
    }

    /// ANSI music: a square wave, as the web client's oscillator plays it. A new tune
    /// cuts off the one playing.
    pub fn ansi_music(&mut self, notes: &[MusicNote]) {
        if let Some(mut h) = self.ansi.take() {
            h.stop();
        }
        let wav = crate::generate_wav_from_notes(notes);
        self.ansi_count = self.ansi_count.wrapping_add(1);
        let (dir, n) = (self.cache_dir.clone(), self.ansi_count);
        self.ansi = audio::play_wav_memory(self.audio(), wav, &dir, n);
    }

    /// A `McmpMedia` message: Default, Play, Stop or Load (`data` is the GMCP JSON).
    pub fn mcmp(&mut self, action: &str, data: &str, server_default: &str) {
        let Ok(data) = serde_json::from_str::<serde_json::Value>(data) else { return };
        match action {
            "Default" => {
                let url = text(&data, "url");
                if !url.is_empty() {
                    self.default_url = url;
                }
            }
            "Play" => {
                if let Some(p) = parse_play(&data, &self.default_url, server_default) {
                    self.play(p);
                }
            }
            "Stop" => self.stop(&parse_stop(&data)),
            "Load" => {
                let url = resolve_url(&data, &self.default_url, server_default);
                if !url.is_empty() {
                    self.fetch(url, None);
                }
            }
            _ => {}
        }
    }

    fn play(&mut self, p: Play) {
        if p.music {
            // One tune at a time; the same one again with continue only changes its volume.
            if let Some(m) = &self.music {
                if p.continue_ && m.name == p.name {
                    m.handle.set_volume(p.volume);
                    return;
                }
            }
            if let Some(mut m) = self.music.take() {
                m.handle.kill();
            }
            self.wanted.retain(|w| !w.music);
        }
        self.next_id += 1;
        let id = self.next_id;
        self.wanted.push(Want { id, key: p.key, name: p.name, music: p.music, volume: p.volume, loops: p.loops });
        self.fetch(p.url, Some(id));
    }

    fn fetch(&self, url: String, id: Option<u64>) {
        let (dir, tx) = (self.cache_dir.clone(), self.fetched_tx.clone());
        std::thread::spawn(move || {
            if let (Some(path), Some(id)) = (audio::download_to_cache(&url, &dir), id) {
                let _ = tx.send((id, path));
            }
        });
    }

    fn stop(&mut self, s: &Stop) {
        let hits = |p: &Playing| (!s.key.is_empty() && p.key == s.key) || (s.key.is_empty() && !s.name.is_empty() && p.name == s.name);
        if self.music.as_ref().is_some_and(|m| s.music || hits(m)) {
            if let Some(mut m) = self.music.take() {
                m.handle.kill();
            }
        }
        let gone: Vec<String> = self.sounds.iter().filter(|(_, p)| s.sounds || hits(p)).map(|(k, _)| k.clone()).collect();
        for k in gone {
            if let Some(mut p) = self.sounds.remove(&k) {
                p.handle.kill();
            }
        }
        // And what hasn't started yet.
        self.wanted.retain(|w| {
            let named = (!s.key.is_empty() && w.key == s.key) || (s.key.is_empty() && !s.name.is_empty() && w.name == s.name);
            !((w.music && s.music) || (!w.music && s.sounds) || named)
        });
    }

    /// Everything the MUD started (switching worlds, turning its media off).
    pub fn stop_all(&mut self) {
        self.stop(&Stop { music: true, sounds: true, ..Default::default() });
    }

    /// Start what's been fetched, and forget sounds that have finished.
    pub fn poll(&mut self) {
        while let Ok((id, path)) = self.fetched_rx.try_recv() {
            let Some(at) = self.wanted.iter().position(|w| w.id == id) else { continue };
            let w = self.wanted.remove(at);
            let Some(handle) = audio::play_file(self.audio(), &path, w.volume, w.loops) else { continue };
            let playing = Playing { key: w.key.clone(), name: w.name, handle };
            if w.music {
                if let Some(mut m) = self.music.replace(playing) {
                    m.handle.kill();
                }
            } else if let Some(mut old) = self.sounds.insert(w.key, playing) {
                old.handle.kill();
            }
        }
        self.sounds.retain(|_, p| p.handle.try_wait() != Some(true));
    }

    /// The server's text-to-speech, in the voice the settings name (local when off: the
    /// server only asks when something should be spoken, as the web client assumes).
    /// Returns true when Kokoro was asked for but is not installed on this machine, so
    /// the local engine spoke (`tts::speak`).
    pub fn speak(&mut self, text: &str, mode: TtsMode, voice: &str) -> bool {
        let mode = if mode == TtsMode::Off { TtsMode::Local } else { mode };
        if self.tts.is_none() {
            // Edge's voices run on the runtime.
            let _guard = self.runtime.as_ref().map(|r| r.enter());
            self.tts = Some(tts::init_tts());
        }
        self.tts.as_ref().is_some_and(|t| tts::speak(t, text, mode, voice))
    }

    /// The Kokoro engine, once an utterance has started it (`/tts status`, `/tts remove`).
    pub fn kokoro_engine(&self) -> Option<&crate::kokoro::Engine> {
        self.tts.as_ref().and_then(tts::kokoro_engine)
    }

    /// Kokoro is no longer the engine: unload its model and close its audio stream
    /// (`tts::release_kokoro`; nothing when no engine was ever started).
    pub fn release_kokoro(&self) {
        if let Some(t) = &self.tts {
            tts::release_kokoro(t);
        }
    }

    /// Load the Kokoro model for `voice` in the background, so this GUI's first line
    /// does not wait for it (`tts::warm_kokoro`; nothing without the files here).
    pub fn warm_kokoro(&mut self, voice: &str) {
        if self.tts.is_none() {
            let _guard = self.runtime.as_ref().map(|r| r.enter());
            self.tts = Some(tts::init_tts());
        }
        if let Some(t) = &self.tts {
            tts::warm_kokoro(t, voice);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_resolve_as_app_js_resolves_them() {
        let d = json!({"name": "rain.mp3"});
        assert_eq!(resolve_url(&d, "", ""), "", "nowhere to fetch from");
        assert_eq!(resolve_url(&d, "https://mud.example/media", ""), "https://mud.example/media/rain.mp3");
        assert_eq!(resolve_url(&d, "", "https://srv.example/"), "https://srv.example/rain.mp3");
        assert_eq!(resolve_url(&json!({"name": "rain.mp3", "url": "https://own.example/m/"}), "https://mud.example/", ""), "https://own.example/m/rain.mp3", "the play's own url first");
        assert_eq!(resolve_url(&json!({"name": "https://x.example/a.ogg"}), "", ""), "https://x.example/a.ogg");
    }

    #[test]
    fn plays_take_app_js_defaults() {
        let p = parse_play(&json!({"name": "rain.mp3", "type": "Music"}), "https://m.example", "").expect("play");
        assert_eq!(p, Play { url: "https://m.example/rain.mp3".into(), music: true, volume: 50, loops: 1, key: "rain.mp3".into(), name: "rain.mp3".into(), continue_: true });
        let p = parse_play(&json!({"name": "hit.wav", "key": "combat", "volume": 250, "loops": -1, "continue": false}), "https://m.example", "").expect("play");
        assert_eq!((p.music, p.volume, p.loops, p.key.as_str(), p.continue_), (false, 100, -1, "combat", false));
    }

    #[test]
    fn stops_name_what_app_js_stops() {
        assert_eq!(parse_stop(&json!({})), Stop { music: true, sounds: true, ..Default::default() }, "nothing named: everything");
        assert_eq!(parse_stop(&json!({"type": "sound"})), Stop { sounds: true, ..Default::default() });
        assert_eq!(parse_stop(&json!({"key": "combat"})), Stop { key: "combat".into(), ..Default::default() });
    }

    #[test]
    fn a_stop_before_the_file_arrives_means_it_never_plays() {
        let mut m = Media::new(None);
        m.wanted.push(Want { id: 1, key: "rain".into(), name: "rain.mp3".into(), music: true, volume: 50, loops: 1 });
        m.wanted.push(Want { id: 2, key: "hit".into(), name: "hit.wav".into(), music: false, volume: 50, loops: 1 });
        m.stop(&parse_stop(&json!({"type": "music"})));
        assert_eq!(m.wanted.iter().map(|w| w.id).collect::<Vec<_>>(), vec![2]);
        m.stop(&parse_stop(&json!({"name": "hit.wav"})));
        assert!(m.wanted.is_empty());
    }
}
