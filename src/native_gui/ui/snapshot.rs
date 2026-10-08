//! `clay --native-gui=host --native-gui-snapshot=out.png`: connect, collect what the server
//! sends, render the main window offscreen with GPUI's headless renderer and save it as a
//! PNG. For comparing the native GUI's rendering against the WebView without opening a
//! window on anyone's desktop (`native-gui-snapshot` feature; Linux and macOS, where GPUI
//! has a headless renderer).

use std::time::{Duration, Instant};

use futures::channel::mpsc;
use gpui::{AppContext, HeadlessAppContext, px, size};

use super::{ClayView, fonts};
use crate::native_gui::core::client::{Input, Status};
use crate::native_gui::core::session::Session;

/// How long the output must be quiet, after the initial state, before the snapshot.
const SETTLE: Duration = Duration::from_millis(1500);
const TIMEOUT: Duration = Duration::from_secs(20);
/// How long to wait for Discord emoji images.
const EMOJI_TIMEOUT: Duration = Duration::from_secs(15);

pub fn run(mut session: Session, mut events: mpsc::UnboundedReceiver<Input>, out: &str, width: f32, height: f32) {
    // Run the session here until the output has settled: the headless app runs on a
    // deterministic test executor, so hand it a finished state rather than a live channel.
    let start = Instant::now();
    let mut last = Instant::now();
    let _ = session.start();
    loop {
        match events.try_recv() {
            Ok(ev) => {
                if let Input::Closed(reason) = &ev {
                    eprintln!("snapshot: connection closed: {reason}");
                }
                let _ = session.handle(ev);
                last = Instant::now();
            }
            Err(mpsc::TryRecvError::Closed) => break,
            Err(mpsc::TryRecvError::Empty) => {
                let _ = session.tick();
                match session.core.status() {
                    Status::NeedLogin { .. } => fail("the server wants a login: set CLAY_PASSWORD"),
                    Status::CertMismatch(m) => fail(&format!("the certificate of {} has changed", m.host)),
                    Status::Stopped(why) => fail(why),
                    _ => {}
                }
                if session.core.is_online() && !session.core.worlds.is_empty() && last.elapsed() >= SETTLE {
                    break;
                }
                if start.elapsed() >= TIMEOUT {
                    fail("timed out waiting for the server");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, std::sync::Arc::new(()), gpui_platform::current_headless_renderer);
    let fonts = cx.update(|app| {
        gpui_component::init(app);
        #[cfg(not(target_os = "windows"))]
        app.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
        fonts::load(app)
    });
    let window = cx
        .open_window(size(px(width), px(height)), |window, app| app.new(|cx| ClayView::new(fonts, session, window, cx)))
        .expect("open the offscreen window");
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

fn fail(why: &str) -> ! {
    eprintln!("snapshot: {why}");
    std::process::exit(1);
}
