//! Import Picture: a picture goes up to a free picture host and its link goes into the
//! input line, at the cursor, with the cursor left at the link's start.
//!
//! The upload always runs on the server (one setting, one implementation; a browser can't
//! post to most hosts anyway). The main console uploads in-process; every other interface
//! sends the file to its server in chunks over the WebSocket (`PictureUpload`, answered per
//! chunk by `PictureUploadAck` and at the end by `PictureUploaded`), because the server caps
//! a frame at 256 KiB. This module holds the parts every side shares: the hosts and the
//! setting's values, recognising a picture by its first bytes, the upload itself, the
//! server's chunk assembly, a Rust client's sending side (remote console and native GUI),
//! the terminal-drop recogniser and the one rule for putting a link into an input line.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;

use crate::websocket::WsMessage;

/// The largest picture that is uploaded.
pub const MAX_PICTURE_BYTES: u64 = 32 * 1024 * 1024;
/// The most the server holds for pictures still arriving, over every client.
pub const MAX_PENDING_BYTES: u64 = 64 * 1024 * 1024;
/// Raw bytes per `PictureUpload` chunk (base64 makes it 128 KiB, under the server's
/// 256 KiB frame cap with the JSON around it).
pub const CHUNK_BYTES: usize = 96 * 1024;
/// Chunks a client keeps unacknowledged.
pub const ACK_WINDOW: usize = 8;
/// A client resends from the last acknowledged offset after this long without an ack
/// (an ack can be dropped when the client's outbound queue on the server is full).
pub const RESEND_AFTER: Duration = Duration::from_secs(10);
/// A client gives up this long after its last ack (no answer at all).
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(180);
/// The server drops a picture still arriving after this long without a chunk.
pub const IDLE_DROP_AFTER: Duration = Duration::from_secs(60);
/// The host's whole request, upload and reply.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

// ---------------------------------------------------------------------------------------
// Hosts and the setting
// ---------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PictureHost {
    Catbox,
    Sxcu,
    X0,
}

impl PictureHost {
    fn value(self) -> &'static str {
        match self {
            PictureHost::Catbox => "catbox",
            PictureHost::Sxcu => "sxcu",
            PictureHost::X0 => "x0",
        }
    }

    /// The host's name in messages ("Uploading cat.png to catbox.moe...").
    pub fn label(self) -> &'static str {
        match self {
            PictureHost::Catbox => "catbox.moe",
            PictureHost::Sxcu => "sxcu.net",
            PictureHost::X0 => "x0.at",
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            PictureHost::Catbox => "https://catbox.moe/user/api.php",
            PictureHost::Sxcu => "https://sxcu.net/api/files/create",
            PictureHost::X0 => "https://x0.at/",
        }
    }

    const ALL: [PictureHost; 3] = [PictureHost::Catbox, PictureHost::Sxcu, PictureHost::X0];
}

/// The Settings > General "Picture Host" choice: a host, optionally with the link run
/// through `/url`'s shortening, or Off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureService {
    pub host: Option<PictureHost>,
    pub shorten: bool,
}

impl Default for PictureService {
    fn default() -> Self {
        PictureService { host: Some(PictureHost::Catbox), shorten: false }
    }
}

impl PictureService {
    pub const OFF: PictureService = PictureService { host: None, shorten: false };

    /// Every choice as (settings value, label), in the drop-down's order. The console's
    /// `/setup`, the native GUI and (by a test) index.html's `<select>` all list these.
    pub const OPTIONS: &'static [(&'static str, &'static str)] = &[
        ("catbox", "catbox.moe"),
        ("catbox+short", "catbox.moe + URL shortening"),
        ("sxcu", "sxcu.net"),
        ("sxcu+short", "sxcu.net + URL shortening"),
        ("x0", "x0.at"),
        ("x0+short", "x0.at + URL shortening"),
        ("off", "Off"),
    ];

    /// The settings value. Empty or unknown (an older peer, a hand-edited file) reads as
    /// the default.
    pub fn from_name(name: &str) -> Self {
        let name = name.trim().to_ascii_lowercase();
        if name == "off" {
            return Self::OFF;
        }
        let (base, shorten) = match name.strip_suffix("+short") {
            Some(base) => (base, true),
            None => (name.as_str(), false),
        };
        match PictureHost::ALL.iter().find(|h| h.value() == base) {
            Some(host) => PictureService { host: Some(*host), shorten },
            None => Self::default(),
        }
    }

    pub fn name(&self) -> String {
        match self.host {
            None => "off".to_string(),
            Some(host) if self.shorten => format!("{}+short", host.value()),
            Some(host) => host.value().to_string(),
        }
    }

    pub fn label(&self) -> &'static str {
        let name = self.name();
        Self::OPTIONS.iter().find(|(v, _)| *v == name).map(|(_, l)| *l).unwrap_or("Off")
    }

    pub fn is_off(&self) -> bool {
        self.host.is_none()
    }
}

/// What to say when Import Picture is used with the host set to Off.
pub const OFF_MESSAGE: &str = "Picture uploads are off - choose a Picture Host in Settings (General).";

// ---------------------------------------------------------------------------------------
// Recognising a picture
// ---------------------------------------------------------------------------------------

/// A picture's type, from its first bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub ext: &'static str,
    pub mime: &'static str,
}

/// The picture types uploaded, recognised by their first bytes (never by the name).
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    let k = |ext, mime| Some(Kind { ext, mime });
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return k("png", "image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return k("jpg", "image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return k("gif", "image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return k("webp", "image/webp");
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return k("tiff", "image/tiff");
    }
    // BMP: "BM", four reserved zero bytes at 6, and a known DIB header size at 14 - "BM"
    // alone is the start of plenty of text.
    if bytes.len() >= 18 && bytes.starts_with(b"BM") && bytes[6..10] == [0, 0, 0, 0] {
        let dib = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        if matches!(dib, 12 | 40 | 52 | 56 | 64 | 108 | 124) {
            return k("bmp", "image/bmp");
        }
    }
    // ISO base media (HEIF/AVIF): a "ftyp" box whose brands name a still-image format.
    if bytes.len() >= 16 && &bytes[4..8] == b"ftyp" {
        let box_len = (u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize).min(bytes.len());
        let mut brands: Vec<&[u8]> = vec![&bytes[8..12]];
        let mut at = 16;
        while at + 4 <= box_len {
            brands.push(&bytes[at..at + 4]);
            at += 4;
        }
        if brands.iter().any(|b| matches!(*b, b"avif" | b"avis")) {
            return k("avif", "image/avif");
        }
        if brands.iter().any(|b| matches!(*b, b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1")) {
            return k("heic", "image/heic");
        }
    }
    None
}

/// The picture at `path` by its first bytes, without reading the rest.
pub fn sniff_file(path: &Path) -> Option<Kind> {
    use std::io::Read;
    let mut head = [0u8; 64];
    let mut file = std::fs::File::open(path).ok()?;
    let mut got = 0;
    while got < head.len() {
        match file.read(&mut head[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return None,
        }
    }
    sniff(&head[..got])
}

/// "2.1 MB", "340 KB", "812 bytes".
pub fn size_label(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{} bytes", bytes)
    }
}

/// A file's name for messages.
pub fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// Read a picture to upload: the whole file, refused when it's too big or isn't a picture.
pub fn read_picture(path: &Path) -> Result<(Vec<u8>, Kind), String> {
    let name = display_name(path);
    let len = std::fs::metadata(path).map_err(|e| format!("Couldn't read {}: {}", name, e))?.len();
    if len > MAX_PICTURE_BYTES {
        return Err(too_big_message(&name, len));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read {}: {}", name, e))?;
    match sniff(&bytes) {
        Some(kind) => Ok((bytes, kind)),
        None => Err(format!("{} isn't a picture.", name)),
    }
}

pub fn too_big_message(name: &str, len: u64) -> String {
    format!(
        "{} is {}; pictures over {} can't be uploaded.",
        name,
        size_label(len),
        size_label(MAX_PICTURE_BYTES)
    )
}

// ---------------------------------------------------------------------------------------
// The upload
// ---------------------------------------------------------------------------------------

/// A finished upload: the link, and a note when shortening was asked for and failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uploaded {
    pub url: String,
    pub note: Option<String>,
}

/// Upload `bytes` to the service's host and return the link, shortened when the service
/// says so. `shorteners` is `Settings::url_shorteners` (empty = the defaults).
pub async fn upload(
    service: PictureService,
    bytes: Vec<u8>,
    kind: Kind,
    shorteners: Vec<crate::encoding::UrlShortener>,
) -> Result<Uploaded, String> {
    let host = service.host.ok_or_else(|| OFF_MESSAGE.to_string())?;
    // End-to-end runs point every host at a local fake.
    let endpoint = std::env::var("CLAY_PICTURE_UPLOAD_URL").unwrap_or_else(|_| host.endpoint().to_string());
    upload_at(&endpoint, service, bytes, kind, shorteners).await
}

async fn upload_at(
    endpoint: &str,
    service: PictureService,
    bytes: Vec<u8>,
    kind: Kind,
    shorteners: Vec<crate::encoding::UrlShortener>,
) -> Result<Uploaded, String> {
    let host = service.host.ok_or_else(|| OFF_MESSAGE.to_string())?;

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(concat!("Clay/", env!("CARGO_PKG_VERSION"), " (+https://github.com/c-hudson/clay)"))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // The original name never leaves the machine; the host only needs the type.
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(format!("picture.{}", kind.ext))
        .mime_str(kind.mime)
        .map_err(|e| e.to_string())?;
    let form = match host {
        PictureHost::Catbox => reqwest::multipart::Form::new()
            .text("reqtype", "fileupload")
            .part("fileToUpload", part),
        PictureHost::X0 => reqwest::multipart::Form::new().part("file", part),
        // `noembed`: the link is the picture itself, not its page.
        PictureHost::Sxcu => reqwest::multipart::Form::new()
            .text("noembed", "")
            .part("file", part),
    };

    let response = client
        .post(endpoint)
        .multipart(form)
        .send()
        .await
        .map_err(|e| describe_send_error(&e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| format!("Failed to read the reply: {}", e))?;
    let url = parse_reply(host, status.as_u16(), &body)?;

    if !service.shorten {
        return Ok(Uploaded { url, note: None });
    }
    match crate::commands::shorten_url_fallback(&url, &shorteners).await {
        Ok(short) => Ok(Uploaded { url: short, note: None }),
        Err(e) => Ok(Uploaded {
            note: Some(format!("Couldn't shorten the link ({}); using the full one.", e)),
            url,
        }),
    }
}

fn describe_send_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "the host didn't answer in time".to_string()
    } else if e.is_connect() {
        "couldn't reach the host".to_string()
    } else {
        format!("the upload failed: {}", e)
    }
}

/// A host's reply: the link, or why not. A reply counts only if it is an https link;
/// anything else is the host's own explanation and is shown as the error.
fn parse_reply(host: PictureHost, status: u16, body: &str) -> Result<String, String> {
    if status == 429 {
        return Err(match host {
            PictureHost::Sxcu => "sxcu.net allows 3 uploads a minute - try again shortly".to_string(),
            _ => format!("{} says too many uploads - try again shortly", host.label()),
        });
    }
    let text = match host {
        PictureHost::Sxcu => {
            let json: Option<serde_json::Value> = serde_json::from_str(body).ok();
            let field = |name: &str| {
                json.as_ref()
                    .and_then(|j| j.get(name))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            };
            match field("url") {
                Some(url) if (200..300).contains(&status) => url,
                _ => {
                    return Err(field("error").unwrap_or_else(|| reply_error(status, body)));
                }
            }
        }
        _ => {
            if !(200..300).contains(&status) {
                return Err(reply_error(status, body));
            }
            body.trim().to_string()
        }
    };
    let text = text.trim().to_string();
    if text.to_ascii_lowercase().starts_with("https://") && !text.contains(char::is_whitespace) {
        Ok(text)
    } else if text.is_empty() {
        Err(format!("{} sent an empty reply", host.label()))
    } else {
        Err(format!("{} refused it: {}", host.label(), clip(&text)))
    }
}

fn reply_error(status: u16, body: &str) -> String {
    let body = clip(body.trim());
    if body.is_empty() || body.starts_with('<') {
        format!("HTTP {}", status)
    } else {
        format!("HTTP {}: {}", status, body)
    }
}

/// A host's text, made safe for one output line.
fn clip(text: &str) -> String {
    let one_line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if one_line.chars().count() > 200 {
        format!("{}...", one_line.chars().take(200).collect::<String>())
    } else {
        one_line
    }
}

// ---------------------------------------------------------------------------------------
// The server's side of a client's upload: assembling the chunks
// ---------------------------------------------------------------------------------------

struct Incoming {
    name: String,
    size: u64,
    bytes: Vec<u8>,
    last_chunk: Instant,
    /// Every byte is in and the upload to the host is running.
    uploading: bool,
}

/// What a `PictureUpload` chunk comes to.
#[derive(Debug, PartialEq, Eq)]
pub enum ChunkOutcome {
    /// Acknowledge this many bytes received.
    Ack(u64),
    /// The last byte is in: acknowledge it and upload.
    Complete { name: String, bytes: Vec<u8> },
    /// Not taken; the reason, for `PictureUploaded.error`.
    Refused(String),
}

/// The pictures clients are sending this server, keyed by (client, upload id): a native
/// GUI's `/window`s share their main window's client id.
#[derive(Default)]
pub struct IncomingUploads {
    map: HashMap<(u64, u64), Incoming>,
}

impl IncomingUploads {
    #[allow(clippy::too_many_arguments)]
    pub fn chunk(
        &mut self,
        client: u64,
        id: u64,
        name: &str,
        size: u64,
        offset: u64,
        data: &str,
        last: bool,
        now: Instant,
    ) -> ChunkOutcome {
        let key = (client, id);
        if let Some(entry) = self.map.get(&key) {
            if entry.uploading {
                return ChunkOutcome::Ack(entry.size);
            }
        } else {
            if size > MAX_PICTURE_BYTES {
                return ChunkOutcome::Refused(too_big_message(name, size));
            }
            if offset != 0 {
                return ChunkOutcome::Refused(format!("The upload of {} was interrupted.", name));
            }
            let pending: u64 = self.map.values().filter(|e| !e.uploading).map(|e| e.size).sum();
            if pending + size > MAX_PENDING_BYTES {
                return ChunkOutcome::Refused("Too many pictures are being uploaded at once - try again shortly.".to_string());
            }
            self.map.insert(key, Incoming {
                name: name.to_string(),
                size,
                bytes: Vec::with_capacity(size as usize),
                last_chunk: now,
                uploading: false,
            });
        }
        let entry = self.map.get_mut(&key).expect("just inserted");
        let received = entry.bytes.len() as u64;
        if offset != received {
            // A resend of chunks already here (its ack was lost), or ahead of a gap: say
            // where this side is and let the client carry on from there.
            return ChunkOutcome::Ack(received);
        }
        let decoded = match base64::engine::general_purpose::STANDARD.decode(data) {
            Ok(d) => d,
            Err(_) => {
                self.map.remove(&key);
                return ChunkOutcome::Refused(format!("The upload of {} was garbled.", name));
            }
        };
        if received + decoded.len() as u64 > entry.size {
            self.map.remove(&key);
            return ChunkOutcome::Refused(format!("The upload of {} was longer than announced.", name));
        }
        entry.bytes.extend_from_slice(&decoded);
        entry.last_chunk = now;
        let received = entry.bytes.len() as u64;
        if last || received == entry.size {
            if received != entry.size {
                self.map.remove(&key);
                return ChunkOutcome::Refused(format!("The upload of {} was cut short.", name));
            }
            entry.uploading = true;
            let bytes = std::mem::take(&mut entry.bytes);
            return ChunkOutcome::Complete { name: entry.name.clone(), bytes };
        }
        ChunkOutcome::Ack(received)
    }

    /// The upload to the host finished (either way).
    pub fn finished(&mut self, client: u64, id: u64) {
        self.map.remove(&(client, id));
    }

    /// A client went away: what it was still sending is dropped (an upload already running
    /// finishes and is simply not delivered).
    pub fn drop_client(&mut self, client: u64) {
        self.map.retain(|(c, _), e| *c != client || e.uploading);
    }

    /// Pictures still arriving with no chunk for `IDLE_DROP_AFTER` are dropped.
    pub fn expire(&mut self, now: Instant) {
        self.map.retain(|_, e| e.uploading || now.duration_since(e.last_chunk) < IDLE_DROP_AFTER);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }
}

// ---------------------------------------------------------------------------------------
// A Rust client's side (the remote console and the native GUI)
// ---------------------------------------------------------------------------------------

static NEXT_UPLOAD_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A process-wide upload id: a native GUI's `/window`s share one connection and see each
/// other's acks, so two windows must never pick the same one.
pub fn next_upload_id() -> u64 {
    NEXT_UPLOAD_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

struct Outgoing {
    id: u64,
    name: String,
    bytes: Vec<u8>,
    sent: usize,
    acked: usize,
    last_progress: Instant,
}

impl Outgoing {
    /// The chunks the ack window lets go now; after `RESEND_AFTER` without an ack, from the
    /// last acknowledged offset again.
    fn due(&mut self, now: Instant) -> Vec<WsMessage> {
        if self.sent > self.acked && now.duration_since(self.last_progress) >= RESEND_AFTER {
            self.sent = self.acked;
            self.last_progress = now;
        }
        let mut out = Vec::new();
        while self.sent < self.bytes.len() && self.sent - self.acked < ACK_WINDOW * CHUNK_BYTES {
            let end = (self.sent + CHUNK_BYTES).min(self.bytes.len());
            out.push(WsMessage::PictureUpload {
                id: self.id,
                name: self.name.clone(),
                size: self.bytes.len() as u64,
                offset: self.sent as u64,
                data: base64::engine::general_purpose::STANDARD.encode(&self.bytes[self.sent..end]),
                last: end == self.bytes.len(),
            });
            self.sent = end;
        }
        out
    }
}

struct Batch {
    pictures: VecDeque<(String, Vec<u8>)>,
    links: Vec<String>,
}

/// What the client's uploads have for its user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientNote {
    /// A line for the output.
    Line(String),
    /// A batch is done: these links (space-separated) go into the input line.
    Insert(String),
}

/// A Rust client's uploads: batches (one drop or one pick each) sent one picture at a time,
/// each batch's links inserted together when its last picture is done.
#[derive(Default)]
pub struct ClientUploads {
    batches: VecDeque<Batch>,
    current: Option<Outgoing>,
}

impl ClientUploads {
    pub fn is_busy(&self) -> bool {
        self.current.is_some() || !self.batches.is_empty()
    }

    pub fn owns(&self, id: u64) -> bool {
        self.current.as_ref().is_some_and(|c| c.id == id)
    }

    /// Queue one batch of (name, bytes) pictures; returns what to send now.
    pub fn add_batch(&mut self, pictures: Vec<(String, Vec<u8>)>, now: Instant) -> (Vec<WsMessage>, Vec<ClientNote>) {
        if pictures.is_empty() {
            return (Vec::new(), Vec::new());
        }
        self.batches.push_back(Batch { pictures: pictures.into(), links: Vec::new() });
        if self.current.is_some() {
            return (Vec::new(), Vec::new());
        }
        self.start_next(now)
    }

    fn start_next(&mut self, now: Instant) -> (Vec<WsMessage>, Vec<ClientNote>) {
        let mut notes = Vec::new();
        while let Some(batch) = self.batches.front_mut() {
            if let Some((name, bytes)) = batch.pictures.pop_front() {
                let mut out = Outgoing { id: next_upload_id(), name, bytes, sent: 0, acked: 0, last_progress: now };
                let msgs = out.due(now);
                self.current = Some(out);
                return (msgs, notes);
            }
            let done = self.batches.pop_front().expect("front exists");
            if !done.links.is_empty() {
                notes.push(ClientNote::Insert(done.links.join(" ")));
            }
        }
        (Vec::new(), notes)
    }

    pub fn on_ack(&mut self, id: u64, received: u64, now: Instant) -> Vec<WsMessage> {
        match self.current.as_mut() {
            Some(cur) if cur.id == id => {
                let received = (received as usize).min(cur.bytes.len());
                if received > cur.acked {
                    cur.acked = received;
                    cur.last_progress = now;
                }
                // A lost ack made this side resend; the server says it's further along.
                if received > cur.sent {
                    cur.sent = received;
                }
                cur.due(now)
            }
            _ => Vec::new(),
        }
    }

    /// The server's answer for an upload; `true` in the third place when it was this
    /// client's (a native `/window` ignores the others).
    pub fn on_uploaded(
        &mut self,
        id: u64,
        url: Option<String>,
        error: Option<String>,
        now: Instant,
    ) -> (Vec<WsMessage>, Vec<ClientNote>, bool) {
        if !self.owns(id) {
            return (Vec::new(), Vec::new(), false);
        }
        let cur = self.current.take().expect("owns");
        let mut notes = Vec::new();
        match url {
            Some(url) => {
                if let Some(batch) = self.batches.front_mut() {
                    batch.links.push(url);
                }
            }
            None => notes.push(ClientNote::Line(format!(
                "Couldn't upload {}: {}",
                cur.name,
                error.unwrap_or_else(|| "the server gave no reason".to_string())
            ))),
        }
        let (msgs, more) = self.start_next(now);
        notes.extend(more);
        (msgs, notes, true)
    }

    /// Resend after a lost ack, or give up after `GIVE_UP_AFTER` with no answer.
    pub fn tick(&mut self, now: Instant) -> (Vec<WsMessage>, Vec<ClientNote>) {
        let Some(cur) = self.current.as_mut() else {
            return (Vec::new(), Vec::new());
        };
        if now.duration_since(cur.last_progress) >= GIVE_UP_AFTER {
            let name = cur.name.clone();
            self.current = None;
            let mut notes = vec![ClientNote::Line(format!("Couldn't upload {}: the server didn't answer.", name))];
            let (msgs, more) = self.start_next(now);
            notes.extend(more);
            return (msgs, notes);
        }
        (cur.due(now), Vec::new())
    }

    /// The connection went: everything still to send is gone.
    pub fn connection_lost(&mut self) -> Vec<ClientNote> {
        if !self.is_busy() {
            return Vec::new();
        }
        self.current = None;
        self.batches.clear();
        vec![ClientNote::Line("Picture upload interrupted (connection lost).".to_string())]
    }
}

// ---------------------------------------------------------------------------------------
// A picture dropped onto a terminal
// ---------------------------------------------------------------------------------------

/// Split what a terminal sends for a drop into words the way a shell would: single quotes
/// (VTE's `g_shell_quote`, `'it'\''s.png'`), double quotes, backslash escapes, bare words.
/// On Windows a backslash is a path separator and only double quotes quote. `None` for
/// anything a drop wouldn't look like (unbalanced quotes, a newline, too long, too many).
pub fn split_dropped_words(text: &str, windows: bool) -> Option<Vec<String>> {
    if text.len() > 16 * 1024 || text.contains(['\n', '\r']) {
        return None;
    }
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' if !windows => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return None,
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if !windows => match chars.peek() {
                            Some(&n) if matches!(n, '"' | '\\' | '$' | '`') => {
                                cur.push(n);
                                chars.next();
                            }
                            Some(_) => cur.push('\\'),
                            None => return None,
                        },
                        Some(ch) => cur.push(ch),
                        None => return None,
                    }
                }
            }
            '\\' if !windows => {
                in_word = true;
                cur.push(chars.next()?);
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    if words.is_empty() || words.len() > 20 {
        return None;
    }
    Some(words)
}

/// A dropped word as a path: a `file://` URI (percent-decoded) or the path itself, and
/// absolute either way.
fn word_to_path(word: &str) -> Option<PathBuf> {
    let path = if word.len() > 7 && word[..7].eq_ignore_ascii_case("file://") {
        url::Url::parse(word).ok()?.to_file_path().ok()?
    } else {
        PathBuf::from(word)
    };
    path.is_absolute().then_some(path)
}

/// The pictures a terminal drop names, when that's all `text` is: one or more paths (in any
/// of the forms terminals send), each an existing file that is a picture by its first
/// bytes. Anything else - a .txt, a missing file, words around a path - is `None`, and the
/// text is ordinary text.
pub fn parse_dropped_paths(text: &str) -> Option<Vec<PathBuf>> {
    let words = split_dropped_words(text, cfg!(windows))?;
    let mut paths = Vec::with_capacity(words.len());
    for word in words {
        let path = word_to_path(&word)?;
        if !path.is_file() || sniff_file(&path).is_none() {
            return None;
        }
        paths.push(path);
    }
    Some(paths)
}

/// Recognises a drop that a terminal *types* (xfce4-terminal feeds the quoted path to the
/// program as keystrokes, not as a paste; Windows' console does the same). A burst is a run
/// of characters typed into the input line while more terminal input was already waiting -
/// `crossterm::event::poll(Duration::ZERO)` after each one, never `EventStream::now_or_never`
/// (which leaves crossterm 0.27's wake thread holding a no-op waker). No one types that fast,
/// so when the input runs dry the run is either a drop or nothing.
#[derive(Default)]
pub struct DropSniffer {
    start: usize,
    run: String,
    active: bool,
    /// This key fed the run (`begin_key` clears it): any other key ends the run.
    fed: bool,
    swallow_until: Option<Instant>,
}

impl DropSniffer {
    /// A key is about to be handled.
    pub fn begin_key(&mut self) {
        self.fed = false;
    }

    /// A character was typed into the input line at byte `at` (insert mode only: in
    /// overwrite mode it replaced text that taking it back out couldn't restore).
    pub fn typed(&mut self, at: usize, c: char) {
        if !(self.active && at == self.start + self.run.len()) {
            self.start = at;
            self.run.clear();
            self.active = true;
        }
        self.run.push(c);
        self.fed = true;
    }

    /// The key was handled: if it didn't feed the run (a binding, a chord, an editing key),
    /// the run is over and wasn't a drop.
    pub fn end_key(&mut self) {
        if !self.fed {
            self.reset();
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.run.clear();
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// No more input is waiting: the run is over. Returns it (start byte, text) when it's
    /// long enough to be a drop.
    pub fn finish(&mut self) -> Option<(usize, String)> {
        if !self.active {
            return None;
        }
        self.active = false;
        let run = std::mem::take(&mut self.run);
        (run.chars().count() >= 3).then_some((self.start, run))
    }

    /// xfce4-terminal sends the drop's trailing space as a write of its own; when it comes
    /// right after, it belongs to the drop.
    pub fn arm_swallow(&mut self, now: Instant) {
        self.swallow_until = Some(now + Duration::from_millis(50));
    }

    /// Whether this key is that trailing space. Any key disarms it.
    pub fn swallow(&mut self, is_plain_space: bool, now: Instant) -> bool {
        match self.swallow_until.take() {
            Some(until) => is_plain_space && now <= until,
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Putting the link into an input line
// ---------------------------------------------------------------------------------------

/// The one rule for inserting a link at a cursor: a space before it unless the character
/// before is whitespace (or there is none), a space after it unless the character after is.
/// Returns the text to insert and where the link starts inside it.
pub fn link_insertion(before: Option<char>, after: Option<char>, link: &str) -> (String, usize) {
    let pre = before.is_some_and(|c| !c.is_whitespace());
    let post = after.is_some_and(|c| !c.is_whitespace());
    let mut text = String::with_capacity(link.len() + 2);
    if pre {
        text.push(' ');
    }
    text.push_str(link);
    if post {
        text.push(' ');
    }
    (text, usize::from(pre))
}

// ---------------------------------------------------------------------------------------
// The App's side: the server's uploads, the master console's own, and a remote console's
// ---------------------------------------------------------------------------------------

/// The master console's own pictures (client 0): batches of paths, one picture up at a time.
#[derive(Default)]
pub struct ConsolePictures {
    batches: VecDeque<(VecDeque<PathBuf>, Vec<String>)>,
    current: Option<u64>,
}

impl crate::App {
    /// The pictures a terminal drop in either console's input line names (`text` is the
    /// bracketed paste or the typed burst), or `None` for ordinary text: uploads off, a
    /// server that can't take them, or not a drop of pictures.
    pub(crate) fn picture_drop(&self, text: &str) -> Option<Vec<PathBuf>> {
        if self.settings.picture_host.is_off() || (!self.is_master && !self.server_picture_upload) {
            return None;
        }
        parse_dropped_paths(text)
    }

    /// A typed burst is over (no more terminal input waiting): if it was a drop of
    /// pictures, it comes back out of the input line and the y/n question goes up.
    pub(crate) fn finish_typed_burst(&mut self) {
        let Some((start, run)) = self.drop_sniffer.finish() else {
            return;
        };
        let Some(paths) = self.picture_drop(&run) else {
            return;
        };
        let end = start + run.len();
        if self.input.buffer.get(start..end) != Some(run.as_str()) {
            return;
        }
        self.input.buffer.replace_range(start..end, "");
        self.input.cursor_position = start;
        self.input.adjust_viewport();
        self.spell_state.reset();
        if !run.ends_with(' ') {
            self.drop_sniffer.arm_swallow(Instant::now());
        }
        self.ask_picture_upload(paths);
    }

    /// Ask before uploading a terminal drop (user ruling: uploads are public, and a dropped
    /// path can't be told from a pasted one). No is the default.
    pub(crate) fn ask_picture_upload(&mut self, paths: Vec<PathBuf>) {
        use crate::popup::definitions::confirm::{create_picture_upload_dialog, CONFIRM_BTN_NO};
        let total: u64 = paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let what = if paths.len() == 1 { display_name(&paths[0]) } else { format!("{} pictures", paths.len()) };
        let host = self.settings.picture_host.host.map(|h| h.label()).unwrap_or("the picture host");
        let message = format!("Upload {} ({}) to {}?\n\nAnyone with the link can see it.", what, size_label(total), host);
        self.pending_picture_drop = Some(paths);
        self.popup_manager.open(create_picture_upload_dialog(&message));
        if let Some(state) = self.popup_manager.current_mut() {
            state.select_button(CONFIRM_BTN_NO);
        }
    }

    /// The y/n question answered: Yes uploads, from whichever console asked.
    pub(crate) fn picture_drop_answered(&mut self, yes: bool) {
        let Some(paths) = self.pending_picture_drop.take() else {
            return;
        };
        if !yes {
            return;
        }
        if self.is_master {
            self.console_pictures.batches.push_back((paths.into(), Vec::new()));
            if self.console_pictures.current.is_none() {
                self.console_next_picture();
            }
        } else {
            self.client_upload_pictures(paths);
        }
    }

    /// Master console: the next picture of its batches, or a finished batch's links.
    fn console_next_picture(&mut self) {
        loop {
            let next = match self.console_pictures.batches.front_mut() {
                None => return,
                Some((paths, _)) => paths.pop_front(),
            };
            match next {
                Some(path) => match read_picture(&path) {
                    Ok((bytes, kind)) => {
                        let id = next_upload_id();
                        self.console_pictures.current = Some(id);
                        self.start_picture_upload(0, id, display_name(&path), bytes, kind);
                        return;
                    }
                    Err(e) => self.add_output(&e),
                },
                None => {
                    let (_, links) = self.console_pictures.batches.pop_front().expect("front exists");
                    if !links.is_empty() {
                        self.insert_picture_links(&links.join(" "));
                    }
                }
            }
        }
    }

    /// Whether upload `client_id` is the master console's own: client 0 in the console
    /// TUI (in a headless GUI master, client 0 is the embedded GUI, a client like any other).
    fn is_console_upload(&self, client_id: u64) -> bool {
        client_id == 0 && self.console_tui
    }

    /// A line about an upload, for the master console or the client whose it is.
    fn picture_line(&mut self, client_id: u64, text: &str) {
        if self.is_console_upload(client_id) {
            self.add_output(text);
        } else {
            let world = self.ws_client_worlds.get(&client_id).map(|s| s.world_index).unwrap_or(self.current_world_index);
            self.ws_tell(client_id, world, text);
        }
    }

    /// Upload one picture to the Settings' host, for the master console or a client. The
    /// result comes back as `AppEvent::PictureUploaded` (`apply_picture_upload`).
    pub(crate) fn start_picture_upload(&mut self, client_id: u64, id: u64, name: String, bytes: Vec<u8>, kind: Kind) {
        let service = self.settings.picture_host;
        let (Some(host), Some(tx)) = (service.host, self.event_tx.clone()) else {
            self.apply_picture_upload(client_id, id, name, Err(OFF_MESSAGE.to_string()));
            return;
        };
        self.picture_line(client_id, &format!("Uploading {} ({}) to {}...", name, size_label(bytes.len() as u64), host.label()));
        let shorteners = self.settings.url_shorteners.clone();
        tokio::spawn(async move {
            let result = upload(service, bytes, kind, shorteners).await;
            let _ = tx.send(crate::AppEvent::PictureUploaded { client_id, id, name, result }).await;
        });
    }

    /// An upload finished: the master console's batch goes on, a client is told.
    pub(crate) fn apply_picture_upload(&mut self, client_id: u64, id: u64, name: String, result: Result<Uploaded, String>) {
        self.picture_uploads.finished(client_id, id);
        if self.is_console_upload(client_id) {
            if self.console_pictures.current != Some(id) {
                return;
            }
            self.console_pictures.current = None;
            match result {
                Ok(up) => {
                    if let Some(note) = up.note {
                        self.add_output(&note);
                    }
                    if let Some((_, links)) = self.console_pictures.batches.front_mut() {
                        links.push(up.url);
                    }
                }
                Err(e) => self.add_output(&format!("Couldn't upload {}: {}", name, e)),
            }
            self.console_next_picture();
            return;
        }
        let (url, error) = match result {
            Ok(up) => {
                if let Some(note) = up.note {
                    self.picture_line(client_id, &note);
                }
                (Some(up.url), None)
            }
            Err(e) => (None, Some(e)),
        };
        self.ws_send_to_client(client_id, WsMessage::PictureUploaded { id, url, error });
    }

    /// A client's `PictureUpload` chunk (both server handlers): acknowledged, and when the
    /// last byte is in, uploaded.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ws_picture_chunk(&mut self, client_id: u64, id: u64, name: String, size: u64, offset: u64, data: String, last: bool) {
        let refuse = |app: &Self, error: String| {
            app.ws_send_to_client(client_id, WsMessage::PictureUploaded { id, url: None, error: Some(error) });
        };
        if self.settings.picture_host.is_off() {
            refuse(self, OFF_MESSAGE.to_string());
            return;
        }
        let now = Instant::now();
        self.picture_uploads.expire(now);
        match self.picture_uploads.chunk(client_id, id, &name, size, offset, &data, last, now) {
            ChunkOutcome::Ack(received) => self.ws_send_to_client(client_id, WsMessage::PictureUploadAck { id, received }),
            ChunkOutcome::Refused(error) => refuse(self, error),
            ChunkOutcome::Complete { name, bytes } => {
                self.ws_send_to_client(client_id, WsMessage::PictureUploadAck { id, received: bytes.len() as u64 });
                match sniff(&bytes) {
                    Some(kind) => self.start_picture_upload(client_id, id, name, bytes, kind),
                    None => {
                        self.picture_uploads.finished(client_id, id);
                        refuse(self, format!("{} isn't a picture.", name));
                    }
                }
            }
        }
    }

    /// A batch's links into this console's input line, by the one rule
    /// (`InputArea::insert_link`); in a live shell world, pasted into the shell instead.
    pub(crate) fn insert_picture_links(&mut self, links: &str) {
        let idx = self.current_world_index;
        if self.shell_live(idx) {
            if self.is_master {
                self.shell_input(idx, None, Some(links.to_string()), true);
            } else if let Some(tx) = &self.ws_client_tx {
                let _ = tx.send(WsMessage::ShellInput { world_index: idx, key: None, text: Some(links.to_string()), paste: true });
            }
            return;
        }
        self.input.insert_link(links);
        self.spell_state.reset();
    }

    /// Remote console: read the confirmed pictures and send them to the server.
    fn client_upload_pictures(&mut self, paths: Vec<PathBuf>) {
        let mut pictures = Vec::new();
        for path in paths {
            match read_picture(&path) {
                Ok((bytes, _)) => pictures.push((display_name(&path), bytes)),
                Err(e) => self.add_output(&e),
            }
        }
        let (msgs, notes) = self.picture_client.add_batch(pictures, Instant::now());
        self.send_picture_messages(msgs);
        self.apply_picture_notes(notes);
    }

    /// Remote console: what its uploads have for the user.
    pub(crate) fn apply_picture_notes(&mut self, notes: Vec<ClientNote>) {
        for note in notes {
            match note {
                ClientNote::Line(line) => self.add_output(&line),
                ClientNote::Insert(links) => self.insert_picture_links(&links),
            }
        }
    }

    /// Remote console: chunks for the server.
    pub(crate) fn send_picture_messages(&self, msgs: Vec<WsMessage>) {
        if let Some(tx) = &self.ws_client_tx {
            for msg in msgs {
                let _ = tx.send(msg);
            }
        }
    }
}

/// For tests: one fake picture host for the whole test process (plain HTTP on loopback,
/// every upload answered `https://pics.example/<n>.<ext>` from the part's file name), with
/// `CLAY_PICTURE_UPLOAD_URL` pointed at it - set once, so tests never race over it.
#[cfg(test)]
pub(crate) fn test_host() {
    use std::io::{Read, Write};
    static HOST: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    HOST.get_or_init(|| {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind the fake host");
        let port = listener.local_addr().unwrap().port();
        std::env::set_var("CLAY_PICTURE_UPLOAD_URL", format!("http://127.0.0.1:{}/", port));
        std::thread::spawn(move || {
            let mut n = 0u64;
            for conn in listener.incoming() {
                let Ok(mut sock) = conn else { continue };
                n += 1;
                let n = n;
                std::thread::spawn(move || {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 8192];
                    let mut want = None;
                    loop {
                        if let Some(w) = want {
                            if req.len() >= w {
                                break;
                            }
                        }
                        match sock.read(&mut buf) {
                            Ok(0) | Err(_) => return,
                            Ok(k) => req.extend_from_slice(&buf[..k]),
                        }
                        if want.is_none() {
                            if let Some(end) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                                let head = String::from_utf8_lossy(&req[..end]).to_ascii_lowercase();
                                let len: usize = head
                                    .lines()
                                    .find_map(|l| l.strip_prefix("content-length:"))
                                    .and_then(|v| v.trim().parse().ok())
                                    .unwrap_or(0);
                                want = Some(end + 4 + len);
                            }
                        }
                    }
                    let text = String::from_utf8_lossy(&req);
                    let ext = text
                        .split("filename=\"picture.")
                        .nth(1)
                        .and_then(|r| r.split('"').next())
                        .unwrap_or("bin")
                        .to_string();
                    let body = format!("https://pics.example/{}.{}", n, ext);
                    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    let _ = sock.write_all(reply.as_bytes());
                });
            }
        });
    });
}

/// For tests: a small PNG's bytes.
#[cfg(test)]
pub(crate) fn test_png() -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(&[0u8; 32]);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend_from_slice(&[0u8; 32]);
        b
    }

    #[test]
    fn the_setting_reads_every_option_and_defaults_to_catbox() {
        for (value, label) in PictureService::OPTIONS {
            let s = PictureService::from_name(value);
            assert_eq!(s.name(), *value);
            assert_eq!(s.label(), *label);
        }
        assert_eq!(PictureService::from_name(""), PictureService::default());
        assert_eq!(PictureService::from_name("nonsense"), PictureService::default());
        assert_eq!(PictureService::default().name(), "catbox");
        assert!(PictureService::from_name("off").is_off());
        assert_eq!(PictureService::from_name("x0+short"), PictureService { host: Some(PictureHost::X0), shorten: true });
    }

    #[test]
    fn pictures_are_known_by_their_first_bytes() {
        assert_eq!(sniff(&png()).unwrap().ext, "png");
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]).unwrap().ext, "jpg");
        assert_eq!(sniff(b"GIF89a....").unwrap().ext, "gif");
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 ").unwrap().ext, "webp");
        assert_eq!(sniff(b"II*\0\x08\0\0\0").unwrap().ext, "tiff");
        let mut bmp = b"BM\0\0\0\0\0\0\0\0\x36\0\0\0".to_vec();
        bmp.extend_from_slice(&40u32.to_le_bytes());
        assert_eq!(sniff(&bmp).unwrap().ext, "bmp");
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic").unwrap().ext, "heic");
        assert_eq!(sniff(b"\0\0\0\x1cftypmif1\0\0\0\0mif1avifmiaf").unwrap().ext, "avif");
        // Text that merely starts like one isn't.
        assert_eq!(sniff(b"BMW 320i owner's manual, chapter one"), None);
        assert_eq!(sniff(b"hello"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn a_drop_is_split_as_a_shell_would() {
        let w = |t: &str| split_dropped_words(t, false);
        assert_eq!(w("'/home/u/cat.png' "), Some(vec!["/home/u/cat.png".to_string()]));
        assert_eq!(w("'/tmp/it'\\''s.png'"), Some(vec!["/tmp/it's.png".to_string()]));
        assert_eq!(w("/tmp/My\\ Pic.png"), Some(vec!["/tmp/My Pic.png".to_string()]));
        assert_eq!(w("\"/tmp/a b.png\" '/tmp/c.png'"), Some(vec!["/tmp/a b.png".to_string(), "/tmp/c.png".to_string()]));
        assert_eq!(w("\"/tmp/\\\"q\\\".png\""), Some(vec!["/tmp/\"q\".png".to_string()]));
        assert_eq!(w("'/tmp/unbalanced.png"), None);
        assert_eq!(w("   "), None);
        assert_eq!(w("a\nb"), None);
        // Windows: backslashes are separators, double quotes quote, single quotes are text.
        let win = |t: &str| split_dropped_words(t, true);
        assert_eq!(win("\"C:\\Users\\me\\My Pic.png\""), Some(vec!["C:\\Users\\me\\My Pic.png".to_string()]));
        assert_eq!(win("C:\\Users\\me\\cat.png "), Some(vec!["C:\\Users\\me\\cat.png".to_string()]));
        assert_eq!(win("C:\\it's.png"), Some(vec!["C:\\it's.png".to_string()]));
    }

    #[test]
    fn a_drop_names_existing_pictures_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("clay-picture-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pic = dir.join("my cat.png");
        std::fs::write(&pic, png()).unwrap();
        let txt = dir.join("notes.txt");
        std::fs::write(&txt, "not a picture").unwrap();
        let p = pic.to_string_lossy().to_string();

        assert_eq!(parse_dropped_paths(&format!("'{}' ", p)), Some(vec![pic.clone()]));
        assert_eq!(parse_dropped_paths(&p.replace(' ', "\\ ")), Some(vec![pic.clone()]));
        let uri = format!("file://{}", p.replace(' ', "%20"));
        assert_eq!(parse_dropped_paths(&uri), Some(vec![pic.clone()]));
        assert_eq!(parse_dropped_paths(&format!("'{}' '{}'", p, p)), Some(vec![pic.clone(), pic.clone()]));
        // A text file, a missing file, words around a path, a relative path: ordinary text.
        assert_eq!(parse_dropped_paths(&format!("'{}'", txt.display())), None);
        assert_eq!(parse_dropped_paths(&format!("'{}' '{}'", p, txt.display())), None);
        assert_eq!(parse_dropped_paths("'/no/such/picture.png'"), None);
        assert_eq!(parse_dropped_paths(&format!("look at '{}'", p)), None);
        assert_eq!(parse_dropped_paths("my cat.png"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_link_goes_in_without_touching_the_text_around_it() {
        let ins = |b: Option<char>, a: Option<char>| link_insertion(b, a, "https://x/y.png");
        assert_eq!(ins(None, None), ("https://x/y.png".to_string(), 0));
        assert_eq!(ins(Some('s'), None), (" https://x/y.png".to_string(), 1));
        assert_eq!(ins(Some(' '), None), ("https://x/y.png".to_string(), 0));
        assert_eq!(ins(None, Some('w')), ("https://x/y.png ".to_string(), 0));
        assert_eq!(ins(Some('a'), Some('b')), (" https://x/y.png ".to_string(), 1));
        assert_eq!(ins(Some('é'), Some(' ')), (" https://x/y.png".to_string(), 1));
    }

    #[test]
    fn a_burst_with_more_input_waiting_is_a_drop_candidate() {
        let mut s = DropSniffer::default();
        // A drop: every character with more input already waiting, then it runs dry.
        for (i, c) in "'/tmp/a.png'".chars().enumerate() {
            s.begin_key();
            s.typed(5 + i, c);
            s.end_key();
            assert!(s.is_active());
        }
        assert_eq!(s.finish(), Some((5, "'/tmp/a.png'".to_string())));
        assert!(!s.is_active());

        // Typing: each character finishes its own run (nothing waiting) and is too short.
        for (i, c) in "'/tmp/a.png'".chars().enumerate() {
            s.begin_key();
            s.typed(i, c);
            s.end_key();
            assert_eq!(s.finish(), None);
        }

        // An editing key in the middle ends the run.
        s.begin_key();
        s.typed(0, '\'');
        s.end_key();
        s.begin_key();
        s.end_key(); // e.g. Left: didn't feed it
        assert!(!s.is_active());

        // The separate trailing space is swallowed only right after.
        let now = Instant::now();
        s.arm_swallow(now);
        assert!(s.swallow(true, now + Duration::from_millis(10)));
        assert!(!s.swallow(true, now + Duration::from_millis(20)));
        s.arm_swallow(now);
        assert!(!s.swallow(true, now + Duration::from_millis(80)));
        s.arm_swallow(now);
        assert!(!s.swallow(false, now));
        assert!(!s.swallow(true, now));
    }

    #[test]
    fn host_replies_are_links_or_errors() {
        assert_eq!(parse_reply(PictureHost::Catbox, 200, "https://files.catbox.moe/ab12cd.png\n"), Ok("https://files.catbox.moe/ab12cd.png".to_string()));
        assert_eq!(parse_reply(PictureHost::X0, 200, "https://x0.at/AbCd.png"), Ok("https://x0.at/AbCd.png".to_string()));
        assert_eq!(
            parse_reply(PictureHost::Sxcu, 200, r#"{"id":"AbC12","url":"https://sxcu.net/AbC12.png","del_url":"x"}"#),
            Ok("https://sxcu.net/AbC12.png".to_string())
        );
        assert_eq!(parse_reply(PictureHost::Sxcu, 400, r#"{"error":"File type not allowed","code":4}"#), Err("File type not allowed".to_string()));
        assert!(parse_reply(PictureHost::Sxcu, 429, "{}").unwrap_err().contains("3 uploads a minute"));
        assert_eq!(parse_reply(PictureHost::Catbox, 412, "No file"), Err("HTTP 412: No file".to_string()));
        assert_eq!(parse_reply(PictureHost::Catbox, 500, "<html>oops</html>"), Err("HTTP 500".to_string()));
        assert_eq!(parse_reply(PictureHost::Catbox, 200, "File too large."), Err("catbox.moe refused it: File too large.".to_string()));
        assert_eq!(parse_reply(PictureHost::X0, 200, "http://insecure/x.png"), Err("x0.at refused it: http://insecure/x.png".to_string()));
        assert!(parse_reply(PictureHost::X0, 200, "  ").unwrap_err().contains("empty"));
    }

    fn chunks_of(bytes: &[u8]) -> Vec<(u64, String, bool)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let end = (at + CHUNK_BYTES).min(bytes.len());
            out.push((at as u64, base64::engine::general_purpose::STANDARD.encode(&bytes[at..end]), end == bytes.len()));
            at = end;
        }
        out
    }

    #[test]
    fn the_server_assembles_chunks_in_order_and_acks_each() {
        let mut up = IncomingUploads::default();
        let now = Instant::now();
        let mut bytes = png();
        bytes.resize(CHUNK_BYTES * 2 + 100, 7);
        let size = bytes.len() as u64;
        let chunks = chunks_of(&bytes);
        assert_eq!(up.chunk(1, 9, "a.png", size, chunks[0].0, &chunks[0].1, false, now), ChunkOutcome::Ack(CHUNK_BYTES as u64));
        // A resend of the first chunk (its ack was lost): told where this side is.
        assert_eq!(up.chunk(1, 9, "a.png", size, chunks[0].0, &chunks[0].1, false, now), ChunkOutcome::Ack(CHUNK_BYTES as u64));
        assert_eq!(up.chunk(1, 9, "a.png", size, chunks[1].0, &chunks[1].1, false, now), ChunkOutcome::Ack(2 * CHUNK_BYTES as u64));
        match up.chunk(1, 9, "a.png", size, chunks[2].0, &chunks[2].1, true, now) {
            ChunkOutcome::Complete { name, bytes: got } => {
                assert_eq!(name, "a.png");
                assert_eq!(got, bytes);
            }
            other => panic!("{:?}", other),
        }
        // While it uploads, a late resend is answered with everything received.
        assert_eq!(up.chunk(1, 9, "a.png", size, 0, &chunks[0].1, false, now), ChunkOutcome::Ack(size));
        up.finished(1, 9);
        assert_eq!(up.len(), 0);
    }

    #[test]
    fn the_server_refuses_what_it_shouldnt_hold() {
        let mut up = IncomingUploads::default();
        let now = Instant::now();
        assert!(matches!(up.chunk(1, 1, "big.png", MAX_PICTURE_BYTES + 1, 0, "", false, now), ChunkOutcome::Refused(m) if m.contains("can't be uploaded")));
        assert!(matches!(up.chunk(1, 2, "x.png", 10, 5, "", false, now), ChunkOutcome::Refused(m) if m.contains("interrupted")));
        assert!(matches!(up.chunk(1, 3, "x.png", 10, 0, "!!!", false, now), ChunkOutcome::Refused(m) if m.contains("garbled")));
        let data = base64::engine::general_purpose::STANDARD.encode([1u8; 20]);
        assert!(matches!(up.chunk(1, 4, "x.png", 10, 0, &data, false, now), ChunkOutcome::Refused(m) if m.contains("longer")));
        let data = base64::engine::general_purpose::STANDARD.encode([1u8; 5]);
        assert!(matches!(up.chunk(1, 5, "x.png", 10, 0, &data, true, now), ChunkOutcome::Refused(m) if m.contains("cut short")));
        // The total held is capped.
        assert_eq!(up.chunk(1, 6, "a.png", MAX_PICTURE_BYTES, 0, &data, false, now), ChunkOutcome::Ack(5));
        assert_eq!(up.chunk(2, 6, "b.png", MAX_PICTURE_BYTES, 0, &data, false, now), ChunkOutcome::Ack(5));
        assert!(matches!(up.chunk(3, 6, "c.png", 10, 0, &data, false, now), ChunkOutcome::Refused(m) if m.contains("at once")));
        // A client's disconnect drops what it was sending; an idle one expires.
        up.drop_client(1);
        assert_eq!(up.len(), 1);
        up.expire(now + IDLE_DROP_AFTER);
        assert_eq!(up.len(), 0);
    }

    #[test]
    fn a_client_sends_in_a_window_resends_after_a_lost_ack_and_batches_its_links() {
        let now = Instant::now();
        let mut big = png();
        big.resize(CHUNK_BYTES * (ACK_WINDOW + 2), 1);
        let mut c = ClientUploads::default();
        let (msgs, notes) = c.add_batch(vec![("a.png".into(), big.clone()), ("b.png".into(), png())], now);
        assert!(notes.is_empty());
        assert_eq!(msgs.len(), ACK_WINDOW);
        let id = match &msgs[0] {
            WsMessage::PictureUpload { id, offset: 0, .. } => *id,
            other => panic!("{:?}", other),
        };
        // An ack opens the window by one chunk.
        let more = c.on_ack(id, CHUNK_BYTES as u64, now);
        assert_eq!(more.len(), 1);
        // No ack for RESEND_AFTER: everything from the last ack again.
        let (resent, _) = c.tick(now + RESEND_AFTER);
        assert!(matches!(&resent[0], WsMessage::PictureUpload { offset, .. } if *offset == CHUNK_BYTES as u64));
        // The server says it has it all; the rest goes and then the answer.
        let rest = c.on_ack(id, big.len() as u64, now);
        assert!(rest.is_empty());
        let (next, notes, mine) = c.on_uploaded(id, Some("https://h/a.png".into()), None, now);
        assert!(mine && notes.is_empty());
        let id2 = match &next[0] {
            WsMessage::PictureUpload { id, last: true, .. } => *id,
            other => panic!("{:?}", other),
        };
        assert_ne!(id, id2);
        // Another window's answer isn't this one's.
        assert!(!c.on_uploaded(id2 + 1000, None, None, now).2);
        let (_, notes, _) = c.on_uploaded(id2, Some("https://h/b.png".into()), None, now);
        assert_eq!(notes, vec![ClientNote::Insert("https://h/a.png https://h/b.png".into())]);
        assert!(!c.is_busy());

        // A failure is a line, and the batch's other links still go in.
        let (msgs, _) = c.add_batch(vec![("x.png".into(), png()), ("y.png".into(), png())], now);
        let idx = match &msgs[0] { WsMessage::PictureUpload { id, .. } => *id, _ => unreachable!() };
        let (msgs, notes, _) = c.on_uploaded(idx, None, Some("HTTP 500".into()), now);
        assert_eq!(notes, vec![ClientNote::Line("Couldn't upload x.png: HTTP 500".into())]);
        let idy = match &msgs[0] { WsMessage::PictureUpload { id, .. } => *id, _ => unreachable!() };
        let (_, notes, _) = c.on_uploaded(idy, Some("https://h/y.png".into()), None, now);
        assert_eq!(notes, vec![ClientNote::Insert("https://h/y.png".into())]);

        // No answer at all: given up on, with a line.
        let (_, _) = c.add_batch(vec![("z.png".into(), png())], now);
        let (_, notes) = c.tick(now + GIVE_UP_AFTER);
        assert_eq!(notes, vec![ClientNote::Line("Couldn't upload z.png: the server didn't answer.".into())]);
        assert!(!c.is_busy());

        // A lost connection ends everything.
        let _ = c.add_batch(vec![("w.png".into(), png())], now);
        assert_eq!(c.connection_lost(), vec![ClientNote::Line("Picture upload interrupted (connection lost).".into())]);
        assert!(!c.is_busy());
    }

    /// The upload's request, against a local fake host.
    #[tokio::test]
    async fn an_upload_posts_the_picture_and_returns_the_link() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let port = crate::testserver::free_port();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 4096];
            // Headers, then Content-Length bytes of body.
            let want = loop {
                let n = sock.read(&mut buf).await.unwrap();
                assert!(n > 0, "closed early");
                req.extend_from_slice(&buf[..n]);
                if let Some(end) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&req[..end]).to_ascii_lowercase();
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map(|v| v.trim().parse().unwrap())
                        .expect("a content length");
                    break end + 4 + len;
                }
            };
            while req.len() < want {
                let n = sock.read(&mut buf).await.unwrap();
                assert!(n > 0, "closed early");
                req.extend_from_slice(&buf[..n]);
            }
            let body = "https://files.example/abc.png\n";
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            sock.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&req).to_string()
        });
        let endpoint = format!("http://127.0.0.1:{}/", port);
        let got = upload_at(&endpoint, PictureService::default(), png(), sniff(&png()).unwrap(), Vec::new()).await;
        assert_eq!(got, Ok(Uploaded { url: "https://files.example/abc.png".to_string(), note: None }));
        let req = server.await.unwrap();
        assert!(req.contains("name=\"reqtype\""), "{}", req);
        assert!(req.contains("fileupload"));
        assert!(req.contains("name=\"fileToUpload\"; filename=\"picture.png\""));
        assert!(req.contains("image/png"));
        assert!(req.contains("Clay/"));
    }
}
