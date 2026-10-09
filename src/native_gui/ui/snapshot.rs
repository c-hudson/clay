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
    let mut cx = HeadlessAppContext::with_platform(text_system, std::sync::Arc::new(super::assets::Assets), gpui_platform::current_headless_renderer);
    let fonts = cx.update(|app| {
        gpui_component::init(app);
        super::bind_main_keys(app);
        #[cfg(not(target_os = "windows"))]
        app.set_text_rendering_mode(gpui::TextRenderingMode::Grayscale);
        fonts::load(app)
    });
    let mut main = None;
    let window = cx
        .open_window(size(px(width), px(height)), |window, app| {
            let view = app.new(|cx| {
                let mut view = ClayView::new(fonts, session, window, cx);
                // CLAY_SNAPSHOT_MENU=main|worlds: draw with that menu open, for reviewing it.
                view.chrome.menu = match std::env::var("CLAY_SNAPSHOT_MENU").as_deref() {
                    Ok("main") => Some(super::chrome::MenuKind::Main),
                    Ok("worlds") => Some(super::chrome::MenuKind::Worlds),
                    _ => None,
                };
                window.focus(&view.focus.clone(), cx);
                view
            });
            main = Some(view.clone());
            app.new(|cx| gpui_component::Root::new(view, window, cx))
        })
        .expect("open the offscreen window");
    let view = main.expect("the main view");
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
        cx.update(|app| view.update(app, |_, cx| cx.notify()));
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, app| {
            let _ = window.draw(app);
        })
        .expect("draw the frame again");
    }
    // CLAY_SNAPSHOT_INPUT: mouse and key events to replay before the capture, for
    // checking selection, menus and dialogs, e.g. "down:40,60;move:300,90;up:300,90;key:ctrl-c".
    if let Ok(script) = std::env::var("CLAY_SNAPSHOT_INPUT") {
        let mut left_down = false;
        for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (verb, arg) = step.split_once(':').unwrap_or((step, ""));
            let nums: Vec<f32> = arg.split(',').filter_map(|n| n.trim().parse().ok()).collect();
            let at = || gpui::point(px(nums.first().copied().unwrap_or(0.0)), px(nums.get(1).copied().unwrap_or(0.0)));
            let mods = gpui::Modifiers::default();
            let events: Vec<gpui::PlatformInput> = match verb {
                "down" | "rdown" => {
                    let button = if verb == "down" { gpui::MouseButton::Left } else { gpui::MouseButton::Right };
                    left_down |= verb == "down";
                    let click_count = nums.get(2).map_or(1, |n| *n as usize);
                    vec![gpui::PlatformInput::MouseDown(gpui::MouseDownEvent { button, position: at(), modifiers: mods, click_count, first_mouse: false })]
                }
                "move" | "hover" => vec![gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                    position: at(),
                    pressed_button: (verb == "move" && left_down).then_some(gpui::MouseButton::Left),
                    modifiers: mods,
                })],
                "up" => {
                    left_down = false;
                    vec![gpui::PlatformInput::MouseUp(gpui::MouseUpEvent { button: gpui::MouseButton::Left, position: at(), modifiers: mods, click_count: 1 })]
                }
                "key" => match gpui::Keystroke::parse(arg) {
                    Ok(keystroke) => vec![gpui::PlatformInput::KeyDown(gpui::KeyDownEvent { keystroke, is_held: false, prefer_character_input: false })],
                    Err(e) => fail(&format!("bad key {arg}: {e}")),
                },
                // Text as the platform delivers it: to a focused Find/Search field, else
                // to the input line through its input handler; `compose` is an input
                // method's uncommitted text, `commit` its result.
                "type" | "compose" | "commit" => {
                    let text = arg.to_string();
                    cx.update_window(window.into(), |_, w, app| {
                        view.update(app, |v, cx| {
                            use gpui::EntityInputHandler;
                            if verb == "compose" {
                                v.replace_and_mark_text_in_range(None, &text, None, w, cx);
                            } else if verb == "commit" || !v.type_into_find(&text, w, cx) {
                                v.replace_text_in_range(None, &text, w, cx);
                            }
                        });
                    })
                    .expect("type");
                    Vec::new()
                }
                "sleep" => {
                    std::thread::sleep(Duration::from_millis(nums.first().copied().unwrap_or(0.0) as u64));
                    Vec::new()
                }
                "wait" => {
                    cx.advance_clock(Duration::from_millis(nums.first().copied().unwrap_or(0.0) as u64));
                    Vec::new()
                }
                other => fail(&format!("unknown input step {other}")),
            };
            for ev in events {
                cx.update_window(window.into(), |_, window, app| {
                    let _ = window.dispatch_event(ev, app);
                })
                .expect("dispatch an event");
                cx.run_until_parked();
                cx.update_window(window.into(), |_, window, app| {
                    let _ = window.draw(app);
                })
                .expect("draw after an event");
            }
        }
        if let Some(text) = cx.update(|app| app.read_from_clipboard()).and_then(|c| c.text()) {
            println!("snapshot-clipboard: {}", text.replace('\n', "\\n"));
        }
    }
    // CLAY_SNAPSHOT_BENCH=1: time frames scrolling the buffer and taking in 200 lines a
    // second, and report memory (the CPU side of the phase 2 performance targets).
    if std::env::var_os("CLAY_SNAPSHOT_BENCH").is_some() {
        bench(&mut cx, window.into(), &view);
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

/// Frame-time statistics, in milliseconds: mean, 95th percentile, worst.
fn stats(mut ms: Vec<f64>) -> (f64, f64, f64) {
    ms.sort_by(f64::total_cmp);
    let mean = ms.iter().sum::<f64>() / ms.len().max(1) as f64;
    let p95 = ms.get(ms.len() * 95 / 100).copied().unwrap_or(0.0);
    (mean, p95, ms.last().copied().unwrap_or(0.0))
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()))
        .map_or(0.0, |kb| kb / 1024.0)
}

fn bench(cx: &mut HeadlessAppContext, window: gpui::AnyWindowHandle, view: &gpui::Entity<ClayView>) {
    let draw = |cx: &mut HeadlessAppContext| -> f64 {
        let t = Instant::now();
        cx.update_window(window, |_, window, app| {
            let _ = window.draw(app);
        })
        .expect("draw");
        t.elapsed().as_secs_f64() * 1000.0
    };
    let with_view = |cx: &mut HeadlessAppContext, f: &mut dyn FnMut(&mut ClayView, &mut gpui::Context<ClayView>)| {
        cx.update(|app| view.update(app, |v, cx| f(v, cx)));
    };
    let mut lines = 0;
    with_view(cx, &mut |v, _| lines = v.session.core.current_world().map_or(0, |w| w.lines.len()));
    println!("bench: {lines} lines in the current world, RSS {:.0} MB", rss_mb());

    // Scrolling: three rows a frame (a wheel notch) up through the buffer, then a page at a
    // time back down, 600 frames.
    let mut ms = Vec::new();
    for k in 0..600 {
        let rows = if k < 400 { -3 } else { 30 };
        with_view(cx, &mut |v, cx| {
            v.user_scroll(rows);
            cx.notify();
        });
        cx.run_until_parked();
        ms.push(draw(cx));
    }
    let (mean, p95, worst) = stats(ms);
    println!("bench: scroll  frames {mean:.2} ms mean, {p95:.2} ms p95, {worst:.2} ms worst (60 fps = 16.7 ms)");

    // Incoming: 200 lines a second at 60 frames a second, following the bottom, 10 s.
    with_view(cx, &mut |v, _| v.scroll.borrow_mut().follow_bottom());
    let (mut ms, mut work) = (Vec::new(), 0.0);
    let (mut core_s, mut after_s) = (0.0f64, 0.0f64);
    let started = Instant::now();
    for frame in 0..600u64 {
        let due = ((frame + 1) * 200 / 60) - (frame * 200 / 60);
        with_view(cx, &mut |v, cx| {
            let (world_index, next) = (v.session.core.current, v.session.core.current_world().map_or(0, |w| w.seen.max_seen() + 1));
            for i in 0..due {
                let seq = next + i;
                let msg = crate::websocket::WsMessage::ServerData {
                    world_index,
                    data: format!("\x1b[1;32mbench\x1b[0m line {seq}: the quick brown fox jumps over the lazy dog, again and again until it wraps\n"),
                    is_viewed: true,
                    ts: 0,
                    from_server: true,
                    seq,
                    end_seq: Some(seq),
                    flush: false,
                    gagged: false,
                    archive_sourced: false,
                    highlight_colors: Vec::new(),
                };
                let t0 = Instant::now();
                let fx = v.session.handle(crate::native_gui::core::client::Input::Message(Box::new(msg)));
                let t1 = Instant::now();
                v.after(fx, cx);
                core_s += (t1 - t0).as_secs_f64();
                after_s += t1.elapsed().as_secs_f64();
            }
        });
        // (The test executor's own turn isn't counted: the real event loop has none.)
        cx.run_until_parked();
        let frame_ms = draw(cx);
        ms.push(frame_ms);
        work += frame_ms / 1000.0;
    }
    work += core_s + after_s;
    let (mean, p95, worst) = stats(ms);
    println!(
        "bench: incoming frames {mean:.2} ms mean, {p95:.2} ms p95, {worst:.2} ms worst; drawing every frame and handling the lines took {:.1}% of one core over 10 s of 200 lines/s (ran in {:.1} s)",
        work / 10.0 * 100.0,
        started.elapsed().as_secs_f64()
    );
    println!("bench: of which the client core {:.0} ms and the view's follow-up {:.0} ms", core_s * 1000.0, after_s * 1000.0);
    println!("bench: RSS {:.0} MB after", rss_mb());
}

fn fail(why: &str) -> ! {
    eprintln!("snapshot: {why}");
    std::process::exit(1);
}
