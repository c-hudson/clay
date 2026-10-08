//! Discord custom emoji for the output view (app.js `convertDiscordEmojis`).
//!
//! The same image the WebView shows, from the same place: Discord's CDN, `.png`, or `.gif`
//! for an animated emoji. Each emoji is fetched once on the tokio runtime and cached for
//! the life of the process. Until it arrives, the output reserves a square for it; if it
//! can't be had (Discord doesn't know the id, no network), the output shows its name,
//! `:name:`, where the WebView shows a broken-image box.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::channel::mpsc;
use gpui::RenderImage;

use crate::native_gui::core::style::DiscordEmoji;

/// The image's height, in ems (app.js: `height: 1.2em; vertical-align: middle`).
pub const HEIGHT_EM: f32 = 1.2;

/// Larger answers are refused: Discord's emoji are at most 256 KiB.
const MAX_BYTES: usize = 4 << 20;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Decoder limits: a hostile or broken file must not exhaust memory.
const MAX_SIDE: u32 = 1024;
const MAX_DECODED: u64 = 64 << 20;
const MAX_FRAMES: usize = 500;

/// A fetched emoji, ready to paint.
pub struct Loaded {
    pub image: Arc<RenderImage>,
    /// Width over height of the first frame.
    pub aspect: f32,
    /// Per frame, how long it shows; empty for a still image.
    delays: Vec<Duration>,
    total: Duration,
}

impl Loaded {
    /// The frame to show at `now`, and when the next one is due (None for a still image).
    /// Every copy of an emoji animates in step, timed from when the process started.
    pub fn frame_at(&self, now: Instant) -> (usize, Option<Instant>) {
        if self.delays.len() < 2 || self.total.is_zero() {
            return (0, None);
        }
        let total = self.total.as_micros();
        let mut t = now.saturating_duration_since(*EPOCH).as_micros() % total;
        for (i, d) in self.delays.iter().enumerate() {
            let d = d.as_micros();
            if t < d {
                return (i, Some(now + Duration::from_micros((d - t) as u64)));
            }
            t -= d;
        }
        (0, None)
    }
}

#[derive(Clone)]
pub enum Look {
    Loading,
    Ready(Arc<Loaded>),
    Failed,
}

type Key = (String, bool);

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
static CACHE: LazyLock<Mutex<HashMap<Key, Look>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();
static REPAINT: Mutex<Option<mpsc::UnboundedSender<()>>> = Mutex::new(None);
static WAKE: Mutex<Option<Instant>> = Mutex::new(None);

/// The runtime fetches run on. Without it every emoji shows as its name.
pub fn init(runtime: tokio::runtime::Handle) {
    let _ = RUNTIME.set(runtime);
    LazyLock::force(&EPOCH);
}

/// Where to say "draw again": an emoji arrived, or an animation moved on.
pub fn set_repaint(tx: mpsc::UnboundedSender<()>) {
    *REPAINT.lock().unwrap() = Some(tx);
}

fn repaint() {
    if let Some(tx) = REPAINT.lock().unwrap().as_ref() {
        let _ = tx.unbounded_send(());
    }
}

/// How the emoji stands now; the first time it's asked for, its fetch starts.
pub fn lookup(e: &DiscordEmoji) -> Look {
    let key = (e.id.clone(), e.animated);
    let mut cache = CACHE.lock().unwrap();
    if let Some(look) = cache.get(&key) {
        return look.clone();
    }
    let Some(rt) = RUNTIME.get() else {
        cache.insert(key, Look::Failed);
        return Look::Failed;
    };
    cache.insert(key.clone(), Look::Loading);
    let url = format!("https://cdn.discordapp.com/emojis/{}.{}", e.id, if e.animated { "gif" } else { "png" });
    rt.spawn(async move {
        let look = match fetch(&url).await {
            Ok(bytes) => match tokio::task::spawn_blocking(move || decode(&bytes)).await {
                Ok(Ok(loaded)) => Look::Ready(Arc::new(loaded)),
                Ok(Err(e)) => {
                    crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: emoji {url}: {e}"));
                    Look::Failed
                }
                Err(_) => Look::Failed,
            },
            Err(e) => {
                crate::debug_log(crate::is_debug_enabled(), &format!("native-gui: emoji {url}: {e}"));
                Look::Failed
            }
        };
        CACHE.lock().unwrap().insert(key, look);
        repaint();
    });
    Look::Loading
}

/// How many emoji are still on their way (the offscreen snapshot waits for them).
#[cfg(feature = "native-gui-snapshot")]
pub fn pending() -> usize {
    CACHE.lock().unwrap().values().filter(|l| matches!(l, Look::Loading)).count()
}

/// Ask for a repaint at `at` (an animated emoji's next frame). One wake is kept: the
/// earliest still to come.
pub fn wake_at(at: Instant) {
    let Some(rt) = RUNTIME.get() else { return };
    {
        let mut wake = WAKE.lock().unwrap();
        if wake.is_some_and(|w| w <= at && w > Instant::now()) {
            return;
        }
        *wake = Some(at);
    }
    rt.spawn(async move {
        tokio::time::sleep_until(at.into()).await;
        {
            let mut wake = WAKE.lock().unwrap();
            if *wake == Some(at) {
                *wake = None;
            }
        }
        repaint();
    });
}

async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder().timeout(FETCH_TIMEOUT).build().unwrap_or_else(|_| reqwest::Client::new())
    });
    let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err("too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// PNG or GIF (by content, not by the URL's extension) into GPUI's BGRA frames.
fn decode(bytes: &[u8]) -> Result<Loaded, String> {
    use image::{AnimationDecoder, ImageDecoder, ImageFormat, Limits};
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_DECODED);

    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    let mut frames: Vec<image::Frame> = Vec::new();
    let mut delays = Vec::new();
    if format == ImageFormat::Gif {
        let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        decoder.set_limits(limits).map_err(|e| e.to_string())?;
        let mut decoded: u64 = 0;
        for frame in decoder.into_frames().take(MAX_FRAMES) {
            let frame = frame.map_err(|e| e.to_string())?;
            decoded += frame.buffer().as_raw().len() as u64;
            if decoded > MAX_DECODED {
                break;
            }
            // Browsers show a frame of 10 ms or less for 100 ms (many GIFs say 0).
            let (num, den) = frame.delay().numer_denom_ms();
            let ms = if den == 0 { 0.0 } else { num as f64 / den as f64 };
            delays.push(Duration::from_micros(if ms <= 10.0 { 100_000 } else { (ms * 1000.0) as u64 }));
            frames.push(frame);
        }
    } else {
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits);
        let image = reader.decode().map_err(|e| e.to_string())?;
        frames.push(image::Frame::new(image.into_rgba8()));
    }
    let Some(first) = frames.first() else { return Err("no frames".into()) };
    let (w, h) = first.buffer().dimensions();
    if w == 0 || h == 0 {
        return Err("empty image".into());
    }
    for frame in &mut frames {
        for px in frame.buffer_mut().chunks_exact_mut(4) {
            px.swap(0, 2); // RGBA -> BGRA, as GPUI's atlas takes it
        }
    }
    if frames.len() < 2 {
        delays.clear();
    }
    let total = delays.iter().sum();
    Ok(Loaded { image: Arc::new(RenderImage::new(frames)), aspect: w as f32 / h as f32, delays, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(delays_ms: &[u64]) -> Loaded {
        let delays: Vec<Duration> = delays_ms.iter().map(|&d| Duration::from_millis(d)).collect();
        let total = delays.iter().sum();
        Loaded { image: Arc::new(RenderImage::new(Vec::<image::Frame>::new())), aspect: 1.0, delays, total }
    }

    #[test]
    fn frames_follow_their_delays_and_loop() {
        let l = loaded(&[100, 50, 200]);
        let at = |ms: u64| l.frame_at(*EPOCH + Duration::from_millis(ms));
        assert_eq!(at(0).0, 0);
        assert_eq!(at(99).0, 0);
        assert_eq!(at(100).0, 1);
        assert_eq!(at(149).0, 1);
        assert_eq!(at(150).0, 2);
        assert_eq!(at(350).0, 0);
        assert_eq!(at(120).1, Some(*EPOCH + Duration::from_millis(150)));
    }

    #[test]
    fn a_still_image_never_asks_for_another_frame() {
        assert_eq!(loaded(&[]).frame_at(Instant::now()), (0, None));
    }

    #[test]
    fn decodes_png_and_gif_into_bgra() {
        // 1x1 red PNG.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 1, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let l = decode(&png).unwrap();
        assert_eq!(l.aspect, 2.0);
        assert_eq!(l.image.frame_count(), 1);
        assert_eq!(&l.image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        assert!(decode(b"not an image").is_err());
    }
}
