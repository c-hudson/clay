//! Kokoro text-to-speech: `TtsMode::Kokoro` in `src/tts.rs`.
//!
//! Kokoro-82M is a small neural voice with no command-line program of its own, so unlike
//! espeak it cannot be spawned per utterance: its model takes about a second to load and
//! has to stay loaded in a running process. Clay runs it in-process through the
//! sherpa-onnx runtime (a C API over ONNX Runtime), which nothing links at build time:
//! `/tts install` downloads the runtime for this platform and the model into
//! `~/.clay/tts` (`CLAY_TTS_DIR` overrides), and this module loads the library with
//! `libloading` on the first utterance. So every build has the mode, the binary stays
//! the size it is, and a build that cannot load a shared library (the static musl
//! console) says so instead of failing silently.
//!
//! Speech runs on one worker thread per [`Engine`]: it loads the model lazily, synthesizes
//! sentence by sentence (sherpa-onnx calls back after each, so playback starts before the
//! whole line is done) and feeds the samples to rodio - or, without `native-audio`, writes
//! a WAV and runs an external player. Like the Local engine it is "latest wins": a new
//! utterance stops the one playing and abandons anything still queued. The runtime is
//! pinned to one sherpa-onnx version ([`RUNTIME_VERSION`]), because the C structs below
//! mirror that version's header field for field; a newer runtime goes in with a header
//! check, never by bumping the constant alone.

use std::ffi::{c_char, c_void, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// The sherpa-onnx release whose C API the structs in this file mirror.
pub const RUNTIME_VERSION: &str = "1.13.8";
/// Which build of the Kokoro model to run - the two the sherpa-onnx release carries.
/// The int8 one is a third of the size, and faster only where the CPU has instructions
/// for 8-bit matrix maths (x86 AVX-512 VNNI; ARM's dot product). On an x86 CPU with just
/// AVX2 it is close to four times *slower* than the full one, because ONNX Runtime's
/// quantized kernels have no fast path there: 2.7 s against 0.7 s for a 3.6 s sentence
/// on a Ryzen 5600X, and slower than real time on an i5-8400T, where a line took 1.65 s
/// to start. So the install picks by CPU (`preferred_model`), `/tts status` says which
/// is there, and `/tts install` switches an install that has the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    Int8,
    Full,
}

impl ModelKind {
    /// The model directory name inside the install dir; also the release asset's stem.
    pub fn dir_name(self) -> &'static str {
        match self {
            ModelKind::Int8 => "kokoro-int8-multi-lang-v1_0",
            ModelKind::Full => "kokoro-multi-lang-v1_0",
        }
    }

    pub fn file_name(self) -> &'static str {
        match self {
            ModelKind::Int8 => "model.int8.onnx",
            ModelKind::Full => "model.onnx",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModelKind::Int8 => "int8",
            ModelKind::Full => "full-precision",
        }
    }

    fn url(self) -> String {
        format!("{MODEL_URL_BASE}{}.tar.bz2", self.dir_name())
    }

    /// The download, roughly, for the install messages (the runtime adds 31 MB).
    pub fn download_mb(self) -> u32 {
        match self {
            ModelKind::Int8 => 132,
            ModelKind::Full => 311,
        }
    }

    fn other(self) -> ModelKind {
        match self {
            ModelKind::Int8 => ModelKind::Full,
            ModelKind::Full => ModelKind::Int8,
        }
    }
}

const MODEL_URL_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/";
const RUNTIME_URL_BASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v";

/// The model build this CPU runs fastest (see `ModelKind`).
pub fn preferred_model() -> ModelKind {
    model_for_cpu(std::env::consts::ARCH, cpu_has_int8_fast_path())
}

/// x86: int8 pays off only with VNNI. Everything else (ARM: every Apple M-series and most
/// phones have the dot-product instructions) gets the small int8 build.
pub(crate) fn model_for_cpu(arch: &str, int8_fast: bool) -> ModelKind {
    match arch {
        "x86_64" | "x86" if !int8_fast => ModelKind::Full,
        _ => ModelKind::Int8,
    }
}

fn cpu_has_int8_fast_path() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx512vnni")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

/// The model file in an installed model directory, whichever build it holds.
pub fn model_file_in(dir: &Path) -> PathBuf {
    let full = dir.join(ModelKind::Full.file_name());
    if full.is_file() { full } else { dir.join(ModelKind::Int8.file_name()) }
}
pub const DEFAULT_VOICE: &str = "af_heart";

/// The model's speakers, in speaker-id order (the id is the index). The first letter is
/// the language (a American English, b British, e Spanish, f French, h Hindi, i Italian,
/// j Japanese, p Brazilian Portuguese, z Mandarin), the second the sex.
pub const VOICES: [&str; 54] = [
    "af_alloy", "af_aoede", "af_bella", "af_heart", "af_jessica", "af_kore", "af_nicole", "af_nova",
    "af_river", "af_sarah", "af_sky", "am_adam", "am_echo", "am_eric", "am_fenrir", "am_liam",
    "am_michael", "am_onyx", "am_puck", "am_santa", "bf_alice", "bf_emma", "bf_isabella", "bf_lily",
    "bm_daniel", "bm_fable", "bm_george", "bm_lewis", "ef_dora", "em_alex", "ff_siwis", "hf_alpha",
    "hf_beta", "hm_omega", "hm_psi", "if_sara", "im_nicola", "jf_alpha", "jf_gongitsune", "jf_nezumi",
    "jf_tebukuro", "jm_kumo", "pf_dora", "pm_alex", "pm_santa", "zf_xiaobei", "zf_xiaoni", "zf_xiaoxiao",
    "zf_xiaoyi", "zm_yunjian", "zm_yunxi", "zm_yunxia", "zm_yunyang", "em_santa",
];

/// A voice name's speaker id, or None for a name the model does not have.
pub fn voice_id(name: &str) -> Option<i32> {
    let name = name.trim().to_ascii_lowercase();
    VOICES.iter().position(|v| *v == name).map(|i| i as i32)
}

/// The language hint sherpa-onnx wants for a voice (`--kokoro-lang`).
pub fn voice_lang(name: &str) -> &'static str {
    match name.trim().chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('e') => "es",
        Some('f') => "fr",
        Some('h') => "hi",
        Some('i') => "it",
        Some('j') => "ja",
        Some('p') => "pt-br",
        Some('z') => "zh",
        _ => "en",
    }
}

/// The lexicon files sherpa-onnx gets for a voice: one English lexicon - GB for the
/// British `b*` voices, US for every other - plus Chinese. Never both English ones: the
/// library warns on stderr for every word the second one repeats ("Duplicated word: ...
/// Ignore it."), and in the console that lands on the screen. A text in another language
/// goes through espeak-ng, which needs no lexicon.
pub fn voice_lexicons(name: &str) -> &'static [&'static str] {
    match name.trim().chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('b') => &["lexicon-gb-en.txt", "lexicon-zh.txt"],
        _ => &["lexicon-us-en.txt", "lexicon-zh.txt"],
    }
}

/// The voice as the settings carry it: a known name, else the default.
pub fn normalize_voice(name: &str) -> String {
    let name = name.trim().to_ascii_lowercase();
    if voice_id(&name).is_some() { name } else { DEFAULT_VOICE.to_string() }
}

/// The voices as `/tts voices` lists them: one line per language group.
pub fn voice_list_lines() -> Vec<String> {
    let groups = [
        ("American English", "a"), ("British English", "b"), ("Spanish", "e"), ("French", "f"),
        ("Hindi", "h"), ("Italian", "i"), ("Japanese", "j"), ("Brazilian Portuguese", "p"), ("Mandarin", "z"),
    ];
    groups.iter().map(|(label, prefix)| {
        let names: Vec<&str> = VOICES.iter().copied().filter(|v| v.starts_with(prefix)).collect();
        format!("  {label}: {}", names.join(", "))
    }).collect()
}

// ---------------------------------------------------------------------------------------
// Where things are

/// The install directory: `$CLAY_TTS_DIR`, else `~/.clay/tts` (`~/clay/tts` on Windows).
pub fn tts_dir() -> PathBuf {
    match std::env::var("CLAY_TTS_DIR") {
        Ok(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => crate::clay_config_path("tts"),
    }
}

pub fn runtime_dir() -> PathBuf { tts_dir().join("sherpa-onnx") }

/// The runtime's release asset for this platform and the C API library's file name
/// inside its `lib/` directory. None: no prebuilt runtime exists for this platform.
pub fn runtime_asset() -> Option<(String, &'static str)> {
    let (plat, lib) = runtime_platform(std::env::consts::OS, std::env::consts::ARCH)?;
    Some((format!("sherpa-onnx-v{RUNTIME_VERSION}-{plat}.tar.bz2"), lib))
}

/// The platform half of the asset name, from `std::env::consts::{OS, ARCH}` values.
pub(crate) fn runtime_platform(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    Some(match (os, arch) {
        ("linux", "x86_64") => ("linux-x64-shared", "libsherpa-onnx-c-api.so"),
        ("linux", "aarch64") => ("linux-aarch64-shared-cpu", "libsherpa-onnx-c-api.so"),
        ("macos", _) => ("osx-universal2-shared", "libsherpa-onnx-c-api.dylib"),
        // MT = static CRT, like Clay's own Windows build (`+crt-static`).
        ("windows", "x86_64") => ("win-x64-shared-MT-Release", "sherpa-onnx-c-api.dll"),
        ("windows", "aarch64") => ("win-arm64-shared-MT-Release", "sherpa-onnx-c-api.dll"),
        ("android", "aarch64") => ("android-aarch64-termux-shared", "libsherpa-onnx-c-api.so"),
        ("android", "arm") => ("android-armeabi-v7a-termux-shared", "libsherpa-onnx-c-api.so"),
        _ => return None,
    })
}

fn runtime_lib_name() -> &'static str {
    runtime_asset().map(|(_, lib)| lib).unwrap_or("libsherpa-onnx-c-api.so")
}

/// Can this build load a shared library at all? The static musl console cannot (dlopen
/// is a stub there), so it gets a clear message instead of a cryptic dlopen error.
pub fn build_can_load() -> bool {
    !cfg!(target_env = "musl")
}

/// The files an install consists of. Each is None when missing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Install {
    /// The C API library, when the runtime is installed at the pinned version.
    pub runtime: Option<PathBuf>,
    /// The runtime version found (whatever it is), for `/tts status`.
    pub runtime_version: Option<String>,
    /// The model directory, when every file the engine needs is there - the build this
    /// CPU prefers if it is installed, else the other one.
    pub model: Option<PathBuf>,
    /// Which build `model` is.
    pub model_kind: Option<ModelKind>,
}

impl Install {
    pub fn complete(&self) -> bool { self.runtime.is_some() && self.model.is_some() }
}

/// What is installed under `dir` (see [`tts_dir`]).
pub fn install_in(dir: &Path) -> Install {
    let runtime_dir = dir.join("sherpa-onnx");
    let version = std::fs::read_to_string(runtime_dir.join("VERSION")).ok().map(|s| s.trim().to_string());
    let lib = runtime_dir.join("lib").join(runtime_lib_name());
    let runtime = (version.as_deref() == Some(RUNTIME_VERSION) && lib.is_file()).then_some(lib);
    let preferred = preferred_model();
    let found = [preferred, preferred.other()].into_iter()
        .map(|k| (dir.join(k.dir_name()), k))
        .find(|(d, k)| model_files_present(d, *k));
    let (model, model_kind) = match found {
        Some((d, k)) => (Some(d), Some(k)),
        None => (None, None),
    };
    Install { runtime, runtime_version: version, model, model_kind }
}

pub fn install() -> Install { install_in(&tts_dir()) }

fn model_files_present(dir: &Path, kind: ModelKind) -> bool {
    [kind.file_name(), "voices.bin", "tokens.txt", "lexicon-us-en.txt"].iter().all(|f| dir.join(f).is_file())
        && dir.join("espeak-ng-data").is_dir()
}

/// Remove the model build this CPU does not prefer when both are installed. Only safe
/// before any engine has loaded in this process: espeak-ng is initialised once per
/// process with the first model's `espeak-ng-data`, so deleting that directory under it
/// leaves every later line a 0.2 s blip, reload or not. `/tts install`'s switch therefore
/// leaves the old build in place, and the next start removes it here.
pub fn remove_stale_build() {
    remove_stale_build_in(&tts_dir());
}

pub(crate) fn remove_stale_build_in(dir: &Path) {
    let wanted = preferred_model();
    let stale = dir.join(wanted.other().dir_name());
    if model_files_present(&dir.join(wanted.dir_name()), wanted) && stale.is_dir() {
        let _ = std::fs::remove_dir_all(&stale);
    }
}

/// What `/tts install` would do now: `Err` with why it cannot or need not run, `Ok` with
/// the line to show as it starts. An install that holds the build this CPU runs slower
/// is replaced, not refused.
pub fn install_check() -> Result<String, String> {
    if !build_can_load() {
        return Err("Kokoro: this build is a static binary and cannot load the runtime; use the GUI build (clay-linux-x86_64-gui) for Kokoro.".into());
    }
    if runtime_asset().is_none() {
        return Err(format!("Kokoro: no prebuilt runtime exists for {}/{}.", std::env::consts::OS, std::env::consts::ARCH));
    }
    let inst = install();
    let wanted = preferred_model();
    match inst.model_kind {
        Some(have) if inst.complete() && have == wanted => {
            Err("Kokoro: already installed (/tts status). Use /tts remove first to reinstall.".into())
        }
        Some(have) if inst.complete() => Ok(format!(
            "Kokoro: switching from the {} model to the {} one, which this CPU runs faster (about {} MB to download)...",
            have.label(), wanted.label(), wanted.download_mb()
        )),
        _ => Ok(format!(
            "Kokoro: installing the {} model into {} in the background (about {} MB to download)...",
            wanted.label(), tts_dir().display(), wanted.download_mb() + 31
        )),
    }
}

/// Bytes used by the install, for `/tts status`.
pub fn install_size(dir: &Path) -> u64 {
    fn walk(p: &Path, acc: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() { walk(&path, acc) } else if let Ok(md) = e.metadata() { *acc += md.len() }
            }
        }
    }
    let mut n = 0;
    walk(dir, &mut n);
    n
}

pub fn fmt_mb(bytes: u64) -> String {
    format!("{} MB", (bytes + 512 * 1024) / (1024 * 1024))
}

/// `/tts status`'s lines about the install.
pub fn status_lines() -> Vec<String> {
    let dir = tts_dir();
    let inst = install_in(&dir);
    let mut out = Vec::new();
    out.push(format!("Kokoro files: {}", dir.display()));
    match (&inst.runtime, &inst.runtime_version) {
        (Some(_), _) => out.push(format!("  runtime: sherpa-onnx {RUNTIME_VERSION} (installed)")),
        (None, Some(v)) => out.push(format!("  runtime: sherpa-onnx {v} found, but {RUNTIME_VERSION} is needed - /tts install replaces it")),
        (None, None) => out.push("  runtime: not installed".to_string()),
    }
    let wanted = preferred_model();
    out.push(match inst.model_kind {
        Some(k) => format!("  model: {} ({}, installed, {} voices)", k.dir_name(), k.label(), VOICES.len()),
        None => format!("  model: {} not installed", wanted.dir_name()),
    });
    if let Some(k) = inst.model_kind.filter(|k| *k != wanted) {
        out.push(format!(
            "  this CPU runs the {} model several times faster than the {} one it has (int8 needs VNNI); /tts install switches to it",
            wanted.label(), k.label()
        ));
    }
    if inst.complete() {
        out.push(format!("  on disk: {}", fmt_mb(install_size(&dir))));
    } else if !build_can_load() {
        out.push("  this build is a static binary and cannot load the runtime; use the GUI build for Kokoro".to_string());
    } else if runtime_asset().is_none() {
        out.push(format!("  no prebuilt runtime exists for {}/{}", std::env::consts::OS, std::env::consts::ARCH));
    } else {
        out.push(format!("  /tts install downloads both (about {} MB)", wanted.download_mb() + 31));
    }
    out
}

// ---------------------------------------------------------------------------------------
// Installing: two downloads from the sherpa-onnx GitHub releases, unpacked in place.

/// Download and unpack the runtime (if missing or at another version) and the model into
/// [`tts_dir`]. Progress lines go back as `AppEvent::TtsLine`; the last one has
/// `done: true`.
pub async fn install_files(event_tx: tokio::sync::mpsc::Sender<crate::AppEvent>, reply_to: Option<(u64, usize)>) {
    let say = |line: String, done: bool| {
        let tx = event_tx.clone();
        async move { let _ = tx.send(crate::AppEvent::TtsLine { line, done, reply_to }).await; }
    };
    match install_files_with(&say).await {
        Ok(summary) => say(summary, true).await,
        Err(e) => say(format!("Kokoro: install failed: {e}"), true).await,
    }
}

/// `clay --tts-install`: the same install, reporting on stdout, for a machine that runs
/// only a client (a remote GUI speaks with its own files). Returns the exit code.
pub async fn install_files_cli() -> i32 {
    let say = |line: String, _done: bool| async move { println!("{line}"); };
    match install_files_with(&say).await {
        Ok(summary) => { println!("{summary}"); 0 }
        Err(e) => { eprintln!("Kokoro: install failed: {e}"); 1 }
    }
}

pub(crate) async fn install_files_with<F, Fut>(say: &F) -> Result<String, String>
where
    F: Fn(String, bool) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    if !build_can_load() {
        return Err("this build is a static binary and cannot load the runtime; use the GUI build for Kokoro".into());
    }
    let (asset, _lib) = runtime_asset()
        .ok_or_else(|| format!("no prebuilt runtime for {}/{}", std::env::consts::OS, std::env::consts::ARCH))?;
    let dir = tts_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let client = reqwest::Client::builder()
        .user_agent("clay-mud-client")
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let before = install_in(&dir);
    if before.runtime.is_none() {
        let url = format!("{RUNTIME_URL_BASE}{RUNTIME_VERSION}/{asset}");
        say(format!("Kokoro: downloading the sherpa-onnx {RUNTIME_VERSION} runtime..."), false).await;
        let bytes = fetch(&client, &url).await?;
        say(format!("Kokoro: unpacking the runtime ({})...", fmt_mb(bytes.len() as u64)), false).await;
        let top = asset.trim_end_matches(".tar.bz2").to_string();
        let staging = dir.join(".staging-runtime");
        let _ = std::fs::remove_dir_all(&staging);
        // Only lib/: the tarball's bin/ is 50 MB of tools Clay never runs.
        extract_tar_bz2(&bytes, &staging, Some(&format!("{top}/lib/")))?;
        let runtime_dir = dir.join("sherpa-onnx");
        let _ = std::fs::remove_dir_all(&runtime_dir);
        std::fs::rename(staging.join(&top), &runtime_dir)
            .map_err(|e| format!("cannot move the runtime into place: {e}"))?;
        std::fs::write(runtime_dir.join("VERSION"), format!("{RUNTIME_VERSION}\n"))
            .map_err(|e| format!("cannot write VERSION: {e}"))?;
        let _ = std::fs::remove_dir_all(&staging);
    } else {
        say(format!("Kokoro: runtime sherpa-onnx {RUNTIME_VERSION} already installed."), false).await;
    }

    let wanted = preferred_model();
    if before.model_kind != Some(wanted) {
        if let Some(have) = before.model_kind {
            say(format!("Kokoro: this CPU runs the {} model faster than the {} one installed; replacing it.", wanted.label(), have.label()), false).await;
        }
        say(format!("Kokoro: downloading the {} model ({} MB)...", wanted.dir_name(), wanted.download_mb()), false).await;
        let bytes = fetch(&client, &wanted.url()).await?;
        say(format!("Kokoro: unpacking the model ({})...", fmt_mb(bytes.len() as u64)), false).await;
        let staging = dir.join(".staging-model");
        let _ = std::fs::remove_dir_all(&staging);
        extract_tar_bz2(&bytes, &staging, None)?;
        let model_dir = dir.join(wanted.dir_name());
        let _ = std::fs::remove_dir_all(&model_dir);
        std::fs::rename(staging.join(wanted.dir_name()), &model_dir)
            .map_err(|e| format!("cannot move the model into place: {e}"))?;
        let _ = std::fs::remove_dir_all(&staging);
        // The old build stays for now: a session loaded from it may be running in this
        // very process, and espeak-ng (initialised once per process, with that build's
        // `espeak-ng-data`) cannot follow a reload - every later line became a 0.2 s
        // blip. `remove_stale_build` clears it at the next start.
        if before.model_kind.is_some() {
            say("Kokoro: the old build's files are removed the next time Clay starts.".to_string(), false).await;
        }
    } else {
        say(format!("Kokoro: model {} ({}) already installed.", wanted.dir_name(), wanted.label()), false).await;
    }

    let after = install_in(&dir);
    if !after.complete() {
        return Err("the files are not where they should be after unpacking (see /tts status)".into());
    }
    Ok(format!("Kokoro: installed in {} ({} on disk). Set TTS to Kokoro, or type /tts test.",
        dir.display(), fmt_mb(install_size(&dir))))
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client.get(url).send().await.map_err(|e| format!("download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {} for {url}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("download incomplete: {e}"))?;
    if bytes.len() < 1024 * 1024 {
        return Err(format!("download too small ({} bytes) for {url}", bytes.len()));
    }
    Ok(bytes.to_vec())
}

/// Unpack a .tar.bz2 into `dest`, keeping only entries under `only_prefix` when given.
/// `unpack_in` refuses anything that would land outside `dest`.
fn extract_tar_bz2(bytes: &[u8], dest: &Path, only_prefix: Option<&str>) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let decoder = bzip2::read::MultiBzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(|e| format!("bad archive: {e}"))?;
    let mut n = 0usize;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("bad archive entry: {e}"))?;
        let path = entry.path().map_err(|e| format!("bad entry path: {e}"))?.to_string_lossy().to_string();
        if let Some(prefix) = only_prefix {
            if !path.starts_with(prefix) { continue; }
        }
        let unpacked = entry.unpack_in(dest).map_err(|e| format!("cannot unpack {path}: {e}"))?;
        if unpacked { n += 1; }
    }
    if n == 0 {
        return Err("the archive held nothing to unpack".into());
    }
    Ok(())
}

/// Delete the install.
pub fn remove_files() -> Result<String, String> {
    let dir = tts_dir();
    if !dir.exists() {
        return Ok("Kokoro: nothing installed.".into());
    }
    let size = install_size(&dir);
    std::fs::remove_dir_all(&dir).map_err(|e| format!("cannot remove {}: {e}", dir.display()))?;
    Ok(format!("Kokoro: removed {} ({} freed).", dir.display(), fmt_mb(size)))
}

// ---------------------------------------------------------------------------------------
// The sherpa-onnx C API, as of RUNTIME_VERSION's c-api.h. Field order and types must match
// the header exactly; the `ffi_struct_sizes_match_the_header` test pins the sizes.

#[repr(C)]
struct VitsCfg { model: *const c_char, lexicon: *const c_char, tokens: *const c_char, data_dir: *const c_char, noise_scale: f32, noise_scale_w: f32, length_scale: f32, dict_dir: *const c_char }
#[repr(C)]
struct MatchaCfg { acoustic_model: *const c_char, vocoder: *const c_char, lexicon: *const c_char, tokens: *const c_char, data_dir: *const c_char, noise_scale: f32, length_scale: f32, dict_dir: *const c_char }
#[repr(C)]
struct KokoroCfg { model: *const c_char, voices: *const c_char, tokens: *const c_char, data_dir: *const c_char, length_scale: f32, dict_dir: *const c_char, lexicon: *const c_char, lang: *const c_char }
#[repr(C)]
struct KittenCfg { model: *const c_char, voices: *const c_char, tokens: *const c_char, data_dir: *const c_char, length_scale: f32 }
#[repr(C)]
struct ZipvoiceCfg { tokens: *const c_char, encoder: *const c_char, decoder: *const c_char, vocoder: *const c_char, data_dir: *const c_char, lexicon: *const c_char, feat_scale: f32, t_shift: f32, target_rms: f32, guidance_scale: f32 }
#[repr(C)]
struct PocketCfg { lm_flow: *const c_char, lm_main: *const c_char, encoder: *const c_char, decoder: *const c_char, text_conditioner: *const c_char, vocab_json: *const c_char, token_scores_json: *const c_char, voice_embedding_cache_capacity: i32 }
#[repr(C)]
struct SupertonicCfg { duration_predictor: *const c_char, text_encoder: *const c_char, vector_estimator: *const c_char, vocoder: *const c_char, tts_json: *const c_char, unicode_indexer: *const c_char, voice_style: *const c_char }
#[repr(C)]
struct ModelCfg { vits: VitsCfg, num_threads: i32, debug: i32, provider: *const c_char, matcha: MatchaCfg, kokoro: KokoroCfg, kitten: KittenCfg, zipvoice: ZipvoiceCfg, pocket: PocketCfg, supertonic: SupertonicCfg }
#[repr(C)]
struct TtsCfg { model: ModelCfg, rule_fsts: *const c_char, max_num_sentences: i32, rule_fars: *const c_char, silence_scale: f32 }
#[repr(C)]
struct GeneratedAudio { samples: *const f32, n: i32, sample_rate: i32 }
#[repr(C)]
struct GenerationCfg { silence_scale: f32, speed: f32, sid: i32, reference_audio: *const f32, reference_audio_len: i32, reference_sample_rate: i32, reference_text: *const c_char, num_steps: i32, extra: *const c_char }

type ProgressCb = unsafe extern "C" fn(*const f32, i32, f32, *mut c_void) -> i32;

/// The runtime's entry points, loaded by name. The libraries stay loaded for the life of
/// the process once used (the symbols are raw, so they must).
struct Api {
    _runtime: libloading::Library,
    _onnxruntime: Option<libloading::Library>,
    create: unsafe extern "C" fn(*const TtsCfg) -> *const c_void,
    destroy: unsafe extern "C" fn(*const c_void),
    sample_rate: unsafe extern "C" fn(*const c_void) -> i32,
    num_speakers: unsafe extern "C" fn(*const c_void) -> i32,
    generate: unsafe extern "C" fn(*const c_void, *const c_char, *const GenerationCfg, Option<ProgressCb>, *mut c_void) -> *const GeneratedAudio,
    destroy_audio: unsafe extern "C" fn(*const GeneratedAudio),
}

impl Api {
    fn load(lib_path: &Path) -> Result<Api, String> {
        // The C API library needs ONNX Runtime from the same directory. Linux's copy
        // carries an $ORIGIN rpath and Windows searches the DLL's own directory, but
        // macOS's install names may not resolve, so load it first, globally, everywhere
        // it exists.
        let dir = lib_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let onnxruntime = ["libonnxruntime.so", "libonnxruntime.dylib", "onnxruntime.dll"]
            .iter().map(|n| dir.join(n)).find(|p| p.is_file());
        let onnxruntime = match onnxruntime {
            Some(p) => Some(load_library(&p, true)?),
            None => None,
        };
        let runtime = load_library(lib_path, false)?;
        // Safety: these are the signatures from c-api.h at RUNTIME_VERSION.
        unsafe {
            let sym = |name: &[u8]| -> Result<*const c_void, String> {
                let s: libloading::Symbol<*const c_void> = runtime.get(name)
                    .map_err(|e| format!("{} lacks {}: {e}", lib_path.display(), String::from_utf8_lossy(name)))?;
                Ok(*s)
            };
            let create = std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const TtsCfg) -> *const c_void>(sym(b"SherpaOnnxCreateOfflineTts\0")?);
            let destroy = std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const c_void)>(sym(b"SherpaOnnxDestroyOfflineTts\0")?);
            let sample_rate = std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const c_void) -> i32>(sym(b"SherpaOnnxOfflineTtsSampleRate\0")?);
            let num_speakers = std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const c_void) -> i32>(sym(b"SherpaOnnxOfflineTtsNumSpeakers\0")?);
            let generate = std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const c_void, *const c_char, *const GenerationCfg, Option<ProgressCb>, *mut c_void) -> *const GeneratedAudio>(sym(b"SherpaOnnxOfflineTtsGenerateWithConfig\0")?);
            let destroy_audio = std::mem::transmute::<*const c_void, unsafe extern "C" fn(*const GeneratedAudio)>(sym(b"SherpaOnnxDestroyOfflineTtsGeneratedAudio\0")?);
            Ok(Api { _runtime: runtime, _onnxruntime: onnxruntime, create, destroy, sample_rate, num_speakers, generate, destroy_audio })
        }
    }
}

fn load_library(path: &Path, global: bool) -> Result<libloading::Library, String> {
    // Safety: loading a library runs its initializers; these are the sherpa-onnx
    // libraries the user installed on purpose.
    unsafe {
        #[cfg(unix)]
        {
            use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_LOCAL, RTLD_NOW};
            let flags = RTLD_NOW | if global { RTLD_GLOBAL } else { RTLD_LOCAL };
            Library::open(Some(path), flags).map(Into::into)
        }
        #[cfg(windows)]
        {
            use libloading::os::windows::{Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR};
            let _ = global;
            Library::load_with_flags(path, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS | LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR).map(Into::into)
        }
    }
    .map_err(|e| format!("cannot load {}: {e}", path.display()))
}

/// One loaded model.
struct Model {
    api: Arc<Api>,
    tts: *const c_void,
    sample_rate: i32,
    num_speakers: i32,
    lang: String,
    /// What it was loaded with (`voice_lexicons`): a voice wanting others reloads it.
    lexicons: &'static [&'static str],
    /// Where it was loaded from. `/tts install`'s switch between model builds deletes the
    /// old directory under a loaded session, whose espeak-ng phonemizer then opens files
    /// that are gone and yields nothing - a 0.2 s blip for any line, however long - so a
    /// session is reloaded as soon as the install's model directory is another one.
    dir: PathBuf,
}

impl Drop for Model {
    fn drop(&mut self) {
        // Safety: created by `api.create` and never freed elsewhere.
        unsafe { (self.api.destroy)(self.tts) }
    }
}

fn cstr(p: &Path) -> CString { CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default() }

/// The threads the model runs on: every CPU the machine has, up to eight. Synthesis
/// has to outrun playback, and the first sentence is made in full before anything is
/// heard; with half the cores (the old rule) that took longer than the sentence itself on
/// a small machine. A line is a short burst, so the spike is brief.
fn num_threads() -> i32 {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    cores.clamp(2, 8) as i32
}

impl Model {
    fn load(api: Arc<Api>, model_dir: &Path, lang: &str, lexicons: &'static [&'static str]) -> Result<Model, String> {
        let model = cstr(&model_file_in(model_dir));
        let voices = cstr(&model_dir.join("voices.bin"));
        let tokens = cstr(&model_dir.join("tokens.txt"));
        let data_dir = cstr(&model_dir.join("espeak-ng-data"));
        let lexicon = {
            let files: Vec<String> = lexicons.iter()
                .map(|f| model_dir.join(f)).filter(|p| p.is_file())
                .map(|p| p.to_string_lossy().to_string()).collect();
            CString::new(files.join(",")).unwrap_or_default()
        };
        let lang_c = CString::new(lang).unwrap_or_default();
        let provider = CString::new("cpu").unwrap_or_default();
        // Safety: an all-zero struct is sherpa-onnx's documented starting point (the C
        // examples memset it); null strings read as empty.
        let mut cfg: TtsCfg = unsafe { std::mem::zeroed() };
        cfg.model.num_threads = num_threads();
        cfg.model.provider = provider.as_ptr();
        cfg.model.kokoro = KokoroCfg {
            model: model.as_ptr(), voices: voices.as_ptr(), tokens: tokens.as_ptr(), data_dir: data_dir.as_ptr(),
            length_scale: 1.0, dict_dir: std::ptr::null(), lexicon: lexicon.as_ptr(), lang: lang_c.as_ptr(),
        };
        cfg.model.vits.noise_scale = 0.667;
        cfg.model.vits.noise_scale_w = 0.8;
        cfg.model.vits.length_scale = 1.0;
        cfg.model.matcha.noise_scale = 1.0;
        cfg.model.matcha.length_scale = 1.0;
        cfg.model.kitten.length_scale = 1.0;
        // One sentence per callback: playback starts after the first.
        cfg.max_num_sentences = 1;
        cfg.silence_scale = 0.2;
        // Safety: the config and every string it points at outlive the call. Whatever the
        // library says about the files goes to debug.log, not the console.
        let tts = {
            let _stderr = crate::platform::StderrToLog::new();
            unsafe { (api.create)(&cfg) }
        };
        if tts.is_null() {
            return Err(format!("sherpa-onnx could not load the model in {} (its report is in debug.log)", model_dir.display()));
        }
        // Safety: a live handle from `create`.
        let (sample_rate, num_speakers) = unsafe { ((api.sample_rate)(tts), (api.num_speakers)(tts)) };
        Ok(Model { api, tts, sample_rate, num_speakers, lang: lang.to_string(), lexicons, dir: model_dir.to_path_buf() })
    }
}

// ---------------------------------------------------------------------------------------
// The engine: one worker thread, latest-wins.

enum Job {
    Speak { text: String, voice: String, gen: u64 },
    /// Load the model for `voice` now, so the first line does not wait for it.
    Warm { voice: String },
    /// Unload the model and close the audio stream: Kokoro is no longer the engine.
    Release,
    Quit,
}

/// How long the audio stream stays open after the last line has played. An open stream
/// is never free: its thread wakes every period to hand the sound server silence (cpal's
/// 25 ms, so forty times a second), and the server mixes it the whole time - a Clay
/// that had spoken once sat at several percent of a CPU for the rest of the day. Lines
/// come in bursts, so the stream is kept through a conversation and let go after a
/// pause; the next line reopens it (some tens of milliseconds, well under the synthesis
/// of its first sentence, and counted in debug.log's "first audio after").
const OUTPUT_LINGER: std::time::Duration = std::time::Duration::from_secs(30);

/// A Kokoro voice ready to speak. Create one per process; `speak` returns at once.
pub struct Engine {
    tx: mpsc::Sender<Job>,
    latest: Arc<AtomicU64>,
    state: Arc<Shared>,
}

/// What the worker and the callers share.
struct Shared {
    /// True once the model loaded, for `/tts status`.
    loaded: AtomicBool,
    /// The last failure (load or playback), for `/tts status`.
    last_error: Mutex<Option<String>>,
    /// What is playing now, so `stop` can cut it.
    player: Mutex<Option<PlayHandle>>,
    /// How the audio leaves (`Output::describe`), once the worker has opened it - for
    /// `/tts status`, since "quiet" means something different for each path.
    output: Mutex<Option<String>>,
    /// How many times the worker unloaded the model (`Job::Release`); for tests.
    releases: AtomicU64,
}

impl Default for Engine {
    fn default() -> Self { Engine::new() }
}

impl Engine {
    pub fn new() -> Engine {
        let (tx, rx) = mpsc::channel::<Job>();
        let latest = Arc::new(AtomicU64::new(0));
        let state = Arc::new(Shared {
            loaded: AtomicBool::new(false), last_error: Mutex::new(None), player: Mutex::new(None), output: Mutex::new(None), releases: AtomicU64::new(0),
        });
        let (latest2, state2) = (latest.clone(), state.clone());
        std::thread::Builder::new().name("clay-kokoro".into()).spawn(move || worker(rx, latest2, state2)).ok();
        Engine { tx, latest, state }
    }

    /// Say this, in this voice, dropping whatever is still being said.
    pub fn speak(&self, text: &str, voice: &str) {
        let gen = self.latest.fetch_add(1, Ordering::SeqCst) + 1;
        self.stop_playback();
        let _ = self.tx.send(Job::Speak { text: text.to_string(), voice: normalize_voice(voice), gen });
    }

    /// Load the model for `voice` in the background (about a second), so the first line
    /// spoken is as quick as any other. Nothing playing is disturbed; a model already
    /// loaded for that voice's language and lexicon is kept.
    pub fn warm(&self, voice: &str) {
        let _ = self.tx.send(Job::Warm { voice: normalize_voice(voice) });
    }

    /// Kokoro is no longer the engine (or its files are gone): stop, unload the model -
    /// a few hundred megabytes and the runtime's threads - and close the audio stream.
    /// The library stays loaded and the worker stays, idle, so a later `warm` or `speak`
    /// starts over from the model load. Unloading an engine with nothing loaded is free.
    pub fn release(&self) {
        self.stop();
        let _ = self.tx.send(Job::Release);
    }

    /// Stop speaking and forget anything queued.
    pub fn stop(&self) {
        self.latest.fetch_add(1, Ordering::SeqCst);
        self.stop_playback();
    }

    fn stop_playback(&self) {
        if let Ok(mut p) = self.state.player.lock() {
            if let Some(h) = p.take() { h.stop(); }
        }
    }

    pub fn loaded(&self) -> bool { self.state.loaded.load(Ordering::SeqCst) }
    pub fn last_error(&self) -> Option<String> { self.state.last_error.lock().ok().and_then(|e| e.clone()) }
    /// How the audio leaves this process (`/tts status`), once an utterance opened it.
    pub fn output_kind(&self) -> Option<String> { self.state.output.lock().ok().and_then(|o| o.clone()) }
    /// How many times `release` has unloaded the model so far (tests).
    #[cfg(test)]
    pub fn releases(&self) -> u64 { self.state.releases.load(Ordering::SeqCst) }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
        let _ = self.tx.send(Job::Quit);
    }
}

fn worker(rx: mpsc::Receiver<Job>, latest: Arc<AtomicU64>, state: Arc<Shared>) {
    let mut api: Option<Arc<Api>> = None;
    let mut model: Option<Model> = None;
    // Opened by the first line, closed again after OUTPUT_LINGER of silence.
    let mut output: Option<Output> = None;
    let mut last_line = std::time::Instant::now();
    let fail = |state: &Shared, msg: String| {
        crate::debug_log(true, &format!("Kokoro: {msg}"));
        if let Ok(mut e) = state.last_error.lock() { *e = Some(msg); }
    };
    loop {
        let job = if output.is_some() {
            // Wake when the linger is up (or the sink finishes playing after it).
            let wait = OUTPUT_LINGER.saturating_sub(last_line.elapsed()).max(std::time::Duration::from_millis(500));
            match rx.recv_timeout(wait) {
                Ok(job) => job,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if output_may_close(last_line.elapsed(), still_playing(&state)) {
                        close_output(&mut output, &state);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(job) => job,
                Err(_) => break,
            }
        };
        let (text, voice, gen) = match job {
            Job::Speak { text, voice, gen } => (Some(text), voice, Some(gen)),
            Job::Warm { voice } => (None, voice, None),
            Job::Release => {
                // The model's drop destroys the runtime session (and its threads).
                model = None;
                state.loaded.store(false, Ordering::SeqCst);
                close_output(&mut output, &state);
                state.releases.fetch_add(1, Ordering::SeqCst);
                continue;
            }
            Job::Quit => break,
        };
        if gen.is_some_and(|g| latest.load(Ordering::SeqCst) != g) { continue; }
        let inst = install();
        let (Some(lib), Some(model_dir)) = (inst.runtime, inst.model) else {
            fail(&state, "not installed (type /tts install)".into());
            continue;
        };
        if api.is_none() {
            match Api::load(&lib) {
                Ok(a) => api = Some(Arc::new(a)),
                Err(e) => { fail(&state, e); continue; }
            }
        }
        let lang = voice_lang(&voice);
        let lexicons = voice_lexicons(&voice);
        if model.as_ref().is_some_and(|m| m.lang != lang || m.lexicons != lexicons || m.dir != model_dir) {
            model = None;
        }
        if model.is_none() {
            let started = std::time::Instant::now();
            match Model::load(api.clone().expect("loaded above"), &model_dir, lang, lexicons) {
                Ok(m) => {
                    crate::debug_log(crate::is_debug_enabled(), &format!(
                        "Kokoro: model loaded in {:.2}s ({} Hz, {} voices, lang {lang}, {}, {} threads)",
                        started.elapsed().as_secs_f32(), m.sample_rate, m.num_speakers,
                        model_file_in(&model_dir).file_name().and_then(|f| f.to_str()).unwrap_or("?"), num_threads()
                    ));
                    state.loaded.store(true, Ordering::SeqCst);
                    model = Some(m);
                }
                Err(e) => { fail(&state, e); continue; }
            }
        }
        let m = model.as_ref().expect("loaded above");
        // A warm-up wanted only the model loaded: done.
        let (Some(text), Some(gen)) = (text, gen) else { continue };
        if latest.load(Ordering::SeqCst) != gen { continue; }
        let sid = voice_id(&voice).unwrap_or(0).min(m.num_speakers.max(1) - 1);
        let Ok(text_c) = CString::new(text.replace('\0', " ")) else { continue };
        // The stream is opened here, not at start-up or on a warm-up: it costs CPU for
        // as long as it is open (OUTPUT_LINGER), and this is the first moment it is
        // needed. Its open is part of the first-audio delay the log reports.
        let started = std::time::Instant::now();
        let out = output.get_or_insert_with(|| {
            let o = Output::open();
            if let Ok(mut d) = state.output.lock() { *d = Some(o.describe()); }
            o
        });
        let mut sink = out.start(m.sample_rate as u32, &state);
        let mut cb = CallbackState {
            gen, latest: latest.clone(), sink: &mut sink, rate: m.sample_rate as u32,
            started, words: text.split_whitespace().count(), first: true,
        };
        let gcfg = GenerationCfg {
            silence_scale: 0.2, speed: 1.0, sid, reference_audio: std::ptr::null(), reference_audio_len: 0,
            reference_sample_rate: 0, reference_text: std::ptr::null(), num_steps: 5, extra: std::ptr::null(),
        };
        // Safety: the model handle is live, the text and config outlive the call, and the
        // callback only touches `cb`, which outlives it too. The library's stderr (an
        // unknown word, say) goes to debug.log meanwhile.
        let audio = {
            let _stderr = crate::platform::StderrToLog::new();
            unsafe { (m.api.generate)(m.tts, text_c.as_ptr(), &gcfg, Some(on_chunk), &mut cb as *mut CallbackState as *mut c_void) }
        };
        if !audio.is_null() {
            // Safety: returned by `generate`, freed once.
            unsafe { (m.api.destroy_audio)(audio) };
        }
        if latest.load(Ordering::SeqCst) == gen {
            sink.finish(&state);
        }
        last_line = std::time::Instant::now();
    }
}

/// Whether the audio stream, last used `idle` ago, may be closed now: after the linger,
/// and never under a line still playing out of the sink.
fn output_may_close(idle: std::time::Duration, playing: bool) -> bool {
    idle >= OUTPUT_LINGER && !playing
}

/// Is the last line still coming out of the speakers?
fn still_playing(state: &Shared) -> bool {
    state.player.lock().ok().is_some_and(|mut p| p.as_mut().is_some_and(|h| h.playing()))
}

/// Close the stream (and drop the finished sink): nothing plays until the next line.
fn close_output(output: &mut Option<Output>, state: &Shared) {
    if output.take().is_some() {
        if let Ok(mut p) = state.player.lock() { *p = None; }
    }
}

struct CallbackState<'a> {
    gen: u64,
    latest: Arc<AtomicU64>,
    sink: &'a mut Sink,
    rate: u32,
    /// When synthesis began and how long the line is, for the debug.log timing of the
    /// first audio (the delay a user hears, once the model is loaded).
    started: std::time::Instant,
    words: usize,
    first: bool,
}

/// sherpa-onnx's per-sentence callback: queue the samples; 0 stops the generation.
unsafe extern "C" fn on_chunk(samples: *const f32, n: i32, _progress: f32, arg: *mut c_void) -> i32 {
    // Safety: `arg` is the `CallbackState` passed to `generate`, alive for the whole call.
    let cb = unsafe { &mut *(arg as *mut CallbackState) };
    if cb.latest.load(Ordering::SeqCst) != cb.gen {
        return 0;
    }
    if std::mem::take(&mut cb.first) {
        crate::debug_log(crate::is_debug_enabled(), &format!(
            "Kokoro: first audio after {:.2}s ({} words; the first sentence is {:.2}s long)",
            cb.started.elapsed().as_secs_f32(), cb.words, n.max(0) as f32 / cb.rate.max(1) as f32
        ));
    }
    if !samples.is_null() && n > 0 {
        // Safety: sherpa-onnx hands `n` valid floats.
        let chunk = unsafe { std::slice::from_raw_parts(samples, n as usize) };
        cb.sink.push(chunk, cb.rate);
    }
    1
}

// ---------------------------------------------------------------------------------------
// Output: rodio when built with native-audio (streamed, sentence by sentence); otherwise
// a WAV file and an external player once the line is done. `CLAY_TTS_WAV_OUT=<file>`
// writes the audio there instead of playing it (tests, headless machines).

enum Output {
    #[cfg(feature = "native-audio")]
    Rodio { _stream: rodio::OutputStream, handle: rodio::OutputStreamHandle, device: String },
    File,
}

impl Output {
    fn open() -> Output {
        #[cfg(feature = "native-audio")]
        {
            if std::env::var("CLAY_TTS_WAV_OUT").map(|v| !v.is_empty()).unwrap_or(false) {
                return Output::File;
            }
            // The same device the MUD's sounds use: on Linux the sound server's, when
            // there is one, rather than whatever ALSA's `default` is (see audio.rs).
            if let Some((stream, handle, device)) = crate::audio::open_output_stream() {
                return Output::Rodio { _stream: stream, handle, device };
            }
        }
        Output::File
    }

    /// Where the audio goes, in words (`/tts status`).
    fn describe(&self) -> String {
        #[cfg(feature = "native-audio")]
        if let Output::Rodio { device, .. } = self {
            return format!("this program's own audio stream (rodio, the {device} device)");
        }
        match std::env::var("CLAY_TTS_WAV_OUT") {
            Ok(path) if !path.is_empty() => format!("the file {path} (CLAY_TTS_WAV_OUT), not the speakers"),
            _ => "a WAV file per line, played by an external player (afplay, mpv, ffplay, play, aplay or paplay)".to_string(),
        }
    }

    fn start(&mut self, rate: u32, state: &Shared) -> Sink {
        #[cfg(feature = "native-audio")]
        if let Output::Rodio { handle, .. } = self {
            if let Ok(sink) = rodio::Sink::try_new(handle) {
                let sink = Arc::new(sink);
                if let Ok(mut p) = state.player.lock() { *p = Some(PlayHandle::Rodio(sink.clone())); }
                return Sink::Rodio(sink);
            }
        }
        let _ = (rate, state);
        Sink::Buffer(Vec::new())
    }
}

enum Sink {
    #[cfg(feature = "native-audio")]
    Rodio(Arc<rodio::Sink>),
    Buffer(Vec<f32>),
}

/// Kokoro's output is quiet: its waveforms peak around 0.25-0.4 of full scale (-8 to
/// -12 dBFS) where espeak and Piper sit near 0.8-0.9, and its loudness is about half
/// theirs - "I turned the volume up and can still barely hear it" next to the Local
/// engine. Every sentence is lifted by this much, limited so its peak stays under
/// `PEAK_CEILING` instead of clipping. One gain per sentence, so no pumping envelope.
const OUTPUT_GAIN: f32 = 2.5;
const PEAK_CEILING: f32 = 0.95;

/// `chunk` lifted by `OUTPUT_GAIN`, or by as much as keeps its peak under the ceiling.
fn amplified(chunk: &[f32]) -> Vec<f32> {
    let peak = chunk.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let gain = if peak > 0.0 { OUTPUT_GAIN.min(PEAK_CEILING / peak) } else { 1.0 };
    chunk.iter().map(|s| (s * gain).clamp(-1.0, 1.0)).collect()
}

impl Sink {
    fn push(&mut self, chunk: &[f32], rate: u32) {
        let chunk = amplified(chunk);
        match self {
            #[cfg(feature = "native-audio")]
            Sink::Rodio(sink) => sink.append(rodio::buffer::SamplesBuffer::new(1, rate, chunk)),
            Sink::Buffer(buf) => { let _ = rate; buf.extend_from_slice(&chunk) }
        }
    }

    /// The line is fully synthesized: a streamed sink just plays on; a buffered one is
    /// written out and played (or kept, under CLAY_TTS_WAV_OUT).
    #[cfg(feature = "native-audio")]
    fn into_buffer(self) -> Option<Vec<f32>> {
        match self {
            Sink::Rodio(_) => None,
            Sink::Buffer(b) => Some(b),
        }
    }

    #[cfg(not(feature = "native-audio"))]
    fn into_buffer(self) -> Option<Vec<f32>> {
        let Sink::Buffer(b) = self;
        Some(b)
    }

    fn finish(self, state: &Shared) {
        let Some(samples) = self.into_buffer() else { return };
        if samples.is_empty() { return; }
        let rate = 24000; // Kokoro's rate; the buffer path only exists for it.
        let out = match std::env::var("CLAY_TTS_WAV_OUT") {
            Ok(p) if !p.is_empty() => PathBuf::from(p),
            _ => std::env::temp_dir().join(format!("clay_kokoro_{}.wav", std::process::id())),
        };
        if let Err(e) = write_wav(&out, &samples, rate) {
            if let Ok(mut err) = state.last_error.lock() { *err = Some(format!("cannot write {}: {e}", out.display())); }
            return;
        }
        if std::env::var("CLAY_TTS_WAV_OUT").map(|v| !v.is_empty()).unwrap_or(false) {
            return;
        }
        let child = spawn_player(&out);
        if let Ok(mut p) = state.player.lock() {
            if let Some(c) = child { *p = Some(PlayHandle::Child(c)); }
        }
        // The file is removed by the next utterance's write or at exit; a player still
        // reading it must not lose it under its feet.
    }
}

/// Something that can be stopped from another thread.
enum PlayHandle {
    #[cfg(feature = "native-audio")]
    Rodio(Arc<rodio::Sink>),
    Child(std::process::Child),
}

impl PlayHandle {
    fn stop(mut self) {
        match &mut self {
            #[cfg(feature = "native-audio")]
            PlayHandle::Rodio(sink) => sink.stop(),
            PlayHandle::Child(child) => { let _ = child.kill(); let _ = child.wait(); }
        }
    }

    /// Still sounding: the sink has samples left, or the player has not exited.
    fn playing(&mut self) -> bool {
        match self {
            #[cfg(feature = "native-audio")]
            PlayHandle::Rodio(sink) => !sink.empty(),
            PlayHandle::Child(child) => matches!(child.try_wait(), Ok(None)),
        }
    }
}

/// The external players the Edge path also tries, in order; the first that starts wins.
fn spawn_player(path: &Path) -> Option<std::process::Child> {
    use std::process::{Command, Stdio};
    let p = path.to_string_lossy().to_string();
    let players: &[(&str, &[&str])] = &[
        #[cfg(target_os = "macos")]
        ("afplay", &[&p]),
        #[cfg(target_os = "windows")]
        ("powershell", &["-NoProfile", "-Command", &format!("(New-Object Media.SoundPlayer '{}').PlaySync()", p.replace('\'', "''"))]),
        ("mpv", &["--no-video", "--really-quiet", &p]),
        ("ffplay", &["-nodisp", "-autoexit", "-loglevel", "quiet", &p]),
        ("play", &["-q", &p]),
        ("aplay", &["-q", &p]),
        ("paplay", &[&p]),
    ];
    for (player, args) in players {
        if let Ok(child) = Command::new(player).args(*args).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            return Some(child);
        }
    }
    None
}

/// 16-bit mono PCM WAV.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> std::io::Result<()> {
    use std::io::Write;
    let data_len = (samples.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&1u16.to_le_bytes())?; // mono
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * 2).to_le_bytes())?;
    f.write_all(&2u16.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        f.write_all(&v.to_le_bytes())?;
    }
    f.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_map_to_the_models_speaker_ids() {
        assert_eq!(voice_id("af_heart"), Some(3));
        assert_eq!(voice_id("am_michael"), Some(16));
        assert_eq!(voice_id("  AF_HEART "), Some(3), "case and spaces are forgiven");
        assert_eq!(voice_id("em_santa"), Some(53));
        assert_eq!(voice_id("nobody"), None);
        assert_eq!(normalize_voice("bm_george"), "bm_george");
        assert_eq!(normalize_voice("nobody"), DEFAULT_VOICE);
        assert_eq!(VOICES.len(), 54);
        let mut sorted = VOICES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), VOICES.len(), "no voice listed twice");
    }

    #[test]
    fn a_voices_first_letter_is_its_language() {
        assert_eq!(voice_lang("af_heart"), "en");
        assert_eq!(voice_lang("bm_george"), "en");
        assert_eq!(voice_lang("ef_dora"), "es");
        assert_eq!(voice_lang("pm_santa"), "pt-br");
        assert_eq!(voice_lang("zf_xiaobei"), "zh");
        assert_eq!(voice_lang(""), "en");
        assert_eq!(voice_list_lines().len(), 9);
        assert!(voice_list_lines()[0].contains("af_heart"));
    }

    #[test]
    fn the_runtime_asset_follows_the_platform() {
        assert_eq!(runtime_platform("linux", "x86_64"), Some(("linux-x64-shared", "libsherpa-onnx-c-api.so")));
        assert_eq!(runtime_platform("macos", "aarch64").map(|p| p.0), Some("osx-universal2-shared"));
        assert_eq!(runtime_platform("windows", "x86_64").map(|p| p.1), Some("sherpa-onnx-c-api.dll"));
        assert_eq!(runtime_platform("android", "aarch64").map(|p| p.0), Some("android-aarch64-termux-shared"));
        assert_eq!(runtime_platform("freebsd", "x86_64"), None);
        if let Some((asset, _)) = runtime_asset() {
            assert!(asset.starts_with(&format!("sherpa-onnx-v{RUNTIME_VERSION}-")) && asset.ends_with(".tar.bz2"));
        }
    }

    /// The C structs mirror c-api.h at RUNTIME_VERSION; these are their sizes on every
    /// A warm-up goes through the load path without speaking: here, with no files, it
    /// records why it could not load, and it never disturbs what is playing (the
    /// generation counter that `speak` and `stop` bump stays put).
    #[test]
    fn warm_loads_without_speaking_and_without_cancelling() {
        let engine = Engine::new();
        assert!(!install().complete(), "the test home has no Kokoro files");
        let gen_before = engine.latest.load(Ordering::SeqCst);
        engine.warm("af_heart");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while engine.last_error().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(engine.last_error().is_some_and(|e| e.contains("not installed")), "the warm-up tried to load: {:?}", engine.last_error());
        assert!(!engine.loaded());
        assert_eq!(engine.latest.load(Ordering::SeqCst), gen_before, "a warm-up cancels nothing");
    }

    /// Kokoro deselected: the worker unloads whatever it has and goes idle, and a later
    /// warm-up starts over (here, with no files, it records the same reason again).
    #[test]
    fn release_unloads_and_a_later_warm_starts_over() {
        let engine = Engine::new();
        assert!(!install().complete(), "the test home has no Kokoro files");
        let wait_for = |done: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !done() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(done(), "the worker did not get there in time");
        };
        engine.release();
        wait_for(&|| engine.releases() == 1);
        assert!(!engine.loaded());
        assert!(engine.last_error().is_none(), "unloading nothing is not an error");
        engine.release();
        wait_for(&|| engine.releases() == 2);
        engine.warm("af_heart");
        wait_for(&|| engine.last_error().is_some());
        assert!(engine.last_error().is_some_and(|e| e.contains("not installed")));
    }

    /// The audio stream outlives a line by the linger, and never closes under one that
    /// is still playing, however long ago it was synthesized.
    #[test]
    fn the_stream_closes_after_the_linger_and_not_while_playing() {
        let d = std::time::Duration::from_secs;
        assert!(!output_may_close(d(0), false));
        assert!(!output_may_close(OUTPUT_LINGER - d(1), false));
        assert!(output_may_close(OUTPUT_LINGER, false));
        assert!(output_may_close(d(600), false));
        assert!(!output_may_close(d(600), true), "a long line is still coming out of the speakers");
    }

    /// The model gets every CPU, within bounds that keep a tiny box and a big one sane.
    #[test]
    fn thread_count_is_within_bounds() {
        let n = num_threads();
        assert!((2..=8).contains(&n), "{n}");
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) as i32;
        assert_eq!(n, cpus.clamp(2, 8));
    }

    /// One English lexicon per voice (GB for the British voices), never both: both at
    /// once made sherpa-onnx print "Duplicated word" for every GB entry onto the console.
    #[test]
    fn one_english_lexicon_per_voice() {
        assert_eq!(voice_lexicons("af_heart"), &["lexicon-us-en.txt", "lexicon-zh.txt"]);
        assert_eq!(voice_lexicons("bm_george"), &["lexicon-gb-en.txt", "lexicon-zh.txt"]);
        assert_eq!(voice_lexicons("ef_dora"), &["lexicon-us-en.txt", "lexicon-zh.txt"], "a non-English voice still reads English words");
        assert_eq!(voice_lexicons("zf_xiaobei"), &["lexicon-us-en.txt", "lexicon-zh.txt"]);
        for lex in VOICES.iter().map(|v| voice_lexicons(v)) {
            assert!(!(lex.contains(&"lexicon-us-en.txt") && lex.contains(&"lexicon-gb-en.txt")), "never both English lexicons");
        }
    }

    /// Whatever a sherpa-onnx call writes to stderr goes to debug.log while the guard is
    /// held, and stderr is itself again once it is dropped. Written at the fd, as the
    /// library does (the test harness captures Rust's own eprintln elsewhere).
    #[cfg(unix)]
    #[test]
    fn library_stderr_goes_to_the_debug_log_while_guarded() {
        let log = crate::clay_config_path("debug.log");
        let before = std::fs::read_to_string(&log).unwrap_or_default();
        let line = b"kokoro-stderr-guard-test\n";
        {
            let _guard = crate::platform::StderrToLog::new();
            // Safety: a plain write of a static buffer to fd 2.
            let n = unsafe { libc::write(2, line.as_ptr() as *const libc::c_void, line.len()) };
            assert_eq!(n as usize, line.len());
        }
        let after = std::fs::read_to_string(&log).unwrap_or_default();
        assert_eq!(after.matches("kokoro-stderr-guard-test").count(), before.matches("kokoro-stderr-guard-test").count() + 1, "the line went to the log");
        // Back to the real stderr: fd 2 is no longer the log file.
        let fstat = |fd: i32| {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            unsafe { libc::fstat(fd, &mut st) };
            (st.st_dev, st.st_ino)
        };
        let log_file = std::fs::File::open(&log).unwrap();
        use std::os::unix::io::AsRawFd;
        assert_ne!(fstat(2), fstat(log_file.as_raw_fd()), "stderr was put back");
    }

    /// A quiet sentence is lifted by the full gain; a loud one only as far as the peak
    /// ceiling allows; silence is left alone. Nothing ever clips.
    #[test]
    fn output_gain_lifts_quiet_audio_and_limits_loud() {
        let quiet: Vec<f32> = vec![0.1, -0.2, 0.3, -0.1];
        let lifted = amplified(&quiet);
        assert!((lifted[2] - 0.75).abs() < 1e-6, "0.3 * 2.5: {lifted:?}");
        assert!((lifted[1] + 0.5).abs() < 1e-6);
        let loud: Vec<f32> = vec![0.5, -0.8, 0.2];
        let limited = amplified(&loud);
        let peak = limited.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - PEAK_CEILING).abs() < 1e-6, "limited to the ceiling, not 2.5x: {limited:?}");
        assert!((limited[0] / limited[1] - loud[0] / loud[1]).abs() < 1e-6, "one gain for the whole chunk");
        assert_eq!(amplified(&[0.0, 0.0]), vec![0.0, 0.0]);
        assert!(amplified(&[]).is_empty());
    }

    /// 64-bit target (pointers 8, int32/float 4, natural alignment).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn ffi_struct_sizes_match_the_header() {
        use std::mem::size_of;
        assert_eq!(size_of::<VitsCfg>(), 56);
        assert_eq!(size_of::<MatchaCfg>(), 56);
        assert_eq!(size_of::<KokoroCfg>(), 64);
        assert_eq!(size_of::<KittenCfg>(), 40);
        assert_eq!(size_of::<ZipvoiceCfg>(), 64);
        assert_eq!(size_of::<PocketCfg>(), 64);
        assert_eq!(size_of::<SupertonicCfg>(), 56);
        assert_eq!(size_of::<ModelCfg>(), 56 + 16 + 56 + 64 + 40 + 64 + 64 + 56);
        assert_eq!(size_of::<TtsCfg>(), size_of::<ModelCfg>() + 32);
        assert_eq!(size_of::<GeneratedAudio>(), 16);
        assert_eq!(size_of::<GenerationCfg>(), 56);
    }

    #[test]
    fn an_install_is_complete_only_with_every_file() {
        let dir = std::env::temp_dir().join(format!("clay-kokoro-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(install_in(&dir), Install::default(), "nothing there");
        let rt = dir.join("sherpa-onnx").join("lib");
        std::fs::create_dir_all(&rt).unwrap();
        std::fs::write(rt.join(runtime_lib_name()), b"x").unwrap();
        assert!(install_in(&dir).runtime.is_none(), "no VERSION: not trusted");
        std::fs::write(dir.join("sherpa-onnx").join("VERSION"), "1.0.0\n").unwrap();
        let i = install_in(&dir);
        assert!(i.runtime.is_none() && i.runtime_version.as_deref() == Some("1.0.0"), "wrong version: reported, not used");
        std::fs::write(dir.join("sherpa-onnx").join("VERSION"), format!("{RUNTIME_VERSION}\n")).unwrap();
        assert!(install_in(&dir).runtime.is_some());
        let m = dir.join(ModelKind::Int8.dir_name());
        std::fs::create_dir_all(m.join("espeak-ng-data")).unwrap();
        for f in ["model.int8.onnx", "voices.bin", "tokens.txt"] { std::fs::write(m.join(f), b"x").unwrap(); }
        assert!(install_in(&dir).model.is_none(), "a lexicon short");
        std::fs::write(m.join("lexicon-us-en.txt"), b"x").unwrap();
        let i = install_in(&dir);
        assert!(i.complete());
        assert_eq!(i.model_kind, Some(ModelKind::Int8));
        assert_eq!(model_file_in(&m), m.join("model.int8.onnx"));
        assert!(install_size(&dir) >= 5);
        // Both builds present: the one this CPU prefers wins.
        let f = dir.join(ModelKind::Full.dir_name());
        std::fs::create_dir_all(f.join("espeak-ng-data")).unwrap();
        for file in ["model.onnx", "voices.bin", "tokens.txt", "lexicon-us-en.txt"] { std::fs::write(f.join(file), b"x").unwrap(); }
        assert_eq!(install_in(&dir).model_kind, Some(preferred_model()));
        assert_eq!(model_file_in(&f), f.join("model.onnx"));
        // The leftover build goes only when the preferred one is complete, and only it.
        remove_stale_build_in(&dir);
        assert!(dir.join(preferred_model().dir_name()).is_dir(), "the preferred build stays");
        assert!(!dir.join(preferred_model().other().dir_name()).exists(), "the other one is gone");
        remove_stale_build_in(&dir);
        assert!(dir.join(preferred_model().dir_name()).is_dir(), "and nothing else happens");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// int8 only where the CPU has 8-bit matrix instructions: x86 needs VNNI (an i5-8400T
    /// with AVX2 alone ran int8 slower than real time); ARM gets the small build.
    #[test]
    fn the_model_build_follows_the_cpu() {
        assert_eq!(model_for_cpu("x86_64", false), ModelKind::Full);
        assert_eq!(model_for_cpu("x86_64", true), ModelKind::Int8);
        assert_eq!(model_for_cpu("aarch64", false), ModelKind::Int8);
        assert_eq!(model_for_cpu("aarch64", true), ModelKind::Int8);
        assert_ne!(ModelKind::Full.dir_name(), ModelKind::Int8.dir_name());
        assert_ne!(ModelKind::Full.file_name(), ModelKind::Int8.file_name());
    }

    #[test]
    fn wav_files_are_well_formed() {
        let path = std::env::temp_dir().join(format!("clay-kokoro-wav-{}.wav", std::process::id()));
        write_wav(&path, &[0.0, 0.5, -0.5, 1.0, -1.0], 24000).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 24000);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 10, "5 samples x 2 bytes");
        assert_eq!(bytes.len(), 44 + 10);
        assert_eq!(i16::from_le_bytes(bytes[50..52].try_into().unwrap()), 32767);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tar_bz2_extraction_keeps_only_the_wanted_prefix() {
        // A tiny archive made in-process: top/lib/a, top/bin/b.
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            for (name, body) in [("top/lib/a.so", b"lib" as &[u8]), ("top/bin/tool", b"bin")] {
                let mut h = tar::Header::new_gnu();
                h.set_size(body.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                b.append_data(&mut h, name, body).unwrap();
            }
            b.finish().unwrap();
        }
        let mut bz = Vec::new();
        {
            use std::io::Write;
            let mut enc = bzip2::write::BzEncoder::new(&mut bz, bzip2::Compression::fast());
            enc.write_all(&tar_bytes).unwrap();
            enc.finish().unwrap();
        }
        let dest = std::env::temp_dir().join(format!("clay-kokoro-tar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);
        extract_tar_bz2(&bz, &dest, Some("top/lib/")).unwrap();
        assert_eq!(std::fs::read(dest.join("top/lib/a.so")).unwrap(), b"lib");
        assert!(!dest.join("top/bin/tool").exists(), "bin/ left out");
        assert!(extract_tar_bz2(&bz, &dest, Some("nothing/")).is_err(), "an empty selection is an error");
        let _ = std::fs::remove_dir_all(&dest);
    }
}
