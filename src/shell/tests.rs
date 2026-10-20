//! Shell worlds: encoder, screen rendering, activity, and a real-PTY round trip.

use crate::shell::keys::{encode, mouse, text_bytes, wheel_arrows};
use crate::websocket::{ShellKey, ShellModeMsg};

fn key(name: &str) -> ShellKey {
    ShellKey { key: name.to_string(), ctrl: false, alt: false, shift: false }
}

#[test]
fn plain_and_control_characters() {
    let m = ShellModeMsg::default();
    assert_eq!(encode(&key("a"), &m), b"a");
    assert_eq!(encode(&ShellKey { key: "c".into(), ctrl: true, ..Default::default() }, &m), vec![0x03]);
    // Ctrl+Space and Ctrl+@ are NUL.
    assert_eq!(encode(&ShellKey { key: " ".into(), ctrl: true, ..Default::default() }, &m), vec![0x00]);
    // Ctrl+4..7 are the file/group separators (how crossterm delivers ^\ ^] ^^ ^_).
    assert_eq!(encode(&ShellKey { key: "4".into(), ctrl: true, ..Default::default() }, &m), vec![0x1c]);
    assert_eq!(encode(&ShellKey { key: "7".into(), ctrl: true, ..Default::default() }, &m), vec![0x1f]);
    // Alt prefixes with ESC.
    assert_eq!(encode(&ShellKey { key: "x".into(), alt: true, ..Default::default() }, &m), vec![0x1b, b'x']);
}

#[test]
fn named_keys_and_cursor_mode() {
    let mut m = ShellModeMsg::default();
    assert_eq!(encode(&key("Enter"), &m), b"\r");
    assert_eq!(encode(&key("Backspace"), &m), vec![0x7f]);
    assert_eq!(encode(&ShellKey { key: "Backspace".into(), ctrl: true, ..Default::default() }, &m), vec![0x08]);
    assert_eq!(encode(&ShellKey { key: "Tab".into(), shift: true, ..Default::default() }, &m), b"\x1b[Z");
    assert_eq!(encode(&key("Up"), &m), b"\x1b[A");
    m.app_cursor = true;
    assert_eq!(encode(&key("Up"), &m), b"\x1bOA");
    // A modifier turns the arrow into a CSI with a parameter, app-cursor or not.
    assert_eq!(encode(&ShellKey { key: "Up".into(), ctrl: true, ..Default::default() }, &m), b"\x1b[1;5A");
    assert_eq!(encode(&key("F1"), &m), b"\x1bOP");
    assert_eq!(encode(&key("F5"), &m), b"\x1b[15~");
    assert_eq!(encode(&key("Delete"), &m), b"\x1b[3~");
}

#[test]
fn paste_bracketing_and_injection() {
    let mut m = ShellModeMsg::default();
    // Without bracketed paste, newlines become carriage returns.
    assert_eq!(text_bytes("a\nb", true, &m), b"a\rb");
    m.bracketed_paste = true;
    assert_eq!(text_bytes("ab", true, &m), b"\x1b[200~ab\x1b[201~");
    // An end-of-paste marker inside the text can't smuggle out of the bracket.
    assert_eq!(text_bytes("a\x1b[201~b", true, &m), b"\x1b[200~ab\x1b[201~");
    // Non-paste text goes as-is.
    assert_eq!(text_bytes("hi", false, &m), b"hi");
}

#[test]
fn mouse_only_when_tracked() {
    let mut m = ShellModeMsg::default();
    assert_eq!(mouse("down", 0, 3, 4, false, false, false, &m), None);
    m.mouse_click = true;
    m.sgr_mouse = true;
    // SGR report: button 0, 1-based column/row, 'M' for press.
    assert_eq!(mouse("down", 0, 3, 4, false, false, false, &m), Some(b"\x1b[<0;4;5M".to_vec()));
    assert_eq!(mouse("up", 0, 3, 4, false, false, false, &m), Some(b"\x1b[<0;4;5m".to_vec()));
    // A wheel with no tracking but an alt screen + alternate scroll becomes three arrows.
    let mut alt = ShellModeMsg { alt_screen: true, alternate_scroll: true, ..Default::default() };
    assert_eq!(mouse("wheel_up", 0, 0, 0, false, false, false, &alt), None);
    assert_eq!(wheel_arrows("wheel_up", &alt), Some(b"\x1b[A\x1b[A\x1b[A".to_vec()));
    alt.app_cursor = true;
    assert_eq!(wheel_arrows("wheel_down", &alt), Some(b"\x1bOB\x1bOB\x1bOB".to_vec()));
}

/// A screen rendered from bytes fed to a mock terminal, so the row text and the activity
/// rule can be checked without a PTY.
#[cfg(unix)]
mod rendered {
    use crate::shell::screen::{render_rows, ShellScreen};
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::vte::ansi::Processor;

    fn term_with(bytes: &[&[u8]]) -> (Term<VoidListener>, ShellScreen) {
        let size = TermSize::new(20, 5);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();
        let mut screen = ShellScreen::new(20, 5);
        for chunk in bytes {
            parser.advance(&mut term, chunk);
        }
        let snap = render_rows(&term);
        screen.apply(snap);
        (term, screen)
    }

    #[test]
    fn text_and_colour_survive() {
        let (_t, screen) = term_with(&[b"\x1b[31mred\x1b[0m plain"]);
        assert!(screen.lines[0].contains("red"), "row: {:?}", screen.lines[0]);
        assert!(screen.lines[0].contains("\x1b[") && screen.lines[0].contains("31"), "has colour: {:?}", screen.lines[0]);
    }

    #[test]
    fn activity_only_on_new_printable() {
        let size = TermSize::new(20, 5);
        let mut term = Term::new(Config::default(), &size, VoidListener);
        let mut parser: Processor = Processor::new();
        let mut screen = ShellScreen::new(20, 5);
        parser.advance(&mut term, b"hello");
        let first = screen.apply(render_rows(&term));
        assert!(first.gained_printable, "first text is activity");
        // Move the cursor around: no new characters, no activity.
        parser.advance(&mut term, b"\x1b[H\x1b[5C");
        let moved = screen.apply(render_rows(&term));
        assert!(!moved.gained_printable, "cursor movement is not activity");
        // Clear the line: characters removed, not gained.
        parser.advance(&mut term, b"\x1b[2K");
        let cleared = screen.apply(render_rows(&term));
        assert!(!cleared.gained_printable, "clearing is not activity");
        // New text: activity again.
        parser.advance(&mut term, b"\x1b[Hworld");
        let more = screen.apply(render_rows(&term));
        assert!(more.gained_printable, "new text is activity");
    }

    #[test]
    fn delta_then_full_frame_shape() {
        use crate::websocket::WsMessage;
        let (term, mut screen) = term_with(&[b"line one"]);
        let before = screen.frame;
        // full_msg does not advance the frame.
        let _full = screen.full_msg(0);
        assert_eq!(screen.frame, before, "a full frame does not advance");
        // The FIRST delta for a fresh screen is sent as a full frame (base_frame None), so
        // a client already viewing the world when the shell connects can apply it directly.
        match screen.delta_msg(0) {
            WsMessage::ShellScreen { base_frame, .. } => assert!(base_frame.is_none(), "the first frame is full"),
            _ => panic!("expected ShellScreen"),
        }
        assert_eq!(screen.frame, before, "the first (full) frame does not advance either");
        // A real change now produces a delta that advances the frame.
        let mut parser: Processor = Processor::new();
        let mut term = term;
        parser.advance(&mut term, b"\r\nline two");
        screen.apply(render_rows(&term));
        match screen.delta_msg(0) {
            WsMessage::ShellScreen { base_frame, .. } => assert_eq!(base_frame, Some(before), "a later delta builds on the held frame"),
            _ => panic!("expected ShellScreen"),
        }
        assert_eq!(screen.frame, before + 1, "a delta advances the frame");
        assert!(!screen.has_changes(), "a delta clears the changed flag");
    }

    /// After a `/reload` re-attaches a shell, the program's repaint is not activity: text
    /// appearing inside the quiet window is not counted, text after it is.
    #[test]
    fn a_reloads_repaint_is_not_activity() {
        use std::time::{Duration, Instant};
        let (term, mut screen) = term_with(&[b"$ "]);
        let mut parser: Processor = Processor::new();
        let mut term = term;
        screen.ignore_activity_until(Instant::now() + Duration::from_secs(60));
        parser.advance(&mut term, b"\r\n[Clay reloaded]\r\n$ ");
        let repaint = screen.apply(render_rows(&term));
        assert!(repaint.changed, "the screen did change");
        assert!(!repaint.gained_printable, "but a reload's repaint is not news");
        // The window over: real output counts again.
        screen.ignore_activity_until(Instant::now() - Duration::from_secs(1));
        parser.advance(&mut term, b"You have new mail.");
        let later = screen.apply(render_rows(&term));
        assert!(later.gained_printable, "text after the window is activity");
    }

    /// The client side: a delta that doesn't build on the frame held asks for the whole
    /// screen once; the stale deltas in flight until it arrives are dropped without
    /// asking again, and a delta rejected after the full frame asks afresh.
    #[test]
    fn stale_deltas_ask_for_a_full_frame_once() {
        use crate::websocket::{ShellCursorMsg, ShellModeMsg};
        let (cur, mode) = (ShellCursorMsg::default(), ShellModeMsg::default());
        let mut client = ShellScreen::new(80, 24);
        assert!(client.apply_msg(7, None, 80, 24, &[(0, "$ ".into())], cur, mode, None, true), "a full frame always applies");
        assert!(client.apply_msg(8, Some(7), 80, 24, &[(0, "$ x".into())], cur, mode, None, true), "a delta on the held frame applies");
        assert_eq!(client.frame, 8);
        // A resize on the server: the next delta carries other dimensions.
        assert!(!client.apply_msg(9, Some(8), 73, 24, &[], cur, mode, None, true), "out of step: ask for the screen");
        assert!(client.apply_msg(10, Some(9), 73, 24, &[], cur, mode, None, true), "still waiting: dropped, no second request");
        assert!(client.apply_msg(11, Some(10), 73, 24, &[], cur, mode, None, true), "and again");
        assert_eq!(client.frame, 8, "nothing stale was applied");
        assert!(client.apply_msg(11, None, 73, 24, &[(0, "$ x".into())], cur, mode, None, true), "the full frame lands");
        assert_eq!((client.frame, client.cols), (11, 73));
        assert!(client.apply_msg(12, Some(11), 73, 24, &[], cur, mode, None, true), "back in step");
        assert!(!client.apply_msg(14, Some(13), 73, 24, &[], cur, mode, None, true), "out of step after the full frame: asks again");
    }
}

/// A real PTY: start a short-lived shell, see its output and its exit code, with Clay's
/// own child reaper running on the side (the regression that spun the event loop).
#[cfg(all(unix, not(target_os = "android")))] // Clay's reaper, as on the desktop
#[test]
fn real_pty_runs_and_exits() {
    use crate::shell::{ShellSession, Size};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::AppEvent>(100);
        let session = ShellSession::start("test", 1, Some("printf 'hi-there\\n'; exit 3"), Size { cols: 40, rows: 10 }, tx).unwrap();

        // A reaper hammering waitpid(0) the way the event loop does.
        let stop = Arc::new(AtomicBool::new(false));
        let reaper = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    crate::platform::reap_zombie_children();
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };

        let mut exit_code = None;
        let mut saw_text = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(crate::AppEvent::Shell(_, _, ev))) => match ev {
                    crate::shell::ShellEvent::Wakeup => {
                        let snap = session.with_term(crate::shell::screen::render_rows);
                        if snap.lines.iter().any(|l| l.contains("hi-there")) {
                            saw_text = true;
                        }
                    }
                    crate::shell::ShellEvent::Exited(code) => {
                        exit_code = Some(code);
                        break;
                    }
                    _ => {}
                },
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {}
            }
        }
        stop.store(true, Ordering::Relaxed);
        let _ = reaper.join();
        assert!(saw_text, "the shell's output reached the screen");
        assert_eq!(exit_code, Some(Some(3)), "the shell's exit code was reported");
    });
}

/// Phase 7: an AdoptedPty built from an existing PTY master + a running child (as a reload
/// hands it over) runs the emulator, shows what the child wrote, and reports its exit.
#[cfg(target_os = "linux")]
#[test]
fn adopted_pty_runs_and_exits() {
    use crate::shell::{ShellSession, Size};
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    // A real PTY pair.
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let r = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) };
    assert_eq!(r, 0, "openpty");

    // A child on the slave, in its own session (as the real spawn does), that writes a
    // marker and then waits - the "already-running program" a reload re-attaches.
    let slave_owned = unsafe { OwnedFd::from_raw_fd(slave) };
    let (sin, sout, serr) = (slave_owned.try_clone().unwrap(), slave_owned.try_clone().unwrap(), slave_owned);
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg("printf ADOPTED_OK; exec sleep 5");
    cmd.stdin(Stdio::from(sin)).stdout(Stdio::from(sout)).stderr(Stdio::from(serr));
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn().expect("spawn child on the pty slave");
    let pid = child.id();
    // The AdoptedPty owns the child from here (its Drop reaps it); don't let std wait too.
    std::mem::forget(child);

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::AppEvent>(100);
        let session = ShellSession::adopt("reloaded", 1, master, pid, Size { cols: 40, rows: 10 }, tx).expect("adopt");

        // The child's marker reaches the screen.
        let mut saw = false;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !saw {
            if tokio::time::timeout(Duration::from_millis(200), rx.recv()).await.is_ok() {
                let snap = session.with_term(crate::shell::screen::render_rows);
                if snap.lines.iter().any(|l| l.contains("ADOPTED_OK")) {
                    saw = true;
                }
            }
        }
        assert!(saw, "the re-attached child's output reached the screen");

        // Ending the session hangs the child up and reports its exit.
        drop(session);
        let mut exited = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(crate::AppEvent::Shell(_, _, crate::shell::ShellEvent::Exited(_)))) => { exited = true; break; }
                Ok(Some(_)) => {}
                _ => {}
            }
        }
        assert!(exited, "the re-attached session reported Exited after shutdown");
    });
}
