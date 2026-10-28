//! `clay --gui=host --gui-snapshot=out.png`: connect, collect what the server
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
    // CLAY_SNAPSHOT_CACHE=1: a cold start as the real window makes one, from the world
    // cache (loaded here, and written after the capture as the real window writes it).
    let use_cache = std::env::var_os("CLAY_SNAPSHOT_CACHE").is_some();
    let cache_key = session.target.key();
    if use_cache {
        session.core.set_cache(crate::native_gui::core::cache::load(&cache_key));
        session.core.prefer_world(crate::native_gui::core::store::server(&cache_key).last_world);
    }
    // CLAY_SNAPSHOT_SETTLE=ms: how long the output must be quiet before the view takes
    // over. 0 hands the view the session as soon as the initial state is in, so what
    // follows (a cold start's catch-up) reaches it as it does the real window: `pump:`.
    let settle = std::env::var("CLAY_SNAPSHOT_SETTLE").ok().and_then(|s| s.parse().ok()).map_or(SETTLE, Duration::from_millis);
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
                if session.core.is_online() && !session.core.worlds.is_empty() && last.elapsed() >= settle {
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
                // CLAY_SNAPSHOT_GREP=pattern: draw as a /window --grep window would.
                if let Ok(p) = std::env::var("CLAY_SNAPSHOT_GREP") {
                    view.mode.grep = Some(super::viewmode::Grep::new(&p, false));
                }
                // As the real window with the connection: it plays the sounds (and
                // speaks), so a /tts or ServerSpeak step goes through the same path.
                view.media = Some(crate::native_gui::core::media::Media::new(super::emoji::runtime()));
                view.warm_kokoro_if_wanted();
                // The session came online before the view existed: take its world as shown,
                // as the real window does when the InitialState arrives (else the first
                // change after this looks like a world switch and moves the view).
                view.follow_world();
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
    let window_kind = |kind: &str| match kind {
        "settings" => super::windows::Kind::Settings,
        "help" => super::windows::Kind::Help,
        "actions" => super::windows::Kind::Actions,
        "worlds" => super::windows::Kind::Worlds,
        "notes" => super::windows::Kind::Notes(0),
        "theme" => super::windows::Kind::ThemeEditor,
        "keybind" => super::windows::Kind::KeybindEditor,
        "action" => super::windows::Kind::ActionEditor,
        "emoji" => super::windows::Kind::Emoji,
        other => fail(&format!("no window kind {other}")),
    };
    // CLAY_SNAPSHOT_INPUT: mouse and key events to replay before the capture, for
    // checking selection, menus and dialogs, e.g. "down:40,60;move:300,90;up:300,90;key:ctrl-c".
    if let Ok(script) = std::env::var("CLAY_SNAPSHOT_INPUT") {
        let mut left_down = false;
        // `win:help` sends what follows to that window (`win:main` back).
        let mut target: gpui::AnyWindowHandle = window.into();
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
                // Files dropped onto the window as the platform delivers them:
                // `drop:x,y|/path/one|/path/two` (Import Picture).
                "drop" => {
                    let mut parts = arg.split('|');
                    let xy: Vec<f32> = parts.next().unwrap_or("").split(',').filter_map(|n| n.trim().parse().ok()).collect();
                    let position = gpui::point(px(xy.first().copied().unwrap_or(0.0)), px(xy.get(1).copied().unwrap_or(0.0)));
                    let paths = gpui::ExternalPaths(parts.map(std::path::PathBuf::from).collect());
                    vec![
                        gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Entered { position, paths }),
                        gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Pending { position }),
                        gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Submit { position }),
                    ]
                }
                // The main window's input line and cursor, as text.
                "inputline" => {
                    cx.run_until_parked();
                    cx.update(|app| {
                        let v = view.read(app);
                        let last: Vec<String> = v.session.core.current_world().map_or(Vec::new(), |w| {
                            w.lines.iter().rev().take(3).map(|l| super::find::plain(&l.text)).collect()
                        });
                        let sent: Vec<String> = v.input.sent.iter().map(|e| e.text.clone()).collect();
                        println!("snapshot-input: {arg} text={:?} cursor={} last_lines={:?} picture_upload={} sent={:?} sent_rows={}", v.editor.text, v.editor.cursor, last, v.session.core.server_picture_upload, sent, v.input.sent_rows);
                    });
                    Vec::new()
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
                // Keys typed into whatever has the focus (a dialog's fields), one at a time,
                // as a keyboard without an input method sends them.
                "text" => {
                    for ch in arg.chars() {
                        let name = if ch == ' ' { "space".to_string() } else { ch.to_string() };
                        let keystroke = gpui::Keystroke::parse(&name).unwrap_or_else(|e| fail(&format!("bad key {ch}: {e}")));
                        cx.update_window(target, |_, w, app| {
                            w.dispatch_keystroke(keystroke, app);
                        })
                        .expect("type a key");
                        cx.run_until_parked();
                        cx.update_window(target, |_, w, app| {
                            let _ = w.draw(app);
                        })
                        .expect("draw after a key");
                    }
                    Vec::new()
                }
                "win" => {
                    target = if arg == "main" {
                        window.into()
                    } else if arg == "view" {
                        // The first window following this one (`/window`, `--grep`, New Window).
                        cx.update(|app| view.read(app).link.follower_window(0)).unwrap_or_else(|| fail("no /window is open"))
                    } else {
                        let kind = window_kind(arg);
                        let handle = cx.update(|app| view.read(app).windows.handle(kind)).unwrap_or_else(|| fail("that window isn't open (add an open: step)"));
                        // The list keeps a window's handle after it closes (its own Close,
                        // Save in the notes window): a closed one can't be updated.
                        if cx.update_window(handle, |_, _, _| ()).is_err() {
                            fail("that window has closed");
                        }
                        handle
                    };
                    Vec::new()
                }
                // A dialog by name (surfaces.rs `snapshot_open`).
                "open" => {
                    let what = arg.to_string();
                    cx.update_window(window.into(), |_, w, app| view.update(app, |v, cx| v.snapshot_open(&what, w, cx))).expect("open");
                    cx.run_until_parked();
                    cx.update_window(window.into(), |_, w, app| {
                        let _ = w.draw(app);
                    })
                    .expect("draw after open");
                    Vec::new()
                }
                // The target window's close button: the Action editor asks first when it has
                // changes (what its `on_window_should_close` hook answers); others just close.
                "close" => {
                    use super::windows::{Kind, action_editor::ActionEditor};
                    let guard = cx.update(|app| {
                        let v = view.read(app);
                        if v.windows.handle(Kind::ActionEditor) != Some(target) {
                            return None;
                        }
                        v.windows.frame(Kind::ActionEditor).and_then(|f| f.read(app).content::<ActionEditor>())
                    });
                    let _ = cx.update_window(target, |_, w, app| {
                        if guard.is_none_or(|e| e.update(app, |e, cx| e.may_close(w, cx))) {
                            w.remove_window();
                        }
                    });
                    cx.run_until_parked();
                    let _ = cx.update_window(target, |_, w, app| {
                        let _ = w.draw(app);
                    });
                    Vec::new()
                }
                // Real time with the connection live: the server's messages reach the view
                // (round trips: a Save, a reply, a window that opens on one).
                "pump" => {
                    let until = Instant::now() + Duration::from_millis(nums.first().copied().unwrap_or(500.0) as u64);
                    while Instant::now() < until {
                        while let Ok(ev) = events.try_recv() {
                            cx.update(|app| view.update(app, |v, cx| v.handle(ev, cx)));
                        }
                        cx.run_until_parked();
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let _ = cx.update_window(target, |_, w, app| {
                        let _ = w.draw(app);
                    });
                    Vec::new()
                }
                // The main window's output as last drawn, as text: more-mode's counts, the
                // scroll, and the first and last rows painted (by seq), for measuring how
                // far a key moved the view.
                "state" => {
                    cx.run_until_parked();
                    let _ = cx.update_window(window.into(), |_, w, app| {
                        let _ = w.draw(app);
                    });
                    cx.update(|app| {
                        let v = view.read(app);
                        let s = v.scroll.borrow();
                        let world = v.session.core.current_world();
                        let row = |id: u64| {
                            let line = world.and_then(|w| w.lines.iter().find(|l| l.id == id));
                            let seq = line.and_then(|l| l.seq).map_or("-".to_string(), |s| s.to_string());
                            let text: String = line.map(|l| super::find::plain(&l.text)).unwrap_or_default().chars().take(24).collect();
                            format!("{seq}:{text:?}")
                        };
                        let distinct = s.hits.iter().map(|h| h.id).collect::<std::collections::HashSet<_>>().len();
                        let bar = s.bar.map_or("none".to_string(), |b| {
                            format!("x={:.0} y={:.0}+{:.0} thumb={:.0}+{:.0} total={} hot={}", b.left, b.top, b.height, b.thumb_top, b.thumb_h, b.total, v.select.bar_hot())
                        });
                        println!(
                            "snapshot-state: {arg} status={:?} conn_log={} lines={} held={} pending={} paused={} follow={} rows_below={} view_rows={} painted_rows={} painted_lines={} first={} last={} bar[{bar}]",
                            v.session.core.status(),
                            v.surf.stack.is_open(&super::dialogs::Kind::ConnectionLog),
                            world.map_or(0, |w| w.lines.len()),
                            v.held_here(),
                            v.pending_total(),
                            v.more_paused(),
                            s.follow,
                            s.rows_below,
                            s.view_rows,
                            s.hits.len(),
                            distinct,
                            s.hits.first().map_or("none".into(), |h| row(h.id)),
                            s.hits.last().map_or("none".into(), |h| row(h.id)),
                        );
                    });
                    Vec::new()
                }
                // The world each window shows (the main one, then each `/window`), and
                // each world's unseen count as that window knows it.
                "worlds" => {
                    cx.run_until_parked();
                    cx.update(|app| {
                        let describe = |v: &ClayView| {
                            let core = &v.session.core;
                            let unseen: Vec<String> = core.worlds.iter().filter(|w| w.unseen > 0).map(|w| format!("{}={}", w.name, w.unseen)).collect();
                            format!("{} (unseen: {})", core.current_world().map_or("-", |w| w.name.as_str()), unseen.join(" "))
                        };
                        let main = view.read(app);
                        let mut line = format!("snapshot-worlds: {arg} main={}", describe(main));
                        for (i, f) in main.link.follower_views().iter().enumerate() {
                            if let Some(f) = f.upgrade() {
                                line.push_str(&format!(" view{i}={}", describe(f.read(app))));
                            }
                        }
                        println!("{line}");
                    });
                    Vec::new()
                }
                // Text Reveal's New Only: each world's seen mark and how many of its lines
                // are newer than it (what a switch to the world types on).
                "reveal" => {
                    cx.run_until_parked();
                    cx.update(|app| {
                        let v = view.read(app);
                        let core = &v.session.core;
                        let worlds: Vec<String> = core.worlds.iter().map(|w| {
                            let seen = v.textdiv.seen(&w.name);
                            let newer = w.lines.iter().filter(|l| !l.from_archive && l.seq.is_some_and(|s| seen.is_none_or(|m| s > m))).count();
                            format!("{}[lines={} seen={} newer={}]", w.name, w.lines.len(), seen.map_or("-".to_string(), |s| s.to_string()), newer)
                        }).collect();
                        println!(
                            "snapshot-reveal: {arg} current={} new_only={} {}",
                            core.current_world().map_or("-", |w| w.name.as_str()),
                            v.reveal_new_only,
                            worlds.join(" ")
                        );
                    });
                    Vec::new()
                }
                // A capture of the main window now, as well as the one at the end.
                "shot" => {
                    cx.run_until_parked();
                    let _ = cx.update_window(window.into(), |_, w, app| {
                        let _ = w.draw(app);
                    });
                    match cx.capture_screenshot(window.into()) {
                        Ok(image) => match image.save(arg) {
                            Ok(()) => println!("snapshot: {arg} ({}x{})", image.width(), image.height()),
                            Err(e) => fail(&format!("can't write {arg}: {e}")),
                        },
                        Err(e) => fail(&format!("rendering failed: {e:#}")),
                    }
                    Vec::new()
                }
                "sleep" => {
                    std::thread::sleep(Duration::from_millis(nums.first().copied().unwrap_or(0.0) as u64));
                    Vec::new()
                }
                // The connection and its dialog as they stand, without drawing (`state:`
                // draws first, which runs what the view queued for its next frame).
                "peek" => {
                    cx.update(|app| {
                        let v = view.read(app);
                        println!(
                            "snapshot-peek: {arg} status={:?} conn_log={} queued_for_next_frame={}",
                            v.session.core.status(),
                            v.surf.stack.is_open(&super::dialogs::Kind::ConnectionLog),
                            v.surf.later.len()
                        );
                    });
                    Vec::new()
                }
                // The view's clock, once (the real window's 250 ms timer, which this
                // harness doesn't run): keepalives, watchdogs, a reconnect that's due.
                "tick" => {
                    cx.update_window(window.into(), |_, w, app| view.update(app, |v, cx| v.tick(w, cx))).expect("tick");
                    cx.run_until_parked();
                    Vec::new()
                }
                "wait" => {
                    cx.advance_clock(Duration::from_millis(nums.first().copied().unwrap_or(0.0) as u64));
                    Vec::new()
                }
                other => fail(&format!("unknown input step {other}")),
            };
            for ev in events {
                cx.update_window(target, |_, window, app| {
                    let _ = window.dispatch_event(ev, app);
                })
                .expect("dispatch an event");
                cx.run_until_parked();
                // A click can close the window it was in (Close, Ok): that's fine.
                let _ = cx.update_window(target, |_, window, app| {
                    let _ = window.draw(app);
                });
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
    // CLAY_SNAPSHOT_WINDOW=help (etc.): photograph that window, opened by an `open:` step,
    // instead of the main one.
    let target: gpui::AnyWindowHandle = match std::env::var("CLAY_SNAPSHOT_WINDOW").ok().filter(|k| !k.is_empty()).as_deref() {
        Some(kind) => {
            let handle = if kind == "view" {
                cx.update(|app| view.read(app).link.follower_window(0)).unwrap_or_else(|| fail("no /window is open"))
            } else {
                let kind = window_kind(kind);
                cx.update(|app| view.read(app).windows.handle(kind)).unwrap_or_else(|| fail("that window isn't open (add an open: step)"))
            };
            cx.run_until_parked();
            cx.update_window(handle, |_, w, app| {
                let _ = w.draw(app);
            })
            .expect("draw the window");
            handle
        }
        None => {
            // Events sent to another window can change this one (a font preview).
            cx.run_until_parked();
            cx.update_window(window.into(), |_, w, app| {
                let _ = w.draw(app);
            })
            .expect("draw the main window");
            window.into()
        }
    };
    match cx.capture_screenshot(target) {
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
    if use_cache {
        let worlds = cx.update(|app| view.read(app).session.core.cache_snapshot());
        if !worlds.is_empty() {
            crate::native_gui::core::cache::save(&cache_key, &worlds);
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
