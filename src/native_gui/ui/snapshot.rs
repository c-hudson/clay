//! `clay --native-gui=host --native-gui-snapshot=out.png`: connect, collect what the server
//! sends, render the main window offscreen with GPUI's headless renderer and save it as a
//! PNG. For comparing the native GUI's rendering against the WebView without opening a
//! window on anyone's desktop (`native-gui-snapshot` feature; Linux and macOS, where GPUI
//! has a headless renderer).

use std::time::{Duration, Instant};

use futures::channel::mpsc;
use gpui::{AppContext, HeadlessAppContext, px, size};

use super::{ClayView, fonts};
use crate::native_gui::core::net::{NetEvent, Sender};
use crate::websocket::WsMessage;

/// How long the output must be quiet, after the initial state, before the snapshot.
const SETTLE: Duration = Duration::from_millis(1500);
const TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for Discord emoji images.
const EMOJI_TIMEOUT: Duration = Duration::from_secs(15);

pub fn run(sender: Sender, mut events: mpsc::UnboundedReceiver<NetEvent>, out: &str, width: f32, height: f32) {
    // Collect the session's events first: the headless app runs on a deterministic test
    // executor, so feed it everything at once rather than pumping a live channel.
    let start = Instant::now();
    let mut collected = Vec::new();
    let mut have_state = false;
    let mut last = Instant::now();
    loop {
        match events.try_recv() {
            Ok(ev) => {
                if let NetEvent::Message(m) = &ev {
                    have_state |= matches!(**m, WsMessage::InitialState { .. });
                }
                if let NetEvent::Closed(reason) = &ev {
                    eprintln!("snapshot: connection closed: {reason}");
                }
                collected.push(ev);
                last = Instant::now();
            }
            Err(mpsc::TryRecvError::Closed) => break,
            Err(mpsc::TryRecvError::Empty) => {
                if have_state && last.elapsed() >= SETTLE {
                    break;
                }
                if start.elapsed() >= TIMEOUT {
                    eprintln!("snapshot: timed out waiting for the server");
                    std::process::exit(1);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, std::sync::Arc::new(()), gpui_platform::current_headless_renderer);
    let fonts = cx.update(|app| {
        #[cfg(not(target_os = "windows"))]
        app.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
        fonts::load(app)
    });
    let window = cx
        .open_window(size(px(width), px(height)), |_, app| app.new(|cx| ClayView::new(fonts, sender, cx)))
        .expect("open the offscreen window");
    cx.update(|app| {
        window
            .update(app, |view, _, cx| {
                for ev in collected {
                    view.handle(ev);
                }
                cx.notify();
            })
            .expect("update the view");
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, app| {
        let _ = window.draw(app);
    })
    .expect("draw the frame");
    // That frame asked for any Discord emoji it shows: wait for them, then draw again.
    if super::emoji::pending() > 0 {
        let start = Instant::now();
        while super::emoji::pending() > 0 && start.elapsed() < EMOJI_TIMEOUT {
            std::thread::sleep(Duration::from_millis(50));
        }
        cx.update(|app| window.update(app, |_, _, cx| cx.notify()).expect("update the view"));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, app| {
            let _ = window.draw(app);
        })
        .expect("draw the frame again");
    }
    match cx.capture_screenshot(window.into()) {
        Ok(image) => {
            if let Err(e) = image.save(out) {
                eprintln!("snapshot: can't write {out}: {e}");
                std::process::exit(1);
            }
            println!("snapshot: {} ({}x{})", out, image.width(), image.height());
        }
        Err(e) => {
            eprintln!("snapshot: rendering failed: {e:#}");
            std::process::exit(1);
        }
    }
}
